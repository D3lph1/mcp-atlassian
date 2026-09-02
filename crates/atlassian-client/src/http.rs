use std::time::Duration;

use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use url::Url;

use crate::config::{Auth, ServiceConfig};
use crate::error::{Error, Result};

/// Maximum time we are willing to sleep on a single 429 Retry-After.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Thin HTTP client for one Atlassian instance.
///
/// Adds authentication, JSON handling, error mapping and a single retry on
/// HTTP 429. Product crates (`atlassian-jira`, `atlassian-confluence`) build
/// their endpoints on top of this.
#[derive(Debug, Clone)]
pub struct AtlassianClient {
    base_url: Url,
    auth: Auth,
    http: reqwest::Client,
}

impl AtlassianClient {
    pub fn new(config: &ServiceConfig) -> Result<Self> {
        let mut base_url = Url::parse(&config.base_url).map_err(|e| Error::InvalidUrl {
            url: config.base_url.clone(),
            message: e.to_string(),
        })?;
        // `Url::join` drops the last path segment of a base without a trailing
        // slash: ".../wiki" + "rest/x" would lose "/wiki" (Confluence Cloud
        // lives under that prefix). Normalize once here.
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("mcp-atlassian-rust/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            base_url,
            auth: config.auth.clone(),
            http,
        })
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T> {
        let req = self.request(Method::GET, path, query).await?;
        self.send_json(req).await
    }

    pub async fn post<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let req = self.request(Method::POST, path, &[]).await?.json(body);
        self.send_json(req).await
    }

    pub async fn put<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let req = self.request(Method::PUT, path, &[]).await?.json(body);
        self.send_json(req).await
    }

    /// PUT for endpoints that return `204 No Content` (e.g. Jira issue update).
    pub async fn put_no_content<B: Serialize + ?Sized>(&self, path: &str, body: &B) -> Result<()> {
        let req = self.request(Method::PUT, path, &[]).await?.json(body);
        let resp = self.send(req).await?;
        Self::check_status(resp).await.map(|_| ())
    }

    /// POST for endpoints that return `204 No Content` (e.g. Jira transitions).
    pub async fn post_no_content<B: Serialize + ?Sized>(&self, path: &str, body: &B) -> Result<()> {
        let req = self.request(Method::POST, path, &[]).await?.json(body);
        let resp = self.send(req).await?;
        Self::check_status(resp).await.map(|_| ())
    }

    pub async fn delete(&self, path: &str, query: &[(&str, &str)]) -> Result<()> {
        let req = self.request(Method::DELETE, path, query).await?;
        let resp = self.send(req).await?;
        Self::check_status(resp).await.map(|_| ())
    }

    /// Downloads raw bytes. Accepts a path or an absolute URL, but an absolute
    /// URL must be same-origin with the configured base URL — attachment
    /// `content` links are absolute, and we refuse to send credentials
    /// anywhere else.
    pub async fn get_bytes(&self, path_or_url: &str) -> Result<Vec<u8>> {
        if !path_or_url.starts_with("http://") && !path_or_url.starts_with("https://") {
            check_path(path_or_url)?;
        }
        let url = if path_or_url.starts_with("http://") || path_or_url.starts_with("https://") {
            let url = Url::parse(path_or_url).map_err(|e| Error::InvalidUrl {
                url: path_or_url.to_string(),
                message: e.to_string(),
            })?;
            if url.origin() != self.base_url.origin() {
                return Err(Error::Config(format!(
                    "refusing to send credentials to foreign origin `{}` (base is `{}`)",
                    url.origin().ascii_serialization(),
                    self.base_url.origin().ascii_serialization(),
                )));
            }
            url
        } else {
            self.base_url
                .join(path_or_url.trim_start_matches('/'))
                .map_err(|e| Error::InvalidUrl {
                    url: path_or_url.to_string(),
                    message: e.to_string(),
                })?
        };
        let req = self.authorize(self.http.request(Method::GET, url)).await?;
        let resp = self.send(req).await?;
        let resp = Self::check_status(resp).await?;
        Ok(resp.bytes().await?.to_vec())
    }

    /// Uploads one file as `multipart/form-data` (field name `file`).
    /// Sends `X-Atlassian-Token: no-check` as required by attachment endpoints.
    pub async fn post_multipart<T: DeserializeOwned>(
        &self,
        path: &str,
        file_name: &str,
        bytes: Vec<u8>,
    ) -> Result<T> {
        let part = reqwest::multipart::Part::bytes(bytes).file_name(file_name.to_string());
        let form = reqwest::multipart::Form::new().part("file", part);
        let req = self
            .request(Method::POST, path, &[])
            .await?
            .header("X-Atlassian-Token", "no-check")
            .multipart(form);
        // multipart requests are not cloneable — no 429 retry here.
        let resp = req.send().await?;
        let resp = Self::check_status(resp).await?;
        let bytes = resp.bytes().await?;
        serde_json::from_slice(&bytes).map_err(|e| Error::Decode(e.to_string()))
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<RequestBuilder> {
        check_path(path)?;
        let url = self
            .base_url
            .join(path.trim_start_matches('/'))
            .map_err(|e| Error::InvalidUrl {
                url: path.to_string(),
                message: e.to_string(),
            })?;
        let mut req = self.http.request(method, url);
        if !query.is_empty() {
            req = req.query(query);
        }
        self.authorize(req).await
    }

    async fn authorize(&self, req: RequestBuilder) -> Result<RequestBuilder> {
        Ok(match &self.auth {
            Auth::Basic { username, token } => req.basic_auth(username, Some(token)),
            Auth::Pat { token } => req.bearer_auth(token),
            Auth::OAuth(session) => req.bearer_auth(session.access_token().await?),
        })
    }

    /// Sends the request, retrying once on 429 with respect for `Retry-After`.
    async fn send(&self, req: RequestBuilder) -> Result<Response> {
        let retry = req.try_clone();
        let resp = req.send().await?;
        if resp.status() != StatusCode::TOO_MANY_REQUESTS {
            return Ok(resp);
        }
        let Some(retry) = retry else {
            return Err(Error::RateLimited);
        };
        let delay = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(1))
            .min(MAX_RETRY_AFTER);
        tracing::warn!(?delay, "rate limited by Atlassian API, retrying once");
        tokio::time::sleep(delay).await;
        let resp = retry.send().await?;
        if resp.status() == StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::RateLimited);
        }
        Ok(resp)
    }

    async fn send_json<T: DeserializeOwned>(&self, req: RequestBuilder) -> Result<T> {
        let resp = self.send(req).await?;
        let resp = Self::check_status(resp).await?;
        let bytes = resp.bytes().await?;
        serde_json::from_slice(&bytes).map_err(|e| Error::Decode(e.to_string()))
    }

    /// Maps non-2xx responses to typed errors, extracting Atlassian error
    /// messages from the body where possible.
    async fn check_status(resp: Response) -> Result<Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let url = resp.url().path().to_string();
        let body = resp.text().await.unwrap_or_default();
        Err(match status {
            StatusCode::UNAUTHORIZED => Error::Unauthorized,
            StatusCode::FORBIDDEN => Error::Forbidden,
            StatusCode::NOT_FOUND => Error::NotFound(format!(
                "{url}: {}",
                extract_message(&body).unwrap_or_else(|| "resource does not exist".into())
            )),
            _ => Error::Api {
                status: status.as_u16(),
                message: extract_message(&body).unwrap_or(body),
            },
        })
    }
}

/// Rejects a request path that would resolve somewhere other than the endpoint
/// it names.
///
/// Endpoint paths are built with `format!`, interpolating values that reach us
/// from the model — an issue key, a page id, an attachment id. `Url::join`
/// normalizes `..` and honours `?`, so `PROJ-1/../../../myself` or
/// `PROJ-1?expand=x` in one of those slots redirects the request to a
/// different endpoint, carrying the user's credentials and the original HTTP
/// method. For `DELETE` that is worse than deleting the issue that was asked
/// for.
///
/// The check lives here rather than at the ~40 call sites for the reason all
/// the other invariants in this server do: a new endpoint cannot forget it.
/// Rejecting rather than percent-encoding is deliberate — every value that
/// reaches a path segment is an identifier (`PROJ-123`, `123456`, `att10001`),
/// so anything holding a path or query character is a mistake worth reporting,
/// not input worth repairing. `resources.rs` already applies the same rule to
/// URIs (D24); this closes the same hole on the tool side.
fn check_path(path: &str) -> Result<()> {
    let offending = if path.contains('?') {
        Some("a query string (`?`) — query parameters are passed separately")
    } else if path.contains('#') {
        Some("a fragment (`#`)")
    } else if path.split('/').any(|segment| segment == "..") {
        Some("a `..` segment, which would resolve to a different endpoint")
    } else {
        None
    };
    match offending {
        None => Ok(()),
        Some(reason) => Err(Error::InvalidUrl {
            url: path.to_string(),
            message: format!(
                "request path contains {reason}; check the issue key, page id or                  attachment id that was passed"
            ),
        }),
    }
}

/// Pulls a human-readable message out of an Atlassian error body.
/// Jira uses `{"errorMessages": [...], "errors": {...}}`, Confluence uses
/// `{"message": "..."}`.
fn extract_message(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    if let Some(messages) = value.get("errorMessages").and_then(|v| v.as_array()) {
        let joined: Vec<&str> = messages.iter().filter_map(|m| m.as_str()).collect();
        if !joined.is_empty() {
            return Some(joined.join("; "));
        }
    }
    if let Some(errors) = value.get("errors").and_then(|v| v.as_object()) {
        if !errors.is_empty() {
            return Some(
                errors
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", v.as_str().unwrap_or("invalid")))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
    }
    value
        .get("message")
        .and_then(|v| v.as_str())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::check_path;

    #[test]
    fn an_ordinary_endpoint_path_is_accepted() {
        for path in [
            "/rest/api/2/issue/PROJ-123",
            "rest/api/2/issue/PROJ-123/comment",
            "/rest/api/content/123456/child/page",
            "/rest/agile/1.0/board/7/sprint",
            // Percent-encoded dots are literal characters, not a traversal.
            "/rest/api/2/issue/%2e%2e",
        ] {
            assert!(check_path(path).is_ok(), "{path}");
        }
    }

    #[test]
    fn a_path_that_would_resolve_elsewhere_is_rejected() {
        // What an interpolated issue key can do if nothing checks it: escape
        // the endpoint, or bolt on a query string.
        for path in [
            "/rest/api/2/issue/../../../rest/api/2/myself",
            "/rest/api/2/issue/..",
            "/rest/api/2/issue/PROJ-1?expand=changelog",
            "/rest/api/2/issue/PROJ-1#fragment",
        ] {
            let error = check_path(path).unwrap_err().to_string();
            assert!(error.contains(path), "{path}: {error}");
        }
    }

    #[test]
    fn the_rejection_says_which_value_to_look_at() {
        let error = check_path("/rest/api/2/issue/PROJ-1?x=1")
            .unwrap_err()
            .to_string();
        assert!(error.contains("issue key"), "{error}");
        assert!(error.contains("query string"), "{error}");
    }
}
