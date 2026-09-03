use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::oauth::{OAuthConfig, OAuthSession, DEFAULT_TOKEN_URL};
use crate::tool_filter::ToolFilter;

/// How to authenticate against an Atlassian instance.
#[derive(Clone)]
pub enum Auth {
    /// Atlassian Cloud: HTTP Basic with email + API token.
    Basic { username: String, token: String },
    /// Server / Data Center: Bearer personal access token.
    Pat { token: String },
    /// Atlassian Cloud OAuth 2.0 (3LO) with a rotating refresh token; the
    /// session is shared between services (one token cache).
    OAuth(Arc<OAuthSession>),
}

/// Hand-written so that a `{:?}` of a `Config` — in a log line, a panic
/// message, a test failure — never prints a token (D38).
impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Auth::Basic { username, .. } => f
                .debug_struct("Basic")
                .field("username", username)
                .field("token", &"<redacted>")
                .finish(),
            Auth::Pat { .. } => f.debug_struct("Pat").field("token", &"<redacted>").finish(),
            Auth::OAuth(session) => f.debug_tuple("OAuth").field(session).finish(),
        }
    }
}

/// Which Atlassian this is. Cloud and Server/Data Center differ in endpoints
/// (Jira search, projects), user references (`accountId` vs `name`) and
/// parameters (D16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deployment {
    Cloud,
    Server,
}

/// Connection settings for a single service (Jira or Confluence).
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub base_url: String,
    pub auth: Auth,
    /// `{PREFIX}_DEPLOYMENT`, when the operator said so; `None` infers from
    /// the auth mode (D41).
    pub deployment: Option<Deployment>,
}

impl ServiceConfig {
    /// The explicit override, else the inference D16 has always made: a
    /// personal access token means Server/Data Center, anything else Cloud.
    pub fn deployment(&self) -> Deployment {
        self.deployment.unwrap_or(match self.auth {
            Auth::Pat { .. } => Deployment::Server,
            Auth::Basic { .. } | Auth::OAuth(_) => Deployment::Cloud,
        })
    }
}

/// How the MCP server talks to its client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    /// The default: stdout is the protocol, stderr the logs (D7).
    Stdio,
    /// Streamable HTTP at `http://{host}:{port}/mcp` (D18); needs the `http`
    /// cargo feature.
    StreamableHttp {
        host: String,
        port: u16,
        /// Extra `Host` header values accepted besides loopback and the bind
        /// address (DNS-rebinding protection).
        allowed_hosts: Vec<String>,
        /// Bearer token every request must carry; `None` means no
        /// authentication at the HTTP layer (D39).
        bearer_token: Option<String>,
    },
}

impl Transport {
    /// The value `TRANSPORT` spells it as.
    pub fn name(&self) -> &'static str {
        match self {
            Transport::Stdio => "stdio",
            Transport::StreamableHttp { .. } => "streamable-http",
        }
    }
}

/// Default request timeout against Atlassian.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Default cap on one attachment, either direction (D37).
pub const DEFAULT_MAX_ATTACHMENT_BYTES: u64 = 50 * 1024 * 1024;

/// Full server configuration, read from environment variables.
///
/// Variable names follow the conventions established by existing Atlassian
/// MCP servers, so an existing client config works unchanged (D8). Every
/// variable the server reads is read here, so one place validates and one
/// place documents — `AGENTS.md`'s table is this struct.
#[derive(Debug, Clone)]
pub struct Config {
    pub jira: Option<ServiceConfig>,
    pub confluence: Option<ServiceConfig>,
    /// Allowlist of tool-name patterns. `None` means all tools are enabled.
    pub enabled_tools: Option<ToolFilter>,
    /// Denylist of tool-name patterns, subtracted from whatever the allowlist
    /// let through. `None` means nothing is subtracted (D27).
    pub disabled_tools: Option<ToolFilter>,
    /// When true, write tools are not registered at all.
    pub read_only: bool,
    /// When true, write tools stay registered but are described instead of
    /// performed (D26). Orthogonal to `read_only`, which removes them.
    pub dry_run: bool,
    /// When true, destructive tools ask the user through MCP elicitation
    /// before running (D42).
    pub confirm_destructive: bool,
    /// Path of the JSONL audit log for write operations. `None` disables it.
    pub audit_log: Option<PathBuf>,
    /// How long reference data (projects, issue types, boards, spaces, field
    /// definitions) may be reused. `None` disables caching entirely (D25).
    pub cache_ttl: Option<Duration>,
    pub transport: Transport,
    /// Print the framed banner (true) or the structured startup line (D29).
    pub banner: bool,
    /// The directory attachment tools may read from and write to. `None`
    /// means the whole filesystem, which is logged as a warning (D37).
    pub attachment_dir: Option<PathBuf>,
    /// Largest attachment accepted, either direction (D37).
    pub max_attachment_bytes: u64,
    /// Per-request timeout against Atlassian.
    pub request_timeout: Duration,
    /// `LOG_FILTER`: `tracing` target directives (`info`,
    /// `mcp_atlassian_client=debug`). Not `RUST_LOG`, and not `LOG_LEVEL` —
    /// see D8.
    pub log_filter: String,
}

impl Default for Config {
    /// No service, no filtering, stdio — what a test starts from before
    /// setting the two or three fields it is about.
    fn default() -> Self {
        Self {
            jira: None,
            confluence: None,
            enabled_tools: None,
            disabled_tools: None,
            read_only: false,
            dry_run: false,
            confirm_destructive: false,
            audit_log: None,
            cache_ttl: None,
            transport: Transport::Stdio,
            banner: true,
            attachment_dir: None,
            max_attachment_bytes: DEFAULT_MAX_ATTACHMENT_BYTES,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            log_filter: "info".into(),
        }
    }
}

/// Where variables come from: the process environment in `main`, a map in
/// tests. Reads are by name; `None` is "unset".
pub trait Env {
    fn get(&self, name: &str) -> Option<String>;
}

impl<F: Fn(&str) -> Option<String>> Env for F {
    fn get(&self, name: &str) -> Option<String> {
        self(name)
    }
}

impl Env for std::collections::HashMap<&str, &str> {
    fn get(&self, name: &str) -> Option<String> {
        self.get(name).map(|v| v.to_string())
    }
}

impl Config {
    /// Reads the process environment.
    pub fn from_env() -> Result<Self> {
        Self::read(&|name: &str| env::var(name).ok())
    }

    /// Reads from any [`Env`]; every variable the server honours is listed
    /// here, once.
    pub fn read(env: &dyn Env) -> Result<Self> {
        // OAuth (Cloud-only) takes precedence and configures both services
        // against the api.atlassian.com gateway.
        let (jira, confluence) = if let Some((session, cloud_id)) = oauth_from(env)? {
            (
                Some(ServiceConfig {
                    base_url: format!("https://api.atlassian.com/ex/jira/{cloud_id}"),
                    auth: Auth::OAuth(session.clone()),
                    deployment: Some(Deployment::Cloud),
                }),
                Some(ServiceConfig {
                    base_url: format!("https://api.atlassian.com/ex/confluence/{cloud_id}/wiki"),
                    auth: Auth::OAuth(session),
                    deployment: Some(Deployment::Cloud),
                }),
            )
        } else {
            (
                ServiceConfig::read(env, "JIRA")?,
                ServiceConfig::read(env, "CONFLUENCE")?,
            )
        };
        if jira.is_none() && confluence.is_none() {
            return Err(Error::NoService);
        }

        Ok(Self {
            jira,
            confluence,
            enabled_tools: env
                .get("ENABLED_TOOLS")
                .and_then(|raw| ToolFilter::parse(&raw)),
            disabled_tools: env
                .get("DISABLED_TOOLS")
                .and_then(|raw| ToolFilter::parse(&raw)),
            read_only: flag(env, "READ_ONLY"),
            dry_run: flag(env, "DRY_RUN"),
            confirm_destructive: flag(env, "CONFIRM_DESTRUCTIVE"),
            audit_log: path(env, "AUDIT_LOG_FILE"),
            cache_ttl: parse_cache_ttl(env.get("CACHE_TTL").as_deref())?,
            transport: transport(env)?,
            banner: !flag(env, "NO_BANNER"),
            attachment_dir: path(env, "ATTACHMENT_DIR"),
            max_attachment_bytes: parse_bytes(
                "MAX_ATTACHMENT_BYTES",
                env.get("MAX_ATTACHMENT_BYTES").as_deref(),
                DEFAULT_MAX_ATTACHMENT_BYTES,
            )?,
            request_timeout: parse_seconds(
                "REQUEST_TIMEOUT",
                env.get("REQUEST_TIMEOUT").as_deref(),
                DEFAULT_REQUEST_TIMEOUT,
            )?,
            log_filter: env
                .get("LOG_FILTER")
                .map(|raw| raw.trim().to_string())
                .filter(|raw| !raw.is_empty())
                .unwrap_or_else(|| "info".into()),
        })
    }
}

/// Reads a credential from `{name}`, or from the file `{name}_FILE` points at.
///
/// The `*_FILE` convention is what Docker and Kubernetes secrets expect (D28):
/// the secret is mounted as a file, so it never appears in the MCP client's
/// config JSON, in `docker inspect`, or in the process environment.
///
/// Both spellings set at once is an error rather than a precedence rule — with
/// credentials, guessing which one the operator meant is the wrong instinct.
/// Returns the value and, when it came from a file, that file's path.
fn secret(env: &dyn Env, name: &str) -> Result<Option<(String, Option<PathBuf>)>> {
    let inline = env.get(name);
    let Some(path) = env
        .get(&format!("{name}_FILE"))
        .filter(|p| !p.trim().is_empty())
    else {
        return Ok(inline.map(|value| (value, None)));
    };
    if inline.is_some() {
        return Err(Error::Config(format!(
            "{name} and {name}_FILE are both set; use one"
        )));
    }
    let path = path.trim();
    let contents = std::fs::read_to_string(path)
        .map_err(|e| Error::Config(format!("failed to read {name}_FILE `{path}`: {e}")))?;
    // Trailing newlines are what `echo secret > file` leaves behind, and a
    // token with one attached fails authentication for no visible reason.
    let value = contents.trim();
    if value.is_empty() {
        return Err(Error::Config(format!(
            "{name}_FILE `{path}` is empty; it must contain the token"
        )));
    }
    Ok(Some((value.to_string(), Some(PathBuf::from(path)))))
}

/// Reads a boolean switch. Absent or unrecognized means off: a flag that
/// weakens or changes behaviour should never be enabled by a typo.
fn flag(env: &dyn Env, name: &str) -> bool {
    env.get(name).is_some_and(|value| is_truthy(&value))
}

fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes"
    )
}

/// A path-valued variable; blank counts as unset.
fn path(env: &dyn Env, name: &str) -> Option<PathBuf> {
    env.get(name)
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
        .map(PathBuf::from)
}

/// Parses `CACHE_TTL`, a number of seconds. Absent, empty or `0` all mean
/// "no caching" — the default, because a cache changes what a read returns
/// (D25).
fn parse_cache_ttl(raw: Option<&str>) -> Result<Option<Duration>> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(None);
    };
    let seconds: u64 = raw.parse().map_err(|_| {
        Error::Config(format!(
            "CACHE_TTL must be a whole number of seconds (e.g. 300), got `{raw}`"
        ))
    })?;
    Ok((seconds > 0).then(|| Duration::from_secs(seconds)))
}

/// A positive number of seconds, with a default for absent or blank.
fn parse_seconds(name: &str, raw: Option<&str>, default: Duration) -> Result<Duration> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(default);
    };
    match raw.parse::<u64>() {
        Ok(seconds) if seconds > 0 => Ok(Duration::from_secs(seconds)),
        _ => Err(Error::Config(format!(
            "{name} must be a whole number of seconds above zero (e.g. {}), got `{raw}`",
            default.as_secs()
        ))),
    }
}

/// A byte count, with a default for absent or blank. `0` means no limit.
fn parse_bytes(name: &str, raw: Option<&str>, default: u64) -> Result<u64> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(default);
    };
    raw.parse::<u64>().map_err(|_| {
        Error::Config(format!(
            "{name} must be a whole number of bytes (e.g. {default}), got `{raw}`"
        ))
    })
}

/// `{PREFIX}_DEPLOYMENT`: `cloud`, or `server` (also `datacenter`, `dc`).
fn deployment(env: &dyn Env, prefix: &str) -> Result<Option<Deployment>> {
    let name = format!("{prefix}_DEPLOYMENT");
    let Some(raw) = env.get(&name).map(|r| r.trim().to_ascii_lowercase()) else {
        return Ok(None);
    };
    match raw.as_str() {
        "" => Ok(None),
        "cloud" => Ok(Some(Deployment::Cloud)),
        "server" | "datacenter" | "data-center" | "dc" => Ok(Some(Deployment::Server)),
        other => Err(Error::Config(format!(
            "{name} must be `cloud` or `server`, got `{other}`"
        ))),
    }
}

/// `TRANSPORT`, `HOST`, `PORT`, `ALLOWED_HOSTS`, `MCP_BEARER_TOKEN`.
fn transport(env: &dyn Env) -> Result<Transport> {
    let name = env
        .get("TRANSPORT")
        .map(|raw| raw.trim().to_ascii_lowercase())
        .filter(|raw| !raw.is_empty())
        .unwrap_or_else(|| "stdio".into());
    match name.as_str() {
        "stdio" => Ok(Transport::Stdio),
        "streamable-http" | "http" => {
            let port = match env.get("PORT").map(|p| p.trim().to_string()) {
                Some(raw) if !raw.is_empty() => raw
                    .parse()
                    .map_err(|_| Error::Config(format!("PORT must be a number, got `{raw}`")))?,
                _ => 8000,
            };
            Ok(Transport::StreamableHttp {
                host: env
                    .get("HOST")
                    .map(|h| h.trim().to_string())
                    .filter(|h| !h.is_empty())
                    .unwrap_or_else(|| "127.0.0.1".into()),
                port,
                allowed_hosts: env
                    .get("ALLOWED_HOSTS")
                    .unwrap_or_default()
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect(),
                bearer_token: secret(env, "MCP_BEARER_TOKEN")?.map(|(token, _)| token),
            })
        }
        other => Err(Error::Config(format!(
            "unknown TRANSPORT `{other}`: use `stdio` or `streamable-http`"
        ))),
    }
}

/// Reads `ATLASSIAN_OAUTH_{CLIENT_ID,CLIENT_SECRET,REFRESH_TOKEN,CLOUD_ID}`.
/// All four present => OAuth mode; none => `Ok(None)`; partial => error.
/// The user obtains the initial refresh token via a one-time 3LO
/// authorization (offline_access scope); we refresh automatically after that.
fn oauth_from(env: &dyn Env) -> Result<Option<(Arc<OAuthSession>, String)>> {
    let client_id = env.get("ATLASSIAN_OAUTH_CLIENT_ID");
    let cloud_id = env.get("ATLASSIAN_OAUTH_CLOUD_ID");
    // The client secret and the refresh token are credentials, so they also
    // accept the `*_FILE` form; the client and cloud ids are identifiers.
    let client_secret = secret(env, "ATLASSIAN_OAUTH_CLIENT_SECRET")?;
    let refresh_token = secret(env, "ATLASSIAN_OAUTH_REFRESH_TOKEN")?;
    if client_id.is_none()
        && cloud_id.is_none()
        && client_secret.is_none()
        && refresh_token.is_none()
    {
        return Ok(None);
    }
    let missing =
        |name: &str| Error::Config(format!("OAuth is partially configured: {name} is missing"));
    let client_id = client_id.ok_or_else(|| missing("ATLASSIAN_OAUTH_CLIENT_ID"))?;
    let (client_secret, _) =
        client_secret.ok_or_else(|| missing("ATLASSIAN_OAUTH_CLIENT_SECRET"))?;
    let (refresh_token, refresh_token_file) =
        refresh_token.ok_or_else(|| missing("ATLASSIAN_OAUTH_REFRESH_TOKEN"))?;
    let cloud_id = cloud_id.ok_or_else(|| missing("ATLASSIAN_OAUTH_CLOUD_ID"))?;
    let session = OAuthSession::new(OAuthConfig {
        client_id,
        client_secret,
        refresh_token,
        token_url: env
            .get("ATLASSIAN_OAUTH_TOKEN_URL")
            .unwrap_or_else(|| DEFAULT_TOKEN_URL.to_string()),
        // A rotated refresh token is written back to the file it came from,
        // so the next start does not begin with a revoked one (D17).
        persist_refresh_token_to: refresh_token_file,
    })?;
    Ok(Some((Arc::new(session), cloud_id)))
}

impl ServiceConfig {
    /// Reads `{prefix}_URL` plus auth variables. Returns `Ok(None)` when the
    /// service is not configured at all, `Err` when it is half-configured.
    fn read(env: &dyn Env, prefix: &str) -> Result<Option<Self>> {
        let Some(base_url) = env.get(&format!("{prefix}_URL")) else {
            return Ok(None);
        };
        let base_url = base_url.trim_end_matches('/').to_string();
        let deployment = deployment(env, prefix)?;

        // A personal access token switches the instance into Server/DC mode (D6).
        if let Some((token, _)) = secret(env, &format!("{prefix}_PERSONAL_TOKEN"))? {
            return Ok(Some(Self {
                base_url,
                auth: Auth::Pat { token },
                deployment,
            }));
        }

        let username = env.get(&format!("{prefix}_USERNAME"));
        let token = secret(env, &format!("{prefix}_API_TOKEN"))?;
        match (username, token) {
            (Some(username), Some((token, _))) => Ok(Some(Self {
                base_url,
                auth: Auth::Basic { username, token },
                deployment,
            })),
            _ => Err(Error::IncompleteCredentials {
                prefix: prefix.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn read(vars: &[(&str, &str)]) -> Result<Config> {
        let env: HashMap<&str, &str> = vars.iter().copied().collect();
        Config::read(&env)
    }

    /// A file holding a secret, removed on drop.
    struct SecretFile(PathBuf);

    impl SecretFile {
        fn new(case: &str, contents: &str) -> Self {
            let path = env::temp_dir().join(format!("mcp-secret-{case}-{}", std::process::id()));
            std::fs::write(&path, contents).unwrap();
            Self(path)
        }

        fn path(&self) -> &str {
            self.0.to_str().unwrap()
        }
    }

    impl Drop for SecretFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn a_cloud_service_reads_url_username_and_token() {
        let config = read(&[
            ("JIRA_URL", "https://x.atlassian.net/"),
            ("JIRA_USERNAME", "u@example.com"),
            ("JIRA_API_TOKEN", "tok"),
        ])
        .unwrap();
        let jira = config.jira.unwrap();
        assert_eq!(jira.base_url, "https://x.atlassian.net");
        assert!(
            matches!(jira.auth, Auth::Basic { ref username, ref token } if username == "u@example.com" && token == "tok")
        );
        assert!(config.confluence.is_none());
        assert_eq!(config.transport, Transport::Stdio);
        assert!(config.banner);
        assert_eq!(config.request_timeout, DEFAULT_REQUEST_TIMEOUT);
        assert_eq!(config.max_attachment_bytes, DEFAULT_MAX_ATTACHMENT_BYTES);
    }

    #[test]
    fn a_personal_token_selects_server_mode_and_wins_over_basic() {
        let config = read(&[
            ("CONFLUENCE_URL", "https://wiki.example.com"),
            ("CONFLUENCE_PERSONAL_TOKEN", "pat"),
            ("CONFLUENCE_USERNAME", "ignored"),
        ])
        .unwrap();
        assert!(matches!(config.confluence.unwrap().auth, Auth::Pat { .. }));
    }

    #[test]
    fn a_half_configured_service_and_no_service_are_errors_that_name_the_variables() {
        let error = read(&[("JIRA_URL", "https://x")]).unwrap_err().to_string();
        assert!(error.contains("JIRA_USERNAME"), "{error}");
        assert!(error.contains("JIRA_PERSONAL_TOKEN"), "{error}");
        let error = read(&[]).unwrap_err().to_string();
        assert!(error.contains("JIRA_URL"), "{error}");
        assert!(error.contains("ATLASSIAN_OAUTH_"), "{error}");
    }

    #[test]
    fn a_secret_is_read_inline_or_from_the_file_the_var_points_at() {
        let file = SecretFile::new("from-file", "token-from-file\n");
        let config = read(&[
            ("JIRA_URL", "https://x"),
            ("JIRA_USERNAME", "u"),
            ("JIRA_API_TOKEN_FILE", file.path()),
        ])
        .unwrap();
        // The trailing newline `echo secret > file` leaves is stripped.
        assert!(
            matches!(config.jira.unwrap().auth, Auth::Basic { ref token, .. } if token == "token-from-file")
        );
    }

    #[test]
    fn setting_both_spellings_is_an_error_rather_than_a_precedence_rule() {
        let file = SecretFile::new("both", "b");
        let error = read(&[
            ("JIRA_URL", "https://x"),
            ("JIRA_USERNAME", "u"),
            ("JIRA_API_TOKEN", "a"),
            ("JIRA_API_TOKEN_FILE", file.path()),
        ])
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("JIRA_API_TOKEN and JIRA_API_TOKEN_FILE"),
            "{error}"
        );
    }

    #[test]
    fn an_unreadable_or_empty_secret_file_names_the_variable_and_the_path() {
        let error = read(&[
            ("JIRA_URL", "https://x"),
            ("JIRA_PERSONAL_TOKEN_FILE", "/nonexistent-directory/token"),
        ])
        .unwrap_err()
        .to_string();
        assert!(error.contains("JIRA_PERSONAL_TOKEN_FILE"), "{error}");
        assert!(error.contains("/nonexistent-directory/token"), "{error}");

        let empty = SecretFile::new("empty", "   \n");
        let error = read(&[
            ("JIRA_URL", "https://x"),
            ("JIRA_PERSONAL_TOKEN_FILE", empty.path()),
        ])
        .unwrap_err()
        .to_string();
        assert!(error.contains("is empty"), "{error}");
    }

    #[test]
    fn oauth_configures_both_services_and_must_be_complete() {
        let config = read(&[
            ("ATLASSIAN_OAUTH_CLIENT_ID", "id"),
            ("ATLASSIAN_OAUTH_CLIENT_SECRET", "secret"),
            ("ATLASSIAN_OAUTH_REFRESH_TOKEN", "refresh"),
            ("ATLASSIAN_OAUTH_CLOUD_ID", "cloud-1"),
            // Ignored: OAuth takes precedence over per-service URLs (D17).
            ("JIRA_URL", "https://ignored"),
        ])
        .unwrap();
        assert_eq!(
            config.jira.as_ref().unwrap().base_url,
            "https://api.atlassian.com/ex/jira/cloud-1"
        );
        assert_eq!(
            config.confluence.as_ref().unwrap().base_url,
            "https://api.atlassian.com/ex/confluence/cloud-1/wiki"
        );
        assert!(matches!(config.jira.unwrap().auth, Auth::OAuth(_)));

        let error = read(&[
            ("ATLASSIAN_OAUTH_CLIENT_ID", "id"),
            ("ATLASSIAN_OAUTH_CLOUD_ID", "cloud-1"),
        ])
        .unwrap_err()
        .to_string();
        assert!(error.contains("ATLASSIAN_OAUTH_CLIENT_SECRET"), "{error}");
    }

    #[test]
    fn switches_filters_and_paths_are_read_and_blank_means_unset() {
        let config = read(&[
            ("JIRA_URL", "https://x"),
            ("JIRA_PERSONAL_TOKEN", "pat"),
            ("READ_ONLY", "yes"),
            ("DRY_RUN", "0"),
            ("CONFIRM_DESTRUCTIVE", "1"),
            ("NO_BANNER", "true"),
            ("ENABLED_TOOLS", "jira_*"),
            ("DISABLED_TOOLS", " , "),
            ("AUDIT_LOG_FILE", "  "),
            ("ATTACHMENT_DIR", "/tmp/att"),
            ("CACHE_TTL", "300"),
            ("REQUEST_TIMEOUT", "5"),
            ("MAX_ATTACHMENT_BYTES", "1024"),
        ])
        .unwrap();
        assert!(config.read_only);
        assert!(!config.dry_run);
        assert!(config.confirm_destructive);
        assert!(!config.banner);
        assert!(config.enabled_tools.unwrap().matches("jira_search"));
        assert!(config.disabled_tools.is_none());
        assert!(config.audit_log.is_none());
        assert_eq!(config.attachment_dir, Some(PathBuf::from("/tmp/att")));
        assert_eq!(config.cache_ttl, Some(Duration::from_secs(300)));
        assert_eq!(config.request_timeout, Duration::from_secs(5));
        assert_eq!(config.max_attachment_bytes, 1024);
    }

    #[test]
    fn a_flag_is_off_unless_it_is_explicitly_on() {
        for on in ["true", "1", "yes", " TRUE ", "Yes"] {
            assert!(is_truthy(on), "{on}");
        }
        for off in ["", "false", "0", "no", "ture", "on"] {
            assert!(!is_truthy(off), "{off}");
        }
    }

    #[test]
    fn caching_is_off_unless_a_positive_ttl_is_given() {
        for raw in [None, Some(""), Some("  "), Some("0")] {
            assert_eq!(parse_cache_ttl(raw).unwrap(), None, "{raw:?}");
        }
        assert_eq!(
            parse_cache_ttl(Some(" 300 ")).unwrap(),
            Some(Duration::from_secs(300))
        );
    }

    #[test]
    fn numeric_settings_name_the_variable_and_an_example_when_malformed() {
        let error = parse_cache_ttl(Some("5m")).unwrap_err().to_string();
        assert!(error.contains("CACHE_TTL"), "{error}");
        assert!(error.contains("300"), "{error}");
        let error = parse_seconds("REQUEST_TIMEOUT", Some("0"), DEFAULT_REQUEST_TIMEOUT)
            .unwrap_err()
            .to_string();
        assert!(error.contains("REQUEST_TIMEOUT"), "{error}");
        assert!(error.contains("30"), "{error}");
        let error = parse_bytes("MAX_ATTACHMENT_BYTES", Some("50MB"), 1)
            .unwrap_err()
            .to_string();
        assert!(error.contains("MAX_ATTACHMENT_BYTES"), "{error}");
    }

    #[test]
    fn the_http_transport_reads_its_bind_address_hosts_and_token() {
        let base = [("JIRA_URL", "https://x"), ("JIRA_PERSONAL_TOKEN", "pat")];
        let config = read(&[
            base[0],
            base[1],
            ("TRANSPORT", "http"),
            ("HOST", "0.0.0.0"),
            ("PORT", "9000"),
            ("ALLOWED_HOSTS", "mcp.example.com, ,mcp2.example.com"),
            ("MCP_BEARER_TOKEN", "s3cret"),
        ])
        .unwrap();
        assert_eq!(
            config.transport,
            Transport::StreamableHttp {
                host: "0.0.0.0".into(),
                port: 9000,
                allowed_hosts: vec!["mcp.example.com".into(), "mcp2.example.com".into()],
                bearer_token: Some("s3cret".into()),
            }
        );
        assert_eq!(config.transport.name(), "streamable-http");

        let config = read(&[base[0], base[1], ("TRANSPORT", "streamable-http")]).unwrap();
        assert!(matches!(
            config.transport,
            Transport::StreamableHttp { ref host, port: 8000, ref allowed_hosts, bearer_token: None }
                if host == "127.0.0.1" && allowed_hosts.is_empty()
        ));

        let error = read(&[base[0], base[1], ("TRANSPORT", "sse")])
            .unwrap_err()
            .to_string();
        assert!(error.contains("sse"), "{error}");
        let error = read(&[base[0], base[1], ("TRANSPORT", "http"), ("PORT", "eighty")])
            .unwrap_err()
            .to_string();
        assert!(error.contains("PORT"), "{error}");
    }

    #[test]
    fn deployment_is_inferred_from_auth_unless_overridden() {
        let basic = read(&[
            ("JIRA_URL", "https://x"),
            ("JIRA_USERNAME", "u"),
            ("JIRA_API_TOKEN", "t"),
        ])
        .unwrap();
        assert_eq!(basic.jira.unwrap().deployment(), Deployment::Cloud);
        let pat = read(&[("JIRA_URL", "https://x"), ("JIRA_PERSONAL_TOKEN", "p")]).unwrap();
        assert_eq!(pat.jira.unwrap().deployment(), Deployment::Server);

        // Data Center behind Basic auth: PATs disabled by policy, or Jira < 8.14.
        let overridden = read(&[
            ("JIRA_URL", "https://x"),
            ("JIRA_USERNAME", "u"),
            ("JIRA_API_TOKEN", "t"),
            ("JIRA_DEPLOYMENT", "dc"),
            ("CONFLUENCE_URL", "https://y"),
            ("CONFLUENCE_PERSONAL_TOKEN", "p"),
            ("CONFLUENCE_DEPLOYMENT", "Cloud"),
        ])
        .unwrap();
        assert_eq!(overridden.jira.unwrap().deployment(), Deployment::Server);
        assert_eq!(
            overridden.confluence.unwrap().deployment(),
            Deployment::Cloud
        );

        let error = read(&[
            ("JIRA_URL", "https://x"),
            ("JIRA_PERSONAL_TOKEN", "p"),
            ("JIRA_DEPLOYMENT", "onprem"),
        ])
        .unwrap_err()
        .to_string();
        assert!(error.contains("JIRA_DEPLOYMENT"), "{error}");
        assert!(error.contains("onprem"), "{error}");
    }

    #[test]
    fn the_log_filter_defaults_to_info() {
        let base = [("JIRA_URL", "https://x"), ("JIRA_PERSONAL_TOKEN", "p")];
        assert_eq!(read(&base).unwrap().log_filter, "info");
        assert_eq!(
            read(&[base[0], base[1], ("LOG_FILTER", " debug,hyper=warn ")])
                .unwrap()
                .log_filter,
            "debug,hyper=warn"
        );
    }

    /// `RUST_LOG` is not a fallback and must not become one: the filter has
    /// exactly one name (D8).
    #[test]
    fn rust_log_is_not_read() {
        let base = [("JIRA_URL", "https://x"), ("JIRA_PERSONAL_TOKEN", "p")];
        assert_eq!(
            read(&[base[0], base[1], ("RUST_LOG", "trace")])
                .unwrap()
                .log_filter,
            "info",
            "RUST_LOG must not affect the filter"
        );
    }

    #[test]
    fn debug_output_never_contains_a_token() {
        let config = read(&[
            ("JIRA_URL", "https://x"),
            ("JIRA_USERNAME", "u@example.com"),
            ("JIRA_API_TOKEN", "hunter2-api"),
            ("CONFLUENCE_URL", "https://y"),
            ("CONFLUENCE_PERSONAL_TOKEN", "hunter2-pat"),
        ])
        .unwrap();
        let printed = format!("{config:?}");
        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(printed.contains("u@example.com"), "{printed}");
        assert!(printed.contains("<redacted>"), "{printed}");
    }
}
