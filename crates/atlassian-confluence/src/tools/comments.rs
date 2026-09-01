//! Page comments. Bodies cross this boundary as Markdown (D10).

use crate::ConfluenceTools;
use crate::Content;
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use super::storage::{page_to_markdown_view, to_storage, PageView};
use atlassian_client::mcp::{list_result, to_mcp_error, ListResult};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConfluenceAddCommentArgs {
    /// Numeric page id.
    pub page_id: String,
    /// Comment content, Markdown by default.
    pub content: String,
    /// `markdown` (default) or `storage` (raw Confluence storage XHTML).
    pub content_format: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConfluenceGetCommentsArgs {
    /// Numeric page id.
    pub page_id: String,
    /// Max comments to return (default 10).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReplyToCommentArgs {
    /// Comment id to reply to (from confluence_get_comments).
    pub comment_id: String,
    /// Reply content, Markdown by default.
    pub content: String,
    /// `markdown` (default) or `storage`.
    pub content_format: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetInlineCommentsArgs {
    /// Numeric page id.
    pub page_id: String,
    /// Max comments to return (default 25).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddInlineCommentArgs {
    /// Numeric page id.
    pub page_id: String,
    /// Comment content, Markdown by default.
    pub content: String,
    /// Exact text on the page the comment anchors to.
    pub text_selection: String,
    /// `markdown` (default) or `storage`.
    pub content_format: Option<String>,
}

#[tool_router(router = confluence_comments_router, vis = "pub(crate)")]
impl ConfluenceTools {
    #[tool(
        description = "Add a comment to a Confluence page. Content is Markdown by default; pass content_format=\"storage\" for raw storage XHTML.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn confluence_add_comment(
        &self,
        Parameters(args): Parameters<ConfluenceAddCommentArgs>,
    ) -> Result<Json<Content>, McpError> {
        let storage = to_storage(&args.content, args.content_format.as_deref())?;
        let comment = self
            .client()
            .add_comment(&args.page_id, &storage)
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(comment))
    }

    #[tool(
        description = "Get comments of a Confluence page, bodies converted to Markdown.",
        annotations(read_only_hint = true)
    )]
    async fn confluence_get_comments(
        &self,
        Parameters(args): Parameters<ConfluenceGetCommentsArgs>,
    ) -> Result<Json<ListResult<PageView>>, McpError> {
        let comments = self
            .client()
            .get_comments(&args.page_id, args.limit.unwrap_or(10))
            .await
            .map_err(to_mcp_error)?;
        let views: Vec<PageView> = comments.results.iter().map(page_to_markdown_view).collect();
        list_result(views)
    }

    #[tool(
        description = "Reply to an existing Confluence comment, keeping the thread together.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn confluence_reply_to_comment(
        &self,
        Parameters(args): Parameters<ReplyToCommentArgs>,
    ) -> Result<Json<Content>, McpError> {
        let storage = to_storage(&args.content, args.content_format.as_deref())?;
        let reply = self
            .client()
            .reply_to_comment(&args.comment_id, &storage)
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(reply))
    }

    #[tool(
        description = "Get inline comments of a Confluence page — the ones anchored to specific text rather than to the page as a whole.",
        annotations(read_only_hint = true)
    )]
    async fn confluence_get_inline_comments(
        &self,
        Parameters(args): Parameters<GetInlineCommentsArgs>,
    ) -> Result<Json<ListResult<PageView>>, McpError> {
        let comments = self
            .client()
            .get_inline_comments(&args.page_id, args.limit.unwrap_or(25))
            .await
            .map_err(to_mcp_error)?;
        let views: Vec<PageView> = comments.results.iter().map(page_to_markdown_view).collect();
        list_result(views)
    }

    #[tool(
        description = "Add a comment anchored to a specific passage of a Confluence page. text_selection must match the page text exactly.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn confluence_add_inline_comment(
        &self,
        Parameters(args): Parameters<AddInlineCommentArgs>,
    ) -> Result<Json<Content>, McpError> {
        let storage = to_storage(&args.content, args.content_format.as_deref())?;
        let comment = self
            .client()
            .add_inline_comment(&args.page_id, &storage, &args.text_selection)
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(comment))
    }
}
