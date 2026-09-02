//! Agile API (`/rest/agile/1.0`): boards, sprints and sprint membership.
//! Discovery chain: board id -> sprint id -> issues.

use crate::JiraTools;
use crate::{AgilePage, Board, SearchPage, Sprint};
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use atlassian_client::mcp::{page_size, status_result, to_mcp_error, StatusResult};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetBoardsArgs {
    /// Filter boards by project key, e.g. `PROJ`; omit for all boards.
    pub project_key: Option<String>,
    /// Max boards to return (default 25).
    pub max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetSprintsArgs {
    /// Numeric board id (see jira_get_boards).
    pub board_id: u64,
    /// Filter by state: `active`, `future`, `closed` (comma-separated allowed). Omit for all.
    pub state: Option<String>,
    /// Max sprints to return (default 25, cap 50).
    pub max_results: Option<u32>,
    /// Offset of the first sprint; use the previous page's `startAt` plus its
    /// length while `isLast` is false.
    pub start_at: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetSprintIssuesArgs {
    /// Numeric sprint id (see jira_get_sprints).
    pub sprint_id: u64,
    /// Max issues to return (default 25, cap 50).
    pub max_results: Option<u32>,
    /// Offset of the first issue to return; use the previous page's `startAt`
    /// plus its length to fetch the next page.
    pub start_at: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MoveToSprintArgs {
    /// Numeric sprint id (see jira_get_sprints).
    pub sprint_id: u64,
    /// Issue keys to move into the sprint (max 50).
    pub issue_keys: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetBoardIssuesArgs {
    /// Numeric board id (see jira_get_boards).
    pub board_id: u64,
    /// Optional JQL to narrow the board's issues, e.g. `status = "In Progress"`.
    pub jql: Option<String>,
    /// Max issues to return (default 25, cap 50).
    pub max_results: Option<u32>,
    /// Offset of the first issue to return; use the previous page's `startAt`
    /// plus its length to fetch the next page.
    pub start_at: Option<u32>,
}

#[tool_router(router = jira_agile_router, vis = "pub(crate)")]
impl JiraTools {
    #[tool(
        title = "List Jira boards",
        description = "List Jira agile boards (scrum/kanban), optionally filtered by project key. Use to find board ids for sprint tools.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn jira_get_boards(
        &self,
        Parameters(args): Parameters<GetBoardsArgs>,
    ) -> Result<Json<AgilePage<Board>>, McpError> {
        let boards = self
            .client()
            .get_boards(args.project_key.as_deref(), page_size(args.max_results, 25))
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(boards))
    }

    #[tool(
        title = "List Jira sprints",
        description = "List sprints of a Jira agile board. Filter by state (active/future/closed) to keep output small.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn jira_get_sprints(
        &self,
        Parameters(args): Parameters<GetSprintsArgs>,
    ) -> Result<Json<AgilePage<Sprint>>, McpError> {
        let sprints = self
            .client()
            .get_sprints(
                args.board_id,
                args.state.as_deref(),
                page_size(args.max_results, 25),
                args.start_at.unwrap_or(0),
            )
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(sprints))
    }

    #[tool(
        title = "List Jira sprint issues",
        description = "List issues in a Jira sprint with a compact field set.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn jira_get_sprint_issues(
        &self,
        Parameters(args): Parameters<GetSprintIssuesArgs>,
    ) -> Result<Json<SearchPage>, McpError> {
        let page = self
            .client()
            .get_sprint_issues(
                args.sprint_id,
                page_size(args.max_results, 25),
                // An offset, not a page size — capping it would cap paging.
                args.start_at.unwrap_or(0),
            )
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(page))
    }

    #[tool(
        title = "Move Jira issues to sprint",
        description = "Move Jira issues into a sprint (max 50 issue keys per call).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn jira_move_to_sprint(
        &self,
        Parameters(args): Parameters<MoveToSprintArgs>,
    ) -> Result<Json<StatusResult>, McpError> {
        self.client()
            .move_issues_to_sprint(args.sprint_id, &args.issue_keys)
            .await
            .map_err(to_mcp_error)?;
        status_result(format!(
            "Moved {} issue(s) to sprint {}",
            args.issue_keys.len(),
            args.sprint_id
        ))
    }

    #[tool(
        title = "List Jira board issues",
        description = "List issues on a Jira agile board, optionally narrowed by JQL. Unlike jira_search this respects the board's own filter, so it shows what the board actually displays.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn jira_get_board_issues(
        &self,
        Parameters(args): Parameters<GetBoardIssuesArgs>,
    ) -> Result<Json<SearchPage>, McpError> {
        let page = self
            .client()
            .get_board_issues(
                args.board_id,
                args.jql.as_deref(),
                page_size(args.max_results, 25),
                args.start_at.unwrap_or(0),
            )
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(page))
    }
}
