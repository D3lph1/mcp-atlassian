//! Templates, page restrictions and user lookup — the administrative edges of
//! Confluence that page tools do not cover.

use crate::ConfluenceTools;
use crate::{Content, Person, Restrictions, ResultsPage, Template};
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use super::storage::to_storage;
use atlassian_client::mcp::{list_result, page_size, to_mcp_error, ListResult};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListTemplatesArgs {
    /// Restrict to templates of one space, e.g. `DEV`. Omit for global ones.
    pub space_key: Option<String>,
    /// Max templates to return (default 25).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetTemplateArgs {
    /// Template id from confluence_list_templates.
    pub template_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateFromTemplateArgs {
    /// Space key the new page belongs to, e.g. `DEV`.
    pub space_key: String,
    pub title: String,
    /// Template id from confluence_list_templates.
    pub template_id: String,
    /// Optional parent page id.
    pub parent_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetRestrictionsArgs {
    /// Numeric page id.
    pub page_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetRestrictionsArgs {
    /// Numeric page id.
    pub page_id: String,
    /// Users allowed to read; account ids on Cloud, usernames on Server/DC.
    /// An empty list clears the read restriction (page inherits space rights).
    pub read_users: Option<Vec<String>>,
    /// Users allowed to edit. Empty clears the update restriction.
    pub update_users: Option<Vec<String>>,
    /// Groups allowed to read.
    pub read_groups: Option<Vec<String>>,
    /// Groups allowed to edit.
    pub update_groups: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchUsersArgs {
    /// Name fragment to match against user full names.
    pub query: String,
    /// Max users to return (default 10, cap 50).
    pub limit: Option<u32>,
}

#[tool_router(router = confluence_admin_router, vis = "pub(crate)")]
impl ConfluenceTools {
    #[tool(
        description = "List Confluence page templates, optionally for one space. Call before confluence_create_page_from_template to pick a template id.",
        annotations(read_only_hint = true)
    )]
    async fn confluence_list_templates(
        &self,
        Parameters(args): Parameters<ListTemplatesArgs>,
    ) -> Result<Json<ResultsPage<Template>>, McpError> {
        let templates = self
            .client()
            .list_templates(args.space_key.as_deref(), page_size(args.limit, 25))
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(templates))
    }

    #[tool(
        description = "Get one Confluence template including its body.",
        annotations(read_only_hint = true)
    )]
    async fn confluence_get_template(
        &self,
        Parameters(args): Parameters<GetTemplateArgs>,
    ) -> Result<Json<Template>, McpError> {
        let template = self
            .client()
            .get_template(&args.template_id)
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(template))
    }

    #[tool(
        description = "Create a Confluence page from a template. The template's body becomes the page's initial content.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn confluence_create_page_from_template(
        &self,
        Parameters(args): Parameters<CreateFromTemplateArgs>,
    ) -> Result<Json<Content>, McpError> {
        let confluence = self.client();
        let template = confluence
            .get_template(&args.template_id)
            .await
            .map_err(to_mcp_error)?;
        let body = template
            .body
            .as_ref()
            .and_then(|b| b.storage.as_ref())
            .map(|s| s.value.clone())
            .unwrap_or_default();
        let page = confluence
            .create_page(
                &args.space_key,
                &args.title,
                // The template body is already storage format.
                &to_storage(&body, Some("storage"))?,
                args.parent_id.as_deref(),
            )
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(page))
    }

    #[tool(
        description = "Show who may read and edit a Confluence page.",
        annotations(read_only_hint = true)
    )]
    async fn confluence_get_page_restrictions(
        &self,
        Parameters(args): Parameters<GetRestrictionsArgs>,
    ) -> Result<Json<Restrictions>, McpError> {
        let restrictions = self
            .client()
            .get_restrictions(&args.page_id)
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(restrictions))
    }

    #[tool(
        description = "Replace the read/edit restrictions of a Confluence page. This REPLACES the current lists, it does not add to them — read the current state with confluence_get_page_restrictions first, and confirm with the user, since over-restricting can lock people out.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn confluence_set_page_restrictions(
        &self,
        Parameters(args): Parameters<SetRestrictionsArgs>,
    ) -> Result<Json<Restrictions>, McpError> {
        let restrictions = self
            .client()
            .set_restrictions(
                &args.page_id,
                &args.read_users.unwrap_or_default(),
                &args.update_users.unwrap_or_default(),
                &args.read_groups.unwrap_or_default(),
                &args.update_groups.unwrap_or_default(),
            )
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(restrictions))
    }

    #[tool(
        description = "Find Confluence users by name. Use it to resolve a person into the identifier that page restrictions need.",
        annotations(read_only_hint = true)
    )]
    async fn confluence_search_users(
        &self,
        Parameters(args): Parameters<SearchUsersArgs>,
    ) -> Result<Json<ListResult<Person>>, McpError> {
        let users = self
            .client()
            .search_users(&args.query, page_size(args.limit, 10))
            .await
            .map_err(to_mcp_error)?;
        list_result(users)
    }
}
