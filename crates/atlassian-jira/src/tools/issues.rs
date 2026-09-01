//! Issue CRUD. Status changes live in `transitions` — Jira does not accept
//! them through the field update endpoint.

use crate::JiraTools;
use crate::{
    BatchCreateResult, ChangelogEntry, CreateIssueParams, CreatedIssue, Issue, SearchPage,
};
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;
use serde_json::{Map, Value};

use atlassian_client::mcp::{
    list_result, status_result, to_mcp_error, ListResult, StatusResult, MAX_SEARCH_RESULTS,
};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetIssueArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
    /// Comma-separated fields to include; omit for all fields.
    pub fields: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateIssueArgs {
    /// Project key, e.g. `PROJ`
    pub project_key: String,
    /// Issue type name, e.g. `Task`, `Bug`, `Story` (see jira_get_issue_types)
    pub issue_type: String,
    pub summary: String,
    /// Plain text or Jira wiki markup.
    pub description: Option<String>,
    /// Assignee: account id on Jira Cloud, username on Server/Data Center.
    pub assignee: Option<String>,
    /// Priority name, e.g. `High`.
    pub priority: Option<String>,
    pub labels: Option<Vec<String>>,
    /// Extra raw Jira fields merged into the request, e.g. `{"customfield_10011": "value"}`.
    pub additional_fields: Option<Map<String, Value>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateIssueArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
    /// Raw Jira `fields` object, e.g. `{"summary": "New title", "labels": ["x"]}`.
    pub fields: Map<String, Value>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteIssueArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
    /// Also delete subtasks (default false; deletion fails if subtasks exist and this is false).
    pub delete_subtasks: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BatchCreateIssuesArgs {
    /// One raw Jira `fields` object per issue, e.g.
    /// `[{"project": {"key": "PROJ"}, "issuetype": {"name": "Task"}, "summary": "..."}]`.
    pub issues: Vec<Map<String, Value>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetChangelogArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
    /// Max history entries to return (default 25).
    pub max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetProjectIssuesArgs {
    /// Project key, e.g. `PROJ`
    pub project_key: String,
    /// Max issues to return (default 25, cap 50).
    pub max_results: Option<u32>,
}

#[tool_router(router = jira_issues_router, vis = "pub(crate)")]
impl JiraTools {
    #[tool(
        description = "Get a single Jira issue by key (e.g. PROJ-123) with full fields including description.",
        annotations(read_only_hint = true)
    )]
    async fn jira_get_issue(
        &self,
        Parameters(args): Parameters<GetIssueArgs>,
    ) -> Result<Json<Issue>, McpError> {
        let issue = self
            .client()
            .get_issue(&args.issue_key, args.fields.as_deref())
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(issue))
    }

    #[tool(
        description = "Create a Jira issue. Requires project key, issue type name and summary. Use jira_get_issue_types to discover valid issue type names.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn jira_create_issue(
        &self,
        Parameters(args): Parameters<CreateIssueArgs>,
    ) -> Result<Json<CreatedIssue>, McpError> {
        let params = CreateIssueParams {
            project_key: args.project_key,
            issue_type: args.issue_type,
            summary: args.summary,
            description: args.description,
            assignee: args.assignee,
            priority: args.priority,
            labels: args.labels.unwrap_or_default(),
            additional_fields: args.additional_fields,
        };
        let created = self
            .client()
            .create_issue(&params)
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(created))
    }

    #[tool(
        description = "Update fields of an existing Jira issue. Pass a raw `fields` object, e.g. {\"summary\": \"New title\", \"description\": \"...\", \"labels\": [\"a\"]}. To change status use jira_transition_issue instead.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn jira_update_issue(
        &self,
        Parameters(args): Parameters<UpdateIssueArgs>,
    ) -> Result<Json<StatusResult>, McpError> {
        self.client()
            .update_issue(&args.issue_key, &args.fields)
            .await
            .map_err(to_mcp_error)?;
        status_result(format!("Issue {} updated", args.issue_key))
    }

    #[tool(
        description = "Delete a Jira issue permanently. This cannot be undone — confirm with the user before calling.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn jira_delete_issue(
        &self,
        Parameters(args): Parameters<DeleteIssueArgs>,
    ) -> Result<Json<StatusResult>, McpError> {
        self.client()
            .delete_issue(&args.issue_key, args.delete_subtasks.unwrap_or(false))
            .await
            .map_err(to_mcp_error)?;
        status_result(format!("Issue {} deleted", args.issue_key))
    }

    #[tool(
        description = "Create several Jira issues in one request. Each entry is a raw `fields` object, so custom fields pass through unchanged. Returns created issues and per-entry errors separately.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn jira_batch_create_issues(
        &self,
        Parameters(args): Parameters<BatchCreateIssuesArgs>,
    ) -> Result<Json<BatchCreateResult>, McpError> {
        let result = self
            .client()
            .batch_create_issues(args.issues)
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(result))
    }

    #[tool(
        description = "Get the change history of a Jira issue — who changed which field, from what to what, and when. Use it to answer questions about how an issue reached its current state.",
        annotations(read_only_hint = true)
    )]
    async fn jira_get_changelog(
        &self,
        Parameters(args): Parameters<GetChangelogArgs>,
    ) -> Result<Json<ListResult<ChangelogEntry>>, McpError> {
        let entries = self
            .client()
            .get_changelog(&args.issue_key, args.max_results.unwrap_or(25))
            .await
            .map_err(to_mcp_error)?;
        list_result(entries)
    }

    #[tool(
        description = "List issues of a Jira project, newest first. A shortcut for the common `project = X` query; use jira_search when you need any other filter.",
        annotations(read_only_hint = true)
    )]
    async fn jira_get_project_issues(
        &self,
        Parameters(args): Parameters<GetProjectIssuesArgs>,
    ) -> Result<Json<SearchPage>, McpError> {
        let page = self
            .client()
            .get_project_issues(
                &args.project_key,
                args.max_results.unwrap_or(25).min(MAX_SEARCH_RESULTS),
            )
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(page))
    }
}
