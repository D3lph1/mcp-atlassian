//! Field discovery. Custom fields are addressed by opaque ids
//! (`customfield_10011`) that differ per instance, so an LLM has to look them
//! up before it can write to them through jira_create_issue or
//! jira_update_issue.

use crate::JiraTools;
use crate::{Field, FieldOption};
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use atlassian_client::mcp::{list_result, to_mcp_error, ListResult, MAX_SEARCH_RESULTS};

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
    /// Max options to return (default 50).
    pub max_results: Option<u32>,
}

#[tool_router(router = jira_fields_router, vis = "pub(crate)")]
impl JiraTools {
    #[tool(
        description = "Find Jira field definitions by name. Call this to resolve a custom field's id (e.g. \"Story Points\" -> customfield_10011) before setting it in jira_create_issue or jira_update_issue, or before using it in JQL.",
        annotations(read_only_hint = true)
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
        description = "List the allowed values of a select-style custom field (select, multi-select, radio, checkbox). Use it to pick a valid option before writing to the field.",
        annotations(read_only_hint = true)
    )]
    async fn jira_get_field_options(
        &self,
        Parameters(args): Parameters<GetFieldOptionsArgs>,
    ) -> Result<Json<ListResult<FieldOption>>, McpError> {
        let options = self
            .client()
            .get_field_options(
                &args.field_id,
                args.max_results.unwrap_or(50).min(MAX_SEARCH_RESULTS),
            )
            .await
            .map_err(to_mcp_error)?;
        list_result(options)
    }
}
