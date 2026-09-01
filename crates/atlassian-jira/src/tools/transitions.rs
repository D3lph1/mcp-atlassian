//! Status transitions. Always a two-step flow: list valid transitions for the
//! issue's current status, then apply one by id.

use crate::JiraTools;
use crate::Transition;
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use atlassian_client::mcp::{list_result, status_result, to_mcp_error, ListResult, StatusResult};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetTransitionsArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TransitionIssueArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
    /// Transition id from jira_get_transitions (numeric string, e.g. `31`).
    pub transition_id: String,
    /// Optional comment to add with the transition.
    pub comment: Option<String>,
}

#[tool_router(router = jira_transitions_router, vis = "pub(crate)")]
impl JiraTools {
    #[tool(
        description = "List available status transitions for a Jira issue. Call this before jira_transition_issue to get valid transition ids.",
        annotations(read_only_hint = true)
    )]
    async fn jira_get_transitions(
        &self,
        Parameters(args): Parameters<GetTransitionsArgs>,
    ) -> Result<Json<ListResult<Transition>>, McpError> {
        let transitions = self
            .client()
            .get_transitions(&args.issue_key)
            .await
            .map_err(to_mcp_error)?;
        list_result(transitions)
    }

    #[tool(
        description = "Move a Jira issue to another status by applying a transition. Get valid transition ids from jira_get_transitions first.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn jira_transition_issue(
        &self,
        Parameters(args): Parameters<TransitionIssueArgs>,
    ) -> Result<Json<StatusResult>, McpError> {
        self.client()
            .transition_issue(
                &args.issue_key,
                &args.transition_id,
                args.comment.as_deref(),
            )
            .await
            .map_err(to_mcp_error)?;
        status_result(format!(
            "Issue {} transitioned (transition {})",
            args.issue_key, args.transition_id
        ))
    }
}
