//! Shared building blocks for exposing Atlassian data through MCP.
//!
//! Lives here so both product crates use the same result shapes without
//! depending on each other. Gated behind the `mcp` feature so the clients stay
//! usable as plain REST libraries (D15).

use rmcp::{handler::server::wrapper::Json, ErrorData as McpError};
use serde::Serialize;

/// Search results are capped to keep tool output token-friendly.
pub const MAX_SEARCH_RESULTS: u32 = 50;

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

/// Maps transport-layer errors to MCP errors, preserving the actionable
/// message (D13).
pub fn to_mcp_error(err: crate::Error) -> McpError {
    McpError::internal_error(err.to_string(), None)
}
