//! Discovery tools: the authenticated user, user lookup, projects, issue
//! types. LLMs call these to resolve names before creating or assigning
//! issues.

use crate::JiraTools;
use crate::{IssueType, Myself, Project, User};
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use atlassian_client::mcp::{list_result, to_mcp_error, ListResult, MAX_SEARCH_RESULTS};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchUsersArgs {
    /// Who to look for. On Jira Cloud this matches display name and email
    /// (e.g. `alice` or `alice@company.com`); on Server/Data Center it matches
    /// the username.
    pub query: String,
    /// Max users to return (default 10, cap 50).
    pub max_results: Option<u32>,
}

#[tool_router(router = jira_meta_router, vis = "pub(crate)")]
impl JiraTools {
    #[tool(
        description = "Get the currently authenticated Jira user. Use this to verify credentials or to find the current user's account id / username.",
        annotations(read_only_hint = true)
    )]
    async fn jira_get_myself(&self) -> Result<Json<Myself>, McpError> {
        let myself = self.client().get_myself().await.map_err(to_mcp_error)?;
        Ok(Json(myself))
    }

    #[tool(
        description = "Find Jira users by name or email. Call this to resolve a person's name into the identifier other tools need: the `accountId` on Jira Cloud, the `name` (username) on Server/Data Center. Use it before setting an assignee in jira_create_issue or jira_update_issue, or before writing a JQL clause like `assignee = <id>`.",
        annotations(read_only_hint = true)
    )]
    async fn jira_search_users(
        &self,
        Parameters(args): Parameters<SearchUsersArgs>,
    ) -> Result<Json<ListResult<User>>, McpError> {
        let users = self
            .client()
            .search_users(
                &args.query,
                args.max_results.unwrap_or(10).min(MAX_SEARCH_RESULTS),
            )
            .await
            .map_err(to_mcp_error)?;
        list_result(users)
    }

    #[tool(
        description = "List Jira projects visible to the authenticated user.",
        annotations(read_only_hint = true)
    )]
    async fn jira_get_projects(&self) -> Result<Json<ListResult<Project>>, McpError> {
        let projects = self.client().get_projects().await.map_err(to_mcp_error)?;
        list_result(projects)
    }

    #[tool(
        description = "List all Jira issue types (Task, Bug, Story, ...). Use before jira_create_issue to pick a valid issue type name.",
        annotations(read_only_hint = true)
    )]
    async fn jira_get_issue_types(&self) -> Result<Json<ListResult<IssueType>>, McpError> {
        let types = self
            .client()
            .get_issue_types()
            .await
            .map_err(to_mcp_error)?;
        list_result(types)
    }
}
