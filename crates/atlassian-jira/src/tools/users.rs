//! User lookup and watchers. On Cloud a user is identified by `accountId`,
//! on Server/Data Center by username — the client picks the right parameter,
//! and the tools echo whichever the deployment returns.

use crate::JiraTools;
use crate::{User, Watchers};
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use atlassian_client::mcp::{
    list_result, status_result, to_mcp_error, ListResult, StatusResult, MAX_SEARCH_RESULTS,
};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetUserProfileArgs {
    /// Account id on Jira Cloud, username on Server/Data Center
    /// (see jira_search_users).
    pub identifier: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchAssignableUsersArgs {
    /// Name or email fragment to match.
    pub query: String,
    /// Restrict to users assignable in this project, e.g. `PROJ`.
    pub project_key: Option<String>,
    /// Restrict to users assignable to this issue, e.g. `PROJ-123`.
    /// Takes precedence over project_key.
    pub issue_key: Option<String>,
    /// Max users to return (default 10, cap 50).
    pub max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WatcherArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ModifyWatcherArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
    /// Account id on Jira Cloud, username on Server/Data Center.
    pub user: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AssignIssueArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
    /// Account id on Cloud, username on Server/Data Center. Omit to unassign.
    pub assignee: Option<String>,
}

#[tool_router(router = jira_users_router, vis = "pub(crate)")]
impl JiraTools {
    #[tool(
        description = "Get one Jira user by identifier (account id on Cloud, username on Server/Data Center). Use jira_search_users first if you only know the person's name.",
        annotations(read_only_hint = true)
    )]
    async fn jira_get_user_profile(
        &self,
        Parameters(args): Parameters<GetUserProfileArgs>,
    ) -> Result<Json<User>, McpError> {
        let user = self
            .client()
            .get_user_profile(&args.identifier)
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(user))
    }

    #[tool(
        description = "Find users who can actually be assigned to a project or issue. Prefer this over jira_search_users when picking an assignee — it filters by assignable permission, so it cannot suggest someone Jira would reject.",
        annotations(read_only_hint = true)
    )]
    async fn jira_search_assignable_users(
        &self,
        Parameters(args): Parameters<SearchAssignableUsersArgs>,
    ) -> Result<Json<ListResult<User>>, McpError> {
        let users = self
            .client()
            .search_assignable_users(
                &args.query,
                args.project_key.as_deref(),
                args.issue_key.as_deref(),
                args.max_results.unwrap_or(10).min(MAX_SEARCH_RESULTS),
            )
            .await
            .map_err(to_mcp_error)?;
        list_result(users)
    }

    #[tool(
        description = "Assign a Jira issue to a user, or unassign it by omitting the assignee. Resolve the identifier with jira_search_assignable_users first.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn jira_assign_issue(
        &self,
        Parameters(args): Parameters<AssignIssueArgs>,
    ) -> Result<Json<StatusResult>, McpError> {
        self.client()
            .assign_issue(&args.issue_key, args.assignee.as_deref())
            .await
            .map_err(to_mcp_error)?;
        status_result(match args.assignee {
            Some(user) => format!("Issue {} assigned to {user}", args.issue_key),
            None => format!("Issue {} unassigned", args.issue_key),
        })
    }

    #[tool(
        description = "List the watchers of a Jira issue.",
        annotations(read_only_hint = true)
    )]
    async fn jira_get_watchers(
        &self,
        Parameters(args): Parameters<WatcherArgs>,
    ) -> Result<Json<Watchers>, McpError> {
        let watchers = self
            .client()
            .get_watchers(&args.issue_key)
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(watchers))
    }

    #[tool(
        description = "Add a user as a watcher of a Jira issue.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn jira_add_watcher(
        &self,
        Parameters(args): Parameters<ModifyWatcherArgs>,
    ) -> Result<Json<StatusResult>, McpError> {
        self.client()
            .add_watcher(&args.issue_key, &args.user)
            .await
            .map_err(to_mcp_error)?;
        status_result(format!("{} now watches {}", args.user, args.issue_key))
    }

    #[tool(
        description = "Remove a user from the watchers of a Jira issue.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn jira_remove_watcher(
        &self,
        Parameters(args): Parameters<ModifyWatcherArgs>,
    ) -> Result<Json<StatusResult>, McpError> {
        self.client()
            .remove_watcher(&args.issue_key, &args.user)
            .await
            .map_err(to_mcp_error)?;
        status_result(format!(
            "{} no longer watches {}",
            args.user, args.issue_key
        ))
    }
}
