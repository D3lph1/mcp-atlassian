//! Page history: list versions, read an old version, diff two versions.
//! Bodies are converted to Markdown like everywhere else (D10).

use crate::ConfluenceTools;
use crate::{ResultsPage, VersionInfo};
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use super::storage::{page_to_markdown_view, PageDiff, PageView};
use mcp_atlassian_client::mcp::{page_size, to_mcp_error};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetHistoryArgs {
    /// Numeric page id.
    pub page_id: String,
    /// Max versions to return, newest first (default 25).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetVersionArgs {
    /// Numeric page id.
    pub page_id: String,
    /// Version number from confluence_get_page_history.
    pub version: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DiffArgs {
    /// Numeric page id.
    pub page_id: String,
    /// Older version number.
    pub from_version: u64,
    /// Newer version number.
    pub to_version: u64,
}

#[tool_router(router = confluence_versions_router, vis = "pub(crate)")]
impl ConfluenceTools {
    #[tool(
        title = "Get Confluence page history",
        description = "List the version history of a Confluence page: version numbers, authors, timestamps and change messages. Use it before reading or diffing an old version.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn confluence_get_page_history(
        &self,
        Parameters(args): Parameters<GetHistoryArgs>,
    ) -> Result<Json<ResultsPage<VersionInfo>>, McpError> {
        let versions = self
            .client()
            .get_page_versions(&args.page_id, page_size(args.limit, 25))
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(versions))
    }

    #[tool(
        title = "Get Confluence page version",
        description = "Read one historical version of a Confluence page, body converted to Markdown. Get version numbers from confluence_get_page_history.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn confluence_get_page_version(
        &self,
        Parameters(args): Parameters<GetVersionArgs>,
    ) -> Result<Json<PageView>, McpError> {
        let page = self
            .client()
            .get_page_version_body(&args.page_id, args.version)
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(page_to_markdown_view(&page)))
    }

    #[tool(
        title = "Diff Confluence page versions",
        description = "Compare two versions of a Confluence page and return a unified line diff of their Markdown. Use it to answer what changed between versions.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn confluence_get_page_diff(
        &self,
        Parameters(args): Parameters<DiffArgs>,
    ) -> Result<Json<PageDiff>, McpError> {
        let confluence = self.client();
        let from = confluence
            .get_page_version_body(&args.page_id, args.from_version)
            .await
            .map_err(to_mcp_error)?;
        let to = confluence
            .get_page_version_body(&args.page_id, args.to_version)
            .await
            .map_err(to_mcp_error)?;
        let diff = mcp_atlassian_storage_markdown::diff_pages(
            body_markdown(&from).as_deref().unwrap_or(""),
            body_markdown(&to).as_deref().unwrap_or(""),
        );
        Ok(Json(PageDiff {
            page_id: args.page_id,
            from_version: args.from_version,
            to_version: args.to_version,
            diff,
        }))
    }
}

fn body_markdown(page: &crate::Content) -> Option<String> {
    page.body
        .as_ref()
        .and_then(|b| b.storage.as_ref())
        .map(|s| mcp_atlassian_storage_markdown::storage_to_markdown(&s.value))
}
