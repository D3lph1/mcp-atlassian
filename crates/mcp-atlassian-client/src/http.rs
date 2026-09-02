use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::config::{Auth, ServiceConfig, DEFAULT_REQUEST_TIMEOUT};
use crate::error::{Error, Result};

/// Maximum time we are willing to sleep on a single 429 Retry-After.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);
/// Retries after the first attempt, for what is safe to retry (D40).
const MAX_RETRIES: u32 = 2;
/// A download may take longer than an API call; its budget is this many
/// request timeouts.
const DOWNLOAD_TIMEOUT_FACTOR: u32 = 10;

/// One connection pool and one TLS configuration for the whole process.
///
/// Every `reqwest::Client` parses the compiled-in root store and keeps its
/// own pool; with a client per service plus one for OAuth that was three of
/// each, for a server whose whole footprint is a few megabytes. Timeouts are
/// per request, so nothing here depends on configuration.
pub(crate) fn shared_http() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("mcp-atlassian-rust/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("a reqwest client with the built-in TLS roots")
    })
}

/// Thin HTTP client for one Atlassian instance.
///
/// Adds authentication, JSON handling, error mapping and bounded retries.
/// Product crates (`atlassian-jira`, `atlassian-confluence`) build their
/// endpoints on top of this.
#[derive(Debug, Clone)]
pub struct AtlassianClient {
    base_url: Url,
    auth: Auth,
    timeout: Duration,
}

/// A file on its way to Atlassian: streamed, not buffered (D37).
pub struct Upload {
    pub file_name: String,
    body: reqwest::Body,
    len: u64,
}

impl Upload {
    /// An in-memory upload — tests, and callers that already hold the bytes.
    pub fn bytes(file_name: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            file_name: file_name.into(),
            len: bytes.len() as u64,
            body: reqwest::Body::from(bytes),
        }
    }

    /// Streams `path`; the attachment is named after the file. The size is
    /// read up front so the request can carry a `Content-Length`.
    pub async fn file(path: &Path) -> Result<Self> {
        let file_error = |e: std::io::Error| Error::File(format!("{}: {e}", path.display()));
        let len = tokio::fs::metadata(path).await.map_err(file_error)?.len();
        let file = tokio::fs::File::open(path).await.map_err(file_error)?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("attachment")
            .to_string();
        Ok(Self {
            file_name,
            len,
            body: reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(file)),
        })
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
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
        Ok(Self {
            base_url,
            auth: config.auth.clone(),
            timeout: DEFAULT_REQUEST_TIMEOUT,
        })
    }

    /// Per-request timeout (`REQUEST_TIMEOUT`); downloads get ten times it.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T> {
        let req = self.request(Method::GET, path, query).await?;
        self.send_json(req, true).await
    }

    pub async fn post<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let req = self.request(Method::POST, path, &[]).await?.json(body);
        self.send_json(req, false).await
    }

    pub async fn put<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let req = self.request(Method::PUT, path, &[]).await?.json(body);
        self.send_json(req, false).await
    }

    /// PUT for endpoints that return `204 No Content` (e.g. Jira issue update).
    pub async fn put_no_content<B: Serialize + ?Sized>(&self, path: &str, body: &B) -> Result<()> {
        let req = self.request(Method::PUT, path, &[]).await?.json(body);
        let resp = self.send(req, false).await?;
        Self::check_status(resp).await.map(|_| ())
    }

    /// POST for endpoints that return `204 No Content` (e.g. Jira transitions).
    pub async fn post_no_content<B: Serialize + ?Sized>(&self, path: &str, body: &B) -> Result<()> {
        let req = self.request(Method::POST, path, &[]).await?.json(body);
        let resp = self.send(req, false).await?;
        Self::check_status(resp).await.map(|_| ())
    }

    pub async fn delete(&self, path: &str, query: &[(&str, &str)]) -> Result<()> {
        let req = self.request(Method::DELETE, path, query).await?;
        let resp = self.send(req, false).await?;
        Self::check_status(resp).await.map(|_| ())
    }

    /// Downloads raw bytes from a link the API itself returned — a Jira
    /// attachment's `content` URL, a Confluence `_links.download` path.
    ///
    /// Such links are not model-composed identifiers, so the D31 path check
    /// does not apply to them: Confluence's download links always carry a
    /// query string (`?version=1&modificationDate=…&api=v2`), and rejecting
    /// `?` here broke every real download. What *does* apply is origin: an
    /// absolute link must share the configured base URL's origin, because
    /// this request carries the user's credentials (D33).
    ///
    /// Buffers the whole body; for attachments use [`Self::download_to_file`].
    pub async fn get_bytes(&self, link: &str) -> Result<Vec<u8>> {
        let resp = self.start_download(link).await?;
        Ok(resp.bytes().await?.to_vec())
    }

    /// Streams a returned link into `path`, never holding more than one chunk
    /// in memory, and stops — removing the partial file — once `max_bytes`
    /// is exceeded (D37). Returns the size written.
    pub async fn download_to_file(
        &self,
        link: &str,
        path: &Path,
        max_bytes: Option<u64>,
    ) -> Result<u64> {
        let too_large = |size: u64| {
            Error::File(format!(
                "attachment is {size} bytes, over the MAX_ATTACHMENT_BYTES limit of {} bytes",
                max_bytes.unwrap_or(0)
            ))
        };
        let resp = self.start_download(link).await?;
        if let (Some(max), Some(len)) = (max_bytes, resp.content_length()) {
            if len > max {
                return Err(too_large(len));
            }
        }
        let file_error = |e: std::io::Error| Error::File(format!("{}: {e}", path.display()));
        let mut file = tokio::fs::File::create(path).await.map_err(file_error)?;
        let mut stream = resp.bytes_stream();
        let mut written = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            written += chunk.len() as u64;
            if max_bytes.is_some_and(|max| written > max) {
                drop(file);
                let _ = tokio::fs::remove_file(path).await;
                return Err(too_large(written));
            }
            file.write_all(&chunk).await.map_err(file_error)?;
        }
        file.flush().await.map_err(file_error)?;
        Ok(written)
    }

    async fn start_download(&self, link: &str) -> Result<Response> {
        let url = self.returned_url(link)?;
        let req = shared_http()
            .request(Method::GET, url)
            .timeout(self.timeout * DOWNLOAD_TIMEOUT_FACTOR);
        let req = self.authorize(req).await?;
        let resp = self.send(req, true).await?;
        Self::check_status(resp).await
    }

    /// Whether an absolute URL the API returned points at the configured
    /// instance. False for a relative link, which always resolves against the
    /// base and is therefore same-origin by construction.
    pub fn same_origin(&self, link: &str) -> bool {
        Url::parse(link).is_ok_and(|url| url.origin() == self.base_url.origin())
    }

    /// Resolves a link the API returned: absolute links must be same-origin,
    /// relative ones are joined onto the base (keeping any query string).
    fn returned_url(&self, link: &str) -> Result<Url> {
        let invalid = |e: url::ParseError| Error::InvalidUrl {
            url: link.to_string(),
            message: e.to_string(),
        };
        let url = match Url::parse(link) {
            Ok(url) => url,
            Err(url::ParseError::RelativeUrlWithoutBase) => self
                .base_url
                .join(link.trim_start_matches('/'))
                .map_err(invalid)?,
            Err(e) => return Err(invalid(e)),
        };
        if !matches!(url.scheme(), "http" | "https") {
            return Err(Error::InvalidUrl {
                url: link.to_string(),
                message: "only http and https links can be downloaded".into(),
            });
        }
        if url.origin() != self.base_url.origin() {
            return Err(Error::Config(format!(
                "refusing to send credentials to foreign origin `{}` (base is `{}`)",
                url.origin().ascii_serialization(),
                self.base_url.origin().ascii_serialization(),
            )));
        }
        Ok(url)
    }

    /// Uploads one file as `multipart/form-data` (field name `file`).
    /// Sends `X-Atlassian-Token: no-check` as required by attachment endpoints.
    pub async fn post_multipart<T: DeserializeOwned>(
        &self,
        path: &str,
        upload: Upload,
    ) -> Result<T> {
        let part = reqwest::multipart::Part::stream_with_length(upload.body, upload.len)
            .file_name(upload.file_name);
        let form = reqwest::multipart::Form::new().part("file", part);
        let req = self
            .request(Method::POST, path, &[])
            .await?
            .timeout(self.timeout * DOWNLOAD_TIMEOUT_FACTOR)
            .header("X-Atlassian-Token", "no-check")
            .multipart(form);
        // A streamed body is not cloneable — no retry here.
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
        let mut req = shared_http().request(method, url).timeout(self.timeout);
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

    /// Sends the request with bounded retries (D40).
    ///
    /// A 429 is retried for any method, after `Retry-After`: the request was
    /// refused, not performed. Transport failures (connect, timeout, reset)
    /// and 502/503/504 are retried only when `idempotent` — a GET — because a
    /// POST that timed out may well have landed.
    async fn send(&self, req: RequestBuilder, idempotent: bool) -> Result<Response> {
        let mut req = req;
        for attempt in 0..=MAX_RETRIES {
            let retry = req.try_clone();
            let outcome = req.send().await;
            let delay = match &outcome {
                Ok(resp) if resp.status() == StatusCode::TOO_MANY_REQUESTS => {
                    Some(retry_after(resp))
                }
                Ok(resp) if idempotent && matches!(resp.status().as_u16(), 502..=504) => {
                    Some(backoff(attempt))
                }
                Err(e) if idempotent && (e.is_connect() || e.is_timeout() || e.is_request()) => {
                    Some(backoff(attempt))
                }
                _ => None,
            };
            match (delay, retry) {
                (Some(delay), Some(next)) if attempt < MAX_RETRIES => {
                    tracing::warn!(
                        ?delay,
                        attempt = attempt + 1,
                        "Atlassian request did not succeed, retrying"
                    );
                    tokio::time::sleep(delay).await;
                    req = next;
                }
                _ => {
                    return match outcome {
                        Ok(resp) if resp.status() == StatusCode::TOO_MANY_REQUESTS => {
                            Err(Error::RateLimited)
                        }
                        other => Ok(other?),
                    };
                }
            }
        }
        unreachable!("the loop returns on its last attempt")
    }

    async fn send_json<T: DeserializeOwned>(
        &self,
        req: RequestBuilder,
        idempotent: bool,
    ) -> Result<T> {
        let resp = self.send(req, idempotent).await?;
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

/// What a 429 asked for, capped; one second when it did not say.
fn retry_after(resp: &Response) -> Duration {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(1))
        .min(MAX_RETRY_AFTER)
}

/// 500 ms, then 1 s — short, because the caller is an LLM waiting on a tool.
fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(500 << attempt)
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
                "request path contains {reason}; check the issue key, page id or \
                 attachment id that was passed"
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
    use super::{backoff, check_path, AtlassianClient};
    use crate::{Auth, ServiceConfig};

    fn client(base: &str) -> AtlassianClient {
        AtlassianClient::new(&ServiceConfig {
            base_url: base.into(),
            auth: Auth::Pat { token: "t".into() },
            deployment: None,
        })
        .unwrap()
    }

    #[test]
    fn a_returned_link_keeps_its_query_string() {
        // What Confluence actually returns in `_links.download`; the D31 path
        // check must not apply to a link the API composed itself.
        let client = client("https://example.atlassian.net/wiki");
        let url = client
            .returned_url("/download/attachments/123/x.png?version=1&modificationDate=2&api=v2")
            .unwrap();
        assert_eq!(
            url.as_str(),
            "https://example.atlassian.net/wiki/download/attachments/123/x.png?version=1&modificationDate=2&api=v2"
        );
    }

    #[test]
    fn a_returned_link_must_share_the_origin() {
        let client = client("https://example.atlassian.net");
        assert!(client
            .returned_url("https://example.atlassian.net/secure/attachment/1/a.pdf")
            .is_ok());
        let error = client
            .returned_url("https://evil.example.com/steal")
            .unwrap_err()
            .to_string();
        assert!(error.contains("foreign origin"), "{error}");
        assert!(client.same_origin("https://example.atlassian.net/x"));
        assert!(!client.same_origin("https://other.atlassian.net/x"));
        assert!(!client.same_origin("/relative"));
    }

    #[test]
    fn a_returned_link_with_another_scheme_is_refused() {
        let client = client("https://example.atlassian.net");
        let error = client
            .returned_url("file:///etc/passwd")
            .unwrap_err()
            .to_string();
        assert!(error.contains("http"), "{error}");
    }

    #[test]
    fn backoff_doubles_and_stays_short() {
        assert_eq!(backoff(0).as_millis(), 500);
        assert_eq!(backoff(1).as_millis(), 1000);
    }

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
