//! Conversion between Confluence storage format and Markdown (D10), plus the
//! LLM-facing projections of Confluence content. These types are what the
//! page tools advertise through `outputSchema`, so they are declared here
//! rather than built ad hoc as untyped JSON.

use crate::{Content, Space, Version};
use rmcp::ErrorData as McpError;
use serde::Serialize;

/// A page or comment as returned to the client: metadata plus the body
/// converted to Markdown.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PageView {
    pub id: String,
    /// `page`, `comment`, ...
    #[serde(rename = "type")]
    pub content_type: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space: Option<Space>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<Version>,
    /// Body converted from storage format; absent when the body was not
    /// requested or the content has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_markdown: Option<String>,
}

/// One node of a space's page hierarchy.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PageNode {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<PageNode>,
}

/// Line diff between two versions of a page.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PageDiff {
    pub page_id: String,
    pub from_version: u64,
    pub to_version: u64,
    /// Unified-style diff: `-` removed, `+` added, leading space for context.
    pub diff: String,
}

/// Converts tool content input to storage format according to
/// `content_format` (`markdown` default, `storage` passthrough).
pub(super) fn to_storage(content: &str, format: Option<&str>) -> Result<String, McpError> {
    match format.unwrap_or("markdown") {
        "markdown" => Ok(mcp_atlassian_storage_markdown::markdown_to_storage(content)),
        "storage" => Ok(content.to_string()),
        other => Err(McpError::invalid_params(
            format!("unknown content_format `{other}`: use `markdown` or `storage`"),
            None,
        )),
    }
}

/// Projects a Confluence content entity into the LLM-facing view, converting
/// the storage body to Markdown (D10).
pub(crate) fn page_to_markdown_view(page: &Content) -> PageView {
    PageView {
        id: page.id.clone(),
        content_type: page.content_type.clone(),
        title: page.title.clone(),
        space: page.space.clone(),
        version: page.version.clone(),
        body_markdown: page
            .body
            .as_ref()
            .and_then(|b| b.storage.as_ref())
            .map(|s| mcp_atlassian_storage_markdown::storage_to_markdown(&s.value)),
    }
}
