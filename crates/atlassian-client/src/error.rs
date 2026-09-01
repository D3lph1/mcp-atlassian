pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by the Atlassian HTTP layer.
///
/// Messages are written to be actionable for an LLM reading them through an
/// MCP client: they name the entity and hint at the likely fix (D13).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("invalid URL `{url}`: {message}")]
    InvalidUrl { url: String, message: String },

    #[error("authentication failed (HTTP 401): check the API token / personal access token and username")]
    Unauthorized,

    #[error(
        "permission denied (HTTP 403): the authenticated user lacks permission for this operation"
    )]
    Forbidden,

    #[error("not found (HTTP 404): {0}")]
    NotFound(String),

    #[error("rate limited (HTTP 429) and retry did not help; try again later")]
    RateLimited,

    #[error("Atlassian API error (HTTP {status}): {message}")]
    Api { status: u16, message: String },

    #[error("OAuth error: {0}")]
    OAuth(String),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("failed to decode API response: {0}")]
    Decode(String),
}
