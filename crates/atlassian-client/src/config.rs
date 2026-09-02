use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::oauth::{OAuthConfig, OAuthSession, DEFAULT_TOKEN_URL};
use crate::tool_filter::ToolFilter;

/// How to authenticate against an Atlassian instance.
#[derive(Debug, Clone)]
pub enum Auth {
    /// Atlassian Cloud: HTTP Basic with email + API token.
    Basic { username: String, token: String },
    /// Server / Data Center: Bearer personal access token.
    Pat { token: String },
    /// Atlassian Cloud OAuth 2.0 (3LO) with a rotating refresh token; the
    /// session is shared between services (one token cache).
    OAuth(Arc<OAuthSession>),
}

/// Connection settings for a single service (Jira or Confluence).
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub base_url: String,
    pub auth: Auth,
}

/// Full server configuration, read from environment variables.
///
/// Variable names follow the conventions established by existing Atlassian
/// MCP servers, so an existing client config works unchanged (D8).
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
    /// Path of the JSONL audit log for write operations. `None` disables it.
    pub audit_log: Option<PathBuf>,
    /// How long reference data (projects, issue types, boards, spaces, field
    /// definitions) may be reused. `None` disables caching entirely (D25).
    pub cache_ttl: Option<Duration>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        // OAuth (Cloud-only) takes precedence and configures both services
        // against the api.atlassian.com gateway.
        let (jira, confluence) = if let Some((session, cloud_id)) = oauth_from_env()? {
            (
                Some(ServiceConfig {
                    base_url: format!("https://api.atlassian.com/ex/jira/{cloud_id}"),
                    auth: Auth::OAuth(session.clone()),
                }),
                Some(ServiceConfig {
                    base_url: format!("https://api.atlassian.com/ex/confluence/{cloud_id}/wiki"),
                    auth: Auth::OAuth(session),
                }),
            )
        } else {
            (
                ServiceConfig::from_env("JIRA")?,
                ServiceConfig::from_env("CONFLUENCE")?,
            )
        };
        if jira.is_none() && confluence.is_none() {
            return Err(Error::Config(
                "neither JIRA_URL nor CONFLUENCE_URL nor ATLASSIAN_OAUTH_* is set; \
                 configure at least one service"
                    .into(),
            ));
        }

        let enabled_tools = env::var("ENABLED_TOOLS")
            .ok()
            .and_then(|raw| ToolFilter::parse(&raw));
        let disabled_tools = env::var("DISABLED_TOOLS")
            .ok()
            .and_then(|raw| ToolFilter::parse(&raw));

        let read_only = env_flag("READ_ONLY");
        let dry_run = env_flag("DRY_RUN");

        let audit_log = env::var("AUDIT_LOG_FILE")
            .ok()
            .map(|raw| raw.trim().to_string())
            .filter(|raw| !raw.is_empty())
            .map(PathBuf::from);

        let cache_ttl = parse_cache_ttl(env::var("CACHE_TTL").ok().as_deref())?;

        Ok(Self {
            jira,
            confluence,
            enabled_tools,
            disabled_tools,
            read_only,
            dry_run,
            audit_log,
            cache_ttl,
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
fn secret(name: &str) -> Result<Option<String>> {
    let inline = env::var(name).ok();
    let Some(path) = env::var(format!("{name}_FILE"))
        .ok()
        .filter(|p| !p.trim().is_empty())
    else {
        return Ok(inline);
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
    Ok(Some(value.to_string()))
}

/// Reads a boolean switch. Absent or unrecognized means off: a flag that
/// weakens or changes behaviour should never be enabled by a typo.
fn env_flag(name: &str) -> bool {
    env::var(name).is_ok_and(|value| is_truthy(&value))
}

fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes"
    )
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

/// Reads `ATLASSIAN_OAUTH_{CLIENT_ID,CLIENT_SECRET,REFRESH_TOKEN,CLOUD_ID}`.
/// All four present => OAuth mode; none => `Ok(None)`; partial => error.
/// The user obtains the initial refresh token via a one-time 3LO
/// authorization (offline_access scope); we refresh automatically after that.
fn oauth_from_env() -> Result<Option<(Arc<OAuthSession>, String)>> {
    let vars = [
        "ATLASSIAN_OAUTH_CLIENT_ID",
        "ATLASSIAN_OAUTH_CLIENT_SECRET",
        "ATLASSIAN_OAUTH_REFRESH_TOKEN",
        "ATLASSIAN_OAUTH_CLOUD_ID",
    ];
    // The client secret and the refresh token are credentials, so they also
    // accept the `*_FILE` form; the client and cloud ids are identifiers.
    let values: Vec<Option<String>> = vars
        .iter()
        .map(|name| match *name {
            "ATLASSIAN_OAUTH_CLIENT_SECRET" | "ATLASSIAN_OAUTH_REFRESH_TOKEN" => secret(name),
            _ => Ok(env::var(name).ok()),
        })
        .collect::<Result<_>>()?;
    if values.iter().all(Option::is_none) {
        return Ok(None);
    }
    let [client_id, client_secret, refresh_token, cloud_id] = values
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            v.ok_or_else(|| {
                Error::Config(format!(
                    "OAuth is partially configured: {} is missing",
                    vars[i]
                ))
            })
        })
        .collect::<Result<Vec<_>>>()?
        .try_into()
        .expect("exactly four vars");
    let session = OAuthSession::new(OAuthConfig {
        client_id,
        client_secret,
        refresh_token,
        token_url: env::var("ATLASSIAN_OAUTH_TOKEN_URL")
            .unwrap_or_else(|_| DEFAULT_TOKEN_URL.to_string()),
    })?;
    Ok(Some((Arc::new(session), cloud_id)))
}

impl ServiceConfig {
    /// Reads `{prefix}_URL` plus auth variables. Returns `Ok(None)` when the
    /// service is not configured at all, `Err` when it is half-configured.
    fn from_env(prefix: &str) -> Result<Option<Self>> {
        let Ok(base_url) = env::var(format!("{prefix}_URL")) else {
            return Ok(None);
        };
        let base_url = base_url.trim_end_matches('/').to_string();

        // A personal access token switches the instance into Server/DC mode (D6).
        if let Some(token) = secret(&format!("{prefix}_PERSONAL_TOKEN"))? {
            return Ok(Some(Self {
                base_url,
                auth: Auth::Pat { token },
            }));
        }

        let username = env::var(format!("{prefix}_USERNAME")).ok();
        let token = secret(&format!("{prefix}_API_TOKEN"))?;
        match (username, token) {
            (Some(username), Some(token)) => Ok(Some(Self {
                base_url,
                auth: Auth::Basic { username, token },
            })),
            _ => Err(Error::Config(format!(
                "{prefix}_URL is set but credentials are incomplete: \
                 set {prefix}_USERNAME + {prefix}_API_TOKEN (Cloud) \
                 or {prefix}_PERSONAL_TOKEN (Server/Data Center). \
                 Every token may instead be given as a path, via the matching \
                 *_FILE variable"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_truthy, parse_cache_ttl, secret};
    use std::env;
    use std::time::Duration;

    /// Each case uses its own variable names, so the cases do not race each
    /// other through the process environment.
    struct Var(String);

    impl Var {
        fn new(case: &str) -> Self {
            Self(format!("MCP_TEST_SECRET_{case}"))
        }

        fn inline(&self, value: &str) -> &Self {
            env::set_var(&self.0, value);
            self
        }

        fn file(&self, contents: &str) -> &Self {
            let path = env::temp_dir().join(format!("{}-{}", self.0, std::process::id()));
            std::fs::write(&path, contents).unwrap();
            env::set_var(format!("{}_FILE", self.0), &path);
            self
        }

        fn missing_file(&self) -> &Self {
            env::set_var(format!("{}_FILE", self.0), "/nonexistent-directory/token");
            self
        }

        fn read(&self) -> super::Result<Option<String>> {
            secret(&self.0)
        }
    }

    impl Drop for Var {
        fn drop(&mut self) {
            env::remove_var(&self.0);
            env::remove_var(format!("{}_FILE", self.0));
            let _ = std::fs::remove_file(env::temp_dir().join(format!(
                "{}-{}",
                self.0,
                std::process::id()
            )));
        }
    }

    #[test]
    fn a_secret_is_read_inline_or_from_the_file_the_var_points_at() {
        let unset = Var::new("UNSET");
        assert_eq!(unset.read().unwrap(), None);

        let inline = Var::new("INLINE");
        inline.inline("token-from-env");
        assert_eq!(inline.read().unwrap().as_deref(), Some("token-from-env"));

        let from_file = Var::new("FROM_FILE");
        from_file.file("token-from-file");
        assert_eq!(
            from_file.read().unwrap().as_deref(),
            Some("token-from-file")
        );
    }

    #[test]
    fn the_trailing_newline_of_a_secret_file_is_stripped() {
        // What `echo secret > file` leaves behind; kept, it fails auth with no
        // visible cause.
        let var = Var::new("NEWLINE");
        var.file("token-from-file\n");
        assert_eq!(var.read().unwrap().as_deref(), Some("token-from-file"));
    }

    #[test]
    fn setting_both_spellings_is_an_error_rather_than_a_precedence_rule() {
        let var = Var::new("BOTH");
        var.inline("a").file("b");
        let error = var.read().unwrap_err().to_string();
        assert!(error.contains("MCP_TEST_SECRET_BOTH"), "{error}");
        assert!(error.contains("_FILE"), "{error}");
    }

    #[test]
    fn an_unreadable_or_empty_secret_file_names_the_variable_and_the_path() {
        let missing = Var::new("MISSING");
        missing.missing_file();
        let error = missing.read().unwrap_err().to_string();
        assert!(error.contains("MCP_TEST_SECRET_MISSING_FILE"), "{error}");
        assert!(error.contains("/nonexistent-directory/token"), "{error}");

        let empty = Var::new("EMPTY");
        empty.file("   \n");
        let error = empty.read().unwrap_err().to_string();
        assert!(error.contains("is empty"), "{error}");
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
    fn a_non_numeric_ttl_names_the_variable_and_an_example() {
        let error = parse_cache_ttl(Some("5m")).unwrap_err().to_string();
        assert!(error.contains("CACHE_TTL"), "{error}");
        assert!(error.contains("300"), "{error}");
    }
}
