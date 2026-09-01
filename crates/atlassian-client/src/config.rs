use std::collections::HashSet;
use std::env;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::oauth::{OAuthConfig, OAuthSession, DEFAULT_TOKEN_URL};

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
    /// Allowlist of tool names. `None` means all tools are enabled.
    pub enabled_tools: Option<HashSet<String>>,
    /// When true, write tools are not registered at all.
    pub read_only: bool,
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

        let enabled_tools = env::var("ENABLED_TOOLS").ok().and_then(|raw| {
            let set: HashSet<String> = raw
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            (!set.is_empty()).then_some(set)
        });

        let read_only = env::var("READ_ONLY_MODE")
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
            .unwrap_or(false);

        Ok(Self {
            jira,
            confluence,
            enabled_tools,
            read_only,
        })
    }
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
    let values: Vec<Option<String>> = vars.iter().map(|v| env::var(v).ok()).collect();
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
        if let Ok(token) = env::var(format!("{prefix}_PERSONAL_TOKEN")) {
            return Ok(Some(Self {
                base_url,
                auth: Auth::Pat { token },
            }));
        }

        let username = env::var(format!("{prefix}_USERNAME"));
        let token = env::var(format!("{prefix}_API_TOKEN"));
        match (username, token) {
            (Ok(username), Ok(token)) => Ok(Some(Self {
                base_url,
                auth: Auth::Basic { username, token },
            })),
            _ => Err(Error::Config(format!(
                "{prefix}_URL is set but credentials are incomplete: \
                 set {prefix}_USERNAME + {prefix}_API_TOKEN (Cloud) \
                 or {prefix}_PERSONAL_TOKEN (Server/Data Center)"
            ))),
        }
    }
}
