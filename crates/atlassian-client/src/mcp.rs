//! Shared building blocks for exposing Atlassian data through MCP.
//!
//! Lives here so both product crates use the same result shapes without
//! depending on each other. Gated behind the `mcp` feature so the clients stay
//! usable as plain REST libraries (D15).

use rmcp::{handler::server::wrapper::Json, ErrorData as McpError};
use serde::Serialize;

/// Search results are capped to keep tool output token-friendly.
pub const MAX_SEARCH_RESULTS: u32 = 50;

/// Resolves a caller-supplied page size: the default when absent, capped at
/// [`MAX_SEARCH_RESULTS`] either way.
///
/// Every list tool goes through this rather than writing
/// `args.limit.unwrap_or(25)` itself. The cap is the point — an uncapped
/// `limit` is passed straight to Atlassian and floods the context window,
/// which is the failure D9 exists to prevent. Ten tools had grown the
/// unwrap without the `.min`, and nothing made that visible.
pub fn page_size(requested: Option<u32>, default: u32) -> u32 {
    requested.unwrap_or(default).min(MAX_SEARCH_RESULTS)
}

/// Wraps a list so the structured result stays a JSON object (D20).
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ListResult<T> {
    pub items: Vec<T>,
    /// Number of items in `items` — not the total available on the server.
    pub count: usize,
}

impl<T> ListResult<T> {
    pub fn new(items: Vec<T>) -> Self {
        Self {
            count: items.len(),
            items,
        }
    }
}

/// Result of an operation whose interesting output is "it worked".
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct StatusResult {
    /// Always true; failures surface as MCP errors, not as `ok: false`.
    pub ok: bool,
    /// Human-readable summary of what changed.
    pub message: String,
}

/// Builds a structured list result.
pub fn list_result<T>(items: Vec<T>) -> Result<Json<ListResult<T>>, McpError> {
    Ok(Json(ListResult::new(items)))
}

/// Builds a structured status result.
pub fn status_result(message: String) -> Result<Json<StatusResult>, McpError> {
    Ok(Json(StatusResult { ok: true, message }))
}

/// Writes downloaded bytes to `save_path` and reports what landed there.
///
/// Shared by the Jira and Confluence download tools, which differ only in how
/// they resolve the attachment's URL. Keeping the write in one place is also
/// what keeps its annotation honest: this is a local filesystem write, so the
/// tools calling it are annotated as destructive writes, not as reads.
pub async fn save_attachment(
    bytes: Vec<u8>,
    file_name: &str,
    save_path: &str,
) -> Result<Json<StatusResult>, McpError> {
    let size = bytes.len();
    tokio::fs::write(save_path, bytes)
        .await
        .map_err(|e| McpError::internal_error(format!("failed to write {save_path}: {e}"), None))?;
    status_result(format!("Saved {file_name} ({size} bytes) to {save_path}"))
}

/// Reads a local file for upload, returning its base name and contents.
///
/// The base name is what Atlassian stores as the attachment name; the rest of
/// the path is the caller's filesystem and none of the instance's business.
pub async fn read_for_upload(file_path: &str) -> Result<(String, Vec<u8>), McpError> {
    let bytes = tokio::fs::read(file_path)
        .await
        .map_err(|e| McpError::invalid_params(format!("cannot read {file_path}: {e}"), None))?;
    let file_name = std::path::Path::new(file_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment")
        .to_string();
    Ok((file_name, bytes))
}

/// Maps transport-layer errors to MCP errors, preserving the actionable
/// message (D13).
pub fn to_mcp_error(err: crate::Error) -> McpError {
    McpError::internal_error(err.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::{page_size, MAX_SEARCH_RESULTS};

    #[test]
    fn an_absent_page_size_falls_back_to_the_default() {
        assert_eq!(page_size(None, 25), 25);
        assert_eq!(page_size(Some(5), 25), 5);
    }

    #[test]
    fn a_page_size_is_capped_however_it_arrives() {
        assert_eq!(page_size(Some(100_000), 25), MAX_SEARCH_RESULTS);
        // Including one a tool author defaulted above the cap.
        assert_eq!(page_size(None, 100), MAX_SEARCH_RESULTS);
    }
}
