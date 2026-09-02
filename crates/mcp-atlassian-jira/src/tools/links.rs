//! Issue links: typed links between issues, epic membership and remote
//! (external URL) links.

use crate::JiraTools;
use crate::{LinkType, RemoteLink};
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use mcp_atlassian_client::mcp::{
    list_result, status_result, to_mcp_error, ListResult, StatusResult,
};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateIssueLinkArgs {
    /// Link type name from jira_get_link_types, e.g. `Blocks`, `Relates`.
    pub link_type: String,
    /// Issue on the inward side of the type's phrasing (for `Blocks`: the
    /// issue that IS BLOCKED BY the other one).
    pub inward_issue: String,
    /// Issue on the outward side (for `Blocks`: the issue that BLOCKS).
    pub outward_issue: String,
    /// Optional comment added alongside the link.
    pub comment: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RemoveIssueLinkArgs {
    /// Link id: the `id` of an entry in the `issuelinks` field of
    /// jira_get_issue.
    pub link_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateRemoteLinkArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
    /// Absolute URL to link to (a Confluence page, a dashboard, a PR).
    pub url: String,
    /// Link title shown in Jira.
    pub title: String,
    /// Optional longer description.
    pub summary: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LinkToEpicArgs {
    /// Issue to put under the epic, e.g. `PROJ-123`
    pub issue_key: String,
    /// Epic key, e.g. `PROJ-1`
    pub epic_key: String,
}

#[tool_router(router = jira_links_router, vis = "pub(crate)")]
impl JiraTools {
    #[tool(
        title = "List Jira issue link types",
        description = "List available Jira issue link types with their inward/outward phrasing. Call before jira_create_issue_link to pick a valid type name and get the direction right.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn jira_get_link_types(&self) -> Result<Json<ListResult<LinkType>>, McpError> {
        let types = self.client().get_link_types().await.map_err(to_mcp_error)?;
        list_result(types)
    }

    #[tool(
        title = "Link Jira issues",
        description = "Link two Jira issues, e.g. mark one as blocking another. Direction follows the link type's phrasing: for `Blocks`, outward_issue blocks inward_issue.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn jira_create_issue_link(
        &self,
        Parameters(args): Parameters<CreateIssueLinkArgs>,
    ) -> Result<Json<StatusResult>, McpError> {
        self.client()
            .create_issue_link(
                &args.link_type,
                &args.inward_issue,
                &args.outward_issue,
                args.comment.as_deref(),
            )
            .await
            .map_err(to_mcp_error)?;
        status_result(format!(
            "Linked {} and {} as `{}`",
            args.outward_issue, args.inward_issue, args.link_type
        ))
    }

    #[tool(
        title = "Remove Jira issue link",
        description = "Remove a link between two Jira issues by its link id.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn jira_remove_issue_link(
        &self,
        Parameters(args): Parameters<RemoveIssueLinkArgs>,
    ) -> Result<Json<StatusResult>, McpError> {
        self.client()
            .remove_issue_link(&args.link_id)
            .await
            .map_err(to_mcp_error)?;
        status_result(format!("Link {} removed", args.link_id))
    }

    #[tool(
        title = "Add remote link to Jira issue",
        description = "Attach an external URL to a Jira issue as a remote link — a Confluence page, a pull request, a dashboard.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn jira_create_remote_link(
        &self,
        Parameters(args): Parameters<CreateRemoteLinkArgs>,
    ) -> Result<Json<RemoteLink>, McpError> {
        let link = self
            .client()
            .create_remote_issue_link(
                &args.issue_key,
                &args.url,
                &args.title,
                args.summary.as_deref(),
            )
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(link))
    }

    #[tool(
        title = "Link Jira issue to epic",
        description = "Put a Jira issue under an epic. Uses the parent field on Cloud and the Epic Link custom field on Server/Data Center.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn jira_link_to_epic(
        &self,
        Parameters(args): Parameters<LinkToEpicArgs>,
    ) -> Result<Json<StatusResult>, McpError> {
        self.client()
            .link_to_epic(&args.issue_key, &args.epic_key)
            .await
            .map_err(to_mcp_error)?;
        status_result(format!(
            "Issue {} linked to epic {}",
            args.issue_key, args.epic_key
        ))
    }
}
