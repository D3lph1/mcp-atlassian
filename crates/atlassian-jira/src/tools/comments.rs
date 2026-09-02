//! Comments and worklog — the two things users append to an issue's activity.

use crate::JiraTools;
use crate::{Comment, CommentPage, Worklog, WorklogEntry};
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use atlassian_client::mcp::{list_result, page_size, to_mcp_error, ListResult};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddCommentArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
    /// Comment body (plain text or Jira wiki markup).
    pub body: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetCommentsArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
    /// Max comments to return, newest first (default 10, cap 50).
    pub max_results: Option<u32>,
    /// Offset into the issue's comments for paging; compare `start_at +
    /// comments.len()` with `total`.
    pub start_at: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddWorklogArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
    /// Time spent in Jira duration syntax, e.g. `2h`, `1d 4h`, `30m`.
    pub time_spent: String,
    pub comment: Option<String>,
    /// Start timestamp, ISO-8601 with offset, e.g. `2026-01-15T10:00:00.000+0000`. Defaults to now.
    pub started: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EditCommentArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
    /// Comment id from jira_get_comments.
    pub comment_id: String,
    /// Replacement body (plain text or Jira wiki markup).
    pub body: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetWorklogArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
    /// Max entries to return (default 25, cap 50).
    pub max_results: Option<u32>,
}

#[tool_router(router = jira_comments_router, vis = "pub(crate)")]
impl JiraTools {
    #[tool(
        description = "Add a comment to a Jira issue.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn jira_add_comment(
        &self,
        Parameters(args): Parameters<AddCommentArgs>,
    ) -> Result<Json<Comment>, McpError> {
        let comment = self
            .client()
            .add_comment(&args.issue_key, &args.body)
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(comment))
    }

    #[tool(
        description = "Get comments of a Jira issue, newest first.",
        annotations(read_only_hint = true)
    )]
    async fn jira_get_comments(
        &self,
        Parameters(args): Parameters<GetCommentsArgs>,
    ) -> Result<Json<CommentPage>, McpError> {
        let comments = self
            .client()
            .get_comments(
                &args.issue_key,
                page_size(args.max_results, 10),
                // An offset, not a page size — capping it would cap paging.
                args.start_at.unwrap_or(0),
            )
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(comments))
    }

    #[tool(
        description = "Log work time on a Jira issue. time_spent uses Jira duration syntax: 30m, 2h, 1d 4h.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn jira_add_worklog(
        &self,
        Parameters(args): Parameters<AddWorklogArgs>,
    ) -> Result<Json<Worklog>, McpError> {
        let worklog = self
            .client()
            .add_worklog(
                &args.issue_key,
                &args.time_spent,
                args.comment.as_deref(),
                args.started.as_deref(),
            )
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(worklog))
    }

    #[tool(
        description = "Edit an existing comment on a Jira issue. Get comment ids from jira_get_comments.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn jira_edit_comment(
        &self,
        Parameters(args): Parameters<EditCommentArgs>,
    ) -> Result<Json<Comment>, McpError> {
        let comment = self
            .client()
            .edit_comment(&args.issue_key, &args.comment_id, &args.body)
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(comment))
    }

    #[tool(
        description = "List the work logged on a Jira issue: who logged how much time, when, and with what note. Use it to answer time-tracking questions.",
        annotations(read_only_hint = true)
    )]
    async fn jira_get_worklog(
        &self,
        Parameters(args): Parameters<GetWorklogArgs>,
    ) -> Result<Json<ListResult<WorklogEntry>>, McpError> {
        let entries = self
            .client()
            .get_worklog(&args.issue_key, page_size(args.max_results, 25))
            .await
            .map_err(to_mcp_error)?;
        list_result(entries)
    }
}
