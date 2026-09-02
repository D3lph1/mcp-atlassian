//! Spaces and page labels — the taxonomy side of Confluence.

use crate::ConfluenceTools;
use crate::{Label, ResultsPage, Space};
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use atlassian_client::mcp::{page_size, to_mcp_error};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConfluenceGetSpacesArgs {
    /// Max spaces to return (default 25).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConfluenceGetLabelsArgs {
    /// Numeric page id.
    pub page_id: String,
    /// Max labels to return (default 50, cap 50).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConfluenceAddLabelArgs {
    /// Numeric page id.
    pub page_id: String,
    /// Label name, e.g. `ops`.
    pub label: String,
}

#[tool_router(router = confluence_spaces_router, vis = "pub(crate)")]
impl ConfluenceTools {
    #[tool(
        title = "List Confluence spaces",
        description = "List Confluence spaces visible to the authenticated user.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn confluence_get_spaces(
        &self,
        Parameters(args): Parameters<ConfluenceGetSpacesArgs>,
    ) -> Result<Json<ResultsPage<Space>>, McpError> {
        let spaces = self
            .client()
            .get_spaces(page_size(args.limit, 25))
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(spaces))
    }

    #[tool(
        title = "Get Confluence labels",
        description = "List labels of a Confluence page.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn confluence_get_labels(
        &self,
        Parameters(args): Parameters<ConfluenceGetLabelsArgs>,
    ) -> Result<Json<ResultsPage<Label>>, McpError> {
        let labels = self
            .client()
            .get_labels(&args.page_id, page_size(args.limit, 50))
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(labels))
    }

    #[tool(
        title = "Add Confluence label",
        description = "Add a label to a Confluence page.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn confluence_add_label(
        &self,
        Parameters(args): Parameters<ConfluenceAddLabelArgs>,
    ) -> Result<Json<ResultsPage<Label>>, McpError> {
        let labels = self
            .client()
            .add_label(&args.page_id, &args.label)
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(labels))
    }
}
