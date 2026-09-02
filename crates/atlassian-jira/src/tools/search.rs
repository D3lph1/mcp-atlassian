//! JQL search. Pagination differs by deployment — the client routes it
//! (see DECISIONS.md D16); both parameter styles are exposed here.

use crate::JiraTools;
use crate::{SearchPage, SearchParams};
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use atlassian_client::mcp::{page_size, to_mcp_error};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    /// JQL query, e.g. `project = PROJ AND status = "In Progress" ORDER BY updated DESC`
    pub jql: String,
    /// Max results to return (default 10, cap 50). Keep small to save tokens.
    pub max_results: Option<u32>,
    /// Comma-separated fields to include. Default: summary,status,assignee,issuetype,priority,created,updated
    pub fields: Option<String>,
    /// Pagination offset (Jira Server/Data Center only).
    pub start_at: Option<u32>,
    /// Pagination token from a previous page (Jira Cloud only).
    pub next_page_token: Option<String>,
}

#[tool_router(router = jira_search_router, vis = "pub(crate)")]
impl JiraTools {
    #[tool(
        title = "Search Jira issues (JQL)",
        description = "Search Jira issues with JQL. Use this first when looking for issues; keep max_results at 10 or less unless more are needed. Returns a compact field set by default.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn jira_search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<Json<SearchPage>, McpError> {
        let params = SearchParams {
            jql: args.jql,
            max_results: page_size(args.max_results, 10),
            fields: args.fields,
            start_at: args.start_at,
            next_page_token: args.next_page_token,
        };
        let page = self.client().search(&params).await.map_err(to_mcp_error)?;
        Ok(Json(page))
    }
}
