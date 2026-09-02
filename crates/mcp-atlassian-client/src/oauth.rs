use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;

use crate::error::{Error, Result};

pub const DEFAULT_TOKEN_URL: &str = "https://auth.atlassian.com/oauth/token";

/// Refresh tokens are rotated by Atlassian — the session keeps the latest one
/// in memory. Tokens are refreshed this long before their actual expiry.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
    /// Override for tests; defaults to [`DEFAULT_TOKEN_URL`].
    pub token_url: String,
    /// When the refresh token came from a `*_FILE`, a rotated one is written
    /// back there, so a restart does not begin with a revoked token. `None`
    /// keeps rotation in memory only (D17).
    pub persist_refresh_token_to: Option<PathBuf>,
}

/// A shared OAuth 2.0 (3LO) session: caches the access token and refreshes it
/// via the rotating refresh token. One session is shared by the Jira and
/// Confluence clients so they draw from the same token cache.
pub struct OAuthSession {
    config: OAuthConfig,
    state: Mutex<State>,
}

struct State {
    access_token: Option<String>,
    expires_at: Instant,
    refresh_token: String,
}

/// What a token response is assumed to last when it does not say. Zero
/// would mean a refresh on every request.
const DEFAULT_EXPIRES_IN: u64 = 3600;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
    /// Atlassian rotates refresh tokens; absent means keep the current one.
    #[serde(default)]
    refresh_token: Option<String>,
}

fn default_expires_in() -> u64 {
    tracing::debug!("token response carried no expires_in; assuming {DEFAULT_EXPIRES_IN}s");
    DEFAULT_EXPIRES_IN
}

impl std::fmt::Debug for OAuthSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthSession")
            .field("client_id", &self.config.client_id)
            .field("token_url", &self.config.token_url)
            .finish_non_exhaustive()
    }
}

impl OAuthSession {
    pub fn new(config: OAuthConfig) -> Result<Self> {
        let state = State {
            access_token: None,
            expires_at: Instant::now(),
            refresh_token: config.refresh_token.clone(),
        };
        Ok(Self {
            config,
            state: Mutex::new(state),
        })
    }

    /// Returns a valid access token, refreshing if missing or near expiry.
    pub async fn access_token(&self) -> Result<String> {
        let mut state = self.state.lock().await;
        if let Some(token) = &state.access_token {
            if state.expires_at > Instant::now() {
                return Ok(token.clone());
            }
        }

        let resp = crate::http::shared_http()
            .post(&self.config.token_url)
            .timeout(Duration::from_secs(30))
            .json(&json!({
                "grant_type": "refresh_token",
                "client_id": self.config.client_id,
                "client_secret": self.config.client_secret,
                "refresh_token": state.refresh_token,
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::OAuth(format!(
                "token refresh failed (HTTP {status}): {body}; \
                 check ATLASSIAN_OAUTH_* credentials — the refresh token may have been \
                 rotated or revoked, re-run the authorization flow to obtain a new one"
            )));
        }
        let token: TokenResponse = resp
            .json()
            .await
            .map_err(|e| Error::OAuth(format!("invalid token response: {e}")))?;

        if let Some(rotated) = token.refresh_token {
            if rotated != state.refresh_token {
                if let Some(path) = &self.config.persist_refresh_token_to {
                    persist(path, &rotated);
                }
            }
            state.refresh_token = rotated;
        }
        state.expires_at = Instant::now() + Duration::from_secs(token.expires_in)
            - EXPIRY_MARGIN.min(Duration::from_secs(token.expires_in));
        state.access_token = Some(token.access_token.clone());
        tracing::debug!("refreshed Atlassian OAuth access token");
        Ok(token.access_token)
    }
}

/// Writes the rotated refresh token where it was read from: a temporary file
/// beside it, owner-only on Unix, then an atomic rename. A failure is logged
/// and not fatal — the session still holds the token in memory, and the next
/// start will say what went wrong.
fn persist(path: &std::path::Path, token: &str) {
    let tmp = path.with_extension("tmp");
    let written = write_private(&tmp, token).and_then(|()| std::fs::rename(&tmp, path));
    match written {
        Ok(()) => tracing::info!(path = %path.display(), "stored the rotated OAuth refresh token"),
        Err(error) => tracing::error!(
            path = %path.display(),
            %error,
            "could not store the rotated OAuth refresh token; the next start will need a fresh one"
        ),
    }
}

fn write_private(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(contents.as_bytes())?;
    file.write_all(b"\n")
}
