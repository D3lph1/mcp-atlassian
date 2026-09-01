//! CQL search over Confluence content.

use crate::ConfluenceTools;
use crate::{Content, ResultsPage};
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use atlassian_client::mcp::{to_mcp_error, MAX_SEARCH_RESULTS};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConfluenceSearchArgs {
    /// CQL query, e.g. `space = DEV AND title ~ "runbook"` or `text ~ "deploy"`
    pub cql: String,
    /// Max results (default 10, cap 50). Keep small to save tokens.
    pub limit: Option<u32>,
    /// Pagination offset.
    pub start: Option<u32>,
}

#[tool_router(router = confluence_search_router, vis = "pub(crate)")]
impl ConfluenceTools {
    #[tool(
        description = "Search Confluence content with CQL, e.g. `space = DEV AND title ~ \"runbook\"` or `text ~ \"deploy process\"`. Use this first when looking for pages; keep limit at 10 or less.",
        annotations(read_only_hint = true)
    )]
    async fn confluence_search(
        &self,
        Parameters(args): Parameters<ConfluenceSearchArgs>,
    ) -> Result<Json<ResultsPage<Content>>, McpError> {
        let page = self
            .client()
            .search(
                &args.cql,
                args.limit.unwrap_or(10).min(MAX_SEARCH_RESULTS),
                args.start.unwrap_or(0),
            )
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(page))
    }
}
