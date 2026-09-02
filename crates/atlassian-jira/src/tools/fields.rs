//! Field discovery. Custom fields are addressed by opaque ids
//! (`customfield_10011`) that differ per instance, so an LLM has to look them
//! up before it can write to them through jira_create_issue or
//! jira_update_issue.

use crate::JiraTools;
use crate::{Field, FieldOption, FieldOptionsScope};
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use atlassian_client::mcp::{list_result, page_size, to_mcp_error, ListResult};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchFieldsArgs {
    /// Case-insensitive substring matched against field name and id, e.g.
    /// `story points`. Omit to list every field.
    pub query: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetFieldOptionsArgs {
    /// Field id from jira_search_fields, e.g. `customfield_10011`.
    pub field_id: String,
    /// Read the options as offered when editing this issue, e.g. `PROJ-123`.
    /// Preferred: works for any user on every deployment.
    pub issue_key: Option<String>,
    /// Read the options as offered when creating an issue in this project.
    pub project_key: Option<String>,
    /// Issue type name for `project_key`, e.g. `Bug`; the project's first
    /// type when omitted.
    pub issue_type: Option<String>,
    /// Max options to return (default 50, cap 50).
    pub max_results: Option<u32>,
}

#[tool_router(router = jira_fields_router, vis = "pub(crate)")]
impl JiraTools {
    #[tool(
        title = "Search Jira fields",
        description = "Find Jira field definitions by name. Call this to resolve a custom field's id (e.g. \"Story Points\" -> customfield_10011) before setting it in jira_create_issue or jira_update_issue, or before using it in JQL.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn jira_search_fields(
        &self,
        Parameters(args): Parameters<SearchFieldsArgs>,
    ) -> Result<Json<ListResult<Field>>, McpError> {
        let fields = self
            .client()
            .search_fields(args.query.as_deref())
            .await
            .map_err(to_mcp_error)?;
        list_result(fields)
    }

    #[tool(
        title = "Get Jira field options",
        description = "List the allowed values of a select-style field (select, multi-select, radio, checkbox, priority, version, component). Pass issue_key when about to edit an issue, or project_key (+ issue_type) when about to create one; without either only Jira Cloud administrators get an answer. Use it to pick a valid option before writing to the field.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn jira_get_field_options(
        &self,
        Parameters(args): Parameters<GetFieldOptionsArgs>,
    ) -> Result<Json<ListResult<FieldOption>>, McpError> {
        let options = self
            .client()
            .get_field_options(
                &args.field_id,
                FieldOptionsScope {
                    issue_key: args.issue_key.as_deref(),
                    project_key: args.project_key.as_deref(),
                    issue_type: args.issue_type.as_deref(),
                },
                page_size(args.max_results, 50),
            )
            .await
            .map_err(to_mcp_error)?;
        list_result(options)
    }
}
