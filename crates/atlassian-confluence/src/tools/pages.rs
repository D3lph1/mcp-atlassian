//! Page CRUD and hierarchy. Content crosses this boundary as Markdown by
//! default; `content_format: "storage"` passes raw XHTML through (D10).

use crate::ConfluenceTools;
use crate::{Content, ResultsPage};
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use super::storage::{page_to_markdown_view, to_storage, PageNode, PageView};
use atlassian_client::mcp::{list_result, status_result, to_mcp_error, ListResult, StatusResult};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConfluenceGetPageArgs {
    /// Numeric page id, e.g. `123456` (find via confluence_search).
    pub page_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConfluenceGetPageChildrenArgs {
    /// Numeric page id.
    pub page_id: String,
    /// Max children to return (default 25).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConfluenceCreatePageArgs {
    /// Space key, e.g. `DEV` (see confluence_get_spaces).
    pub space_key: String,
    pub title: String,
    /// Page content, Markdown by default.
    pub content: String,
    /// `markdown` (default) or `storage` (raw Confluence storage XHTML).
    pub content_format: Option<String>,
    /// Optional parent page id — page is created as its child.
    pub parent_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConfluenceUpdatePageArgs {
    /// Numeric page id.
    pub page_id: String,
    /// New title; omit to keep the current one.
    pub title: Option<String>,
    /// New content (replaces the whole body); omit to keep the current one.
    pub content: Option<String>,
    /// `markdown` (default) or `storage` (raw Confluence storage XHTML).
    pub content_format: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConfluenceDeletePageArgs {
    /// Numeric page id.
    pub page_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MovePageArgs {
    /// Numeric page id to move.
    pub page_id: String,
    /// New parent page id. Omit to keep the current parent.
    pub target_parent_id: Option<String>,
    /// New space key. Omit to keep the current space.
    pub target_space_key: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SpacePageTreeArgs {
    /// Space key, e.g. `DEV`.
    pub space_key: String,
    /// Max pages to walk (default 100).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateSectionArgs {
    /// Numeric page id.
    pub page_id: String,
    /// Exact heading text whose section should be replaced, e.g. `Rollback`.
    pub heading_text: String,
    /// Replacement content for that section, Markdown by default.
    pub new_content: String,
    /// `markdown` (default) or `storage`.
    pub content_format: Option<String>,
}

#[tool_router(router = confluence_pages_router, vis = "pub(crate)")]
impl ConfluenceTools {
    #[tool(
        description = "Get a Confluence page by id, with its body converted to Markdown. Find page ids via confluence_search.",
        annotations(read_only_hint = true)
    )]
    async fn confluence_get_page(
        &self,
        Parameters(args): Parameters<ConfluenceGetPageArgs>,
    ) -> Result<Json<PageView>, McpError> {
        let page = self
            .client()
            .get_page(&args.page_id)
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(page_to_markdown_view(&page)))
    }

    #[tool(
        description = "List direct child pages of a Confluence page.",
        annotations(read_only_hint = true)
    )]
    async fn confluence_get_page_children(
        &self,
        Parameters(args): Parameters<ConfluenceGetPageChildrenArgs>,
    ) -> Result<Json<ResultsPage<Content>>, McpError> {
        let children = self
            .client()
            .get_page_children(&args.page_id, args.limit.unwrap_or(25))
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(children))
    }

    #[tool(
        description = "Create a Confluence page. Content is Markdown by default (converted to Confluence storage format); pass content_format=\"storage\" to send raw storage XHTML.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn confluence_create_page(
        &self,
        Parameters(args): Parameters<ConfluenceCreatePageArgs>,
    ) -> Result<Json<Content>, McpError> {
        let storage = to_storage(&args.content, args.content_format.as_deref())?;
        let page = self
            .client()
            .create_page(
                &args.space_key,
                &args.title,
                &storage,
                args.parent_id.as_deref(),
            )
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(page))
    }

    #[tool(
        description = "Update a Confluence page's title and/or content. Content is Markdown by default; pass content_format=\"storage\" for raw storage XHTML. Omitted parts stay unchanged. Note: Markdown content REPLACES the whole page body.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn confluence_update_page(
        &self,
        Parameters(args): Parameters<ConfluenceUpdatePageArgs>,
    ) -> Result<Json<PageView>, McpError> {
        let storage = args
            .content
            .as_deref()
            .map(|c| to_storage(c, args.content_format.as_deref()))
            .transpose()?;
        let page = self
            .client()
            .update_page(&args.page_id, args.title.as_deref(), storage.as_deref())
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(page_to_markdown_view(&page)))
    }

    #[tool(
        description = "Delete a Confluence page permanently. This cannot be undone — confirm with the user before calling.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn confluence_delete_page(
        &self,
        Parameters(args): Parameters<ConfluenceDeletePageArgs>,
    ) -> Result<Json<StatusResult>, McpError> {
        self.client()
            .delete_page(&args.page_id)
            .await
            .map_err(to_mcp_error)?;
        status_result(format!("Page {} deleted", args.page_id))
    }

    #[tool(
        description = "Move a Confluence page under a different parent and/or into another space. Content and title are preserved.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn confluence_move_page(
        &self,
        Parameters(args): Parameters<MovePageArgs>,
    ) -> Result<Json<Content>, McpError> {
        if args.target_parent_id.is_none() && args.target_space_key.is_none() {
            return Err(McpError::invalid_params(
                "specify target_parent_id and/or target_space_key — nothing to move otherwise",
                None,
            ));
        }
        let page = self
            .client()
            .move_page(
                &args.page_id,
                args.target_parent_id.as_deref(),
                args.target_space_key.as_deref(),
            )
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(page))
    }

    #[tool(
        description = "Get the page hierarchy of a Confluence space as a tree of ids and titles. Use it to understand how a space is organized before reading individual pages.",
        annotations(read_only_hint = true)
    )]
    async fn confluence_get_space_page_tree(
        &self,
        Parameters(args): Parameters<SpacePageTreeArgs>,
    ) -> Result<Json<ListResult<PageNode>>, McpError> {
        let pages = self
            .client()
            .get_space_pages(&args.space_key, args.limit.unwrap_or(100))
            .await
            .map_err(to_mcp_error)?;
        list_result(build_page_tree(&pages.results))
    }

    #[tool(
        description = "Replace one section of a Confluence page, identified by its heading text, leaving the rest of the page untouched. Prefer this over confluence_update_page when editing part of a long document.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn confluence_update_page_section(
        &self,
        Parameters(args): Parameters<UpdateSectionArgs>,
    ) -> Result<Json<PageView>, McpError> {
        let confluence = self.client();
        let page = confluence
            .get_page(&args.page_id)
            .await
            .map_err(to_mcp_error)?;
        let current = page
            .body
            .as_ref()
            .and_then(|b| b.storage.as_ref())
            .map(|s| storage_markdown::storage_to_markdown(&s.value))
            .unwrap_or_default();
        let replacement = match args.content_format.as_deref().unwrap_or("markdown") {
            "markdown" => args.new_content.clone(),
            "storage" => storage_markdown::storage_to_markdown(&args.new_content),
            other => {
                return Err(McpError::invalid_params(
                    format!("unknown content_format `{other}`: use `markdown` or `storage`"),
                    None,
                ))
            }
        };
        let updated =
            replace_section(&current, &args.heading_text, &replacement).ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                    "no heading `{}` on page {} — read the page first to get its exact headings",
                    args.heading_text, args.page_id
                ),
                    None,
                )
            })?;
        let storage = storage_markdown::markdown_to_storage(&updated);
        let page = confluence
            .update_page(&args.page_id, None, Some(&storage))
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(page_to_markdown_view(&page)))
    }
}

/// Nests pages by their `ancestors` chain into a tree. Pages whose parent is
/// outside the fetched set become roots.
fn build_page_tree(pages: &[Content]) -> Vec<PageNode> {
    use std::collections::{HashMap, HashSet};

    let ids: HashSet<&str> = pages.iter().map(|p| p.id.as_str()).collect();
    let mut children: HashMap<&str, Vec<&Content>> = HashMap::new();
    let mut roots: Vec<&Content> = Vec::new();
    for page in pages {
        match page.ancestors.last() {
            Some(parent) if ids.contains(parent.id.as_str()) => {
                children.entry(parent.id.as_str()).or_default().push(page)
            }
            _ => roots.push(page),
        }
    }

    fn node(page: &Content, children: &std::collections::HashMap<&str, Vec<&Content>>) -> PageNode {
        PageNode {
            id: page.id.clone(),
            title: page.title.clone(),
            children: children
                .get(page.id.as_str())
                .map(|c| c.iter().map(|child| node(child, children)).collect())
                .unwrap_or_default(),
        }
    }

    roots.iter().map(|p| node(p, &children)).collect()
}

/// Replaces the body of the Markdown section introduced by `heading` — from
/// that heading up to the next heading of the same or higher level.
fn replace_section(markdown: &str, heading: &str, replacement: &str) -> Option<String> {
    let lines: Vec<&str> = markdown.lines().collect();
    let heading_level = |line: &str| -> Option<usize> {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        (hashes > 0 && line.chars().nth(hashes) == Some(' ')).then_some(hashes)
    };
    let start = lines.iter().position(|line| {
        heading_level(line).is_some() && line.trim_start_matches('#').trim() == heading.trim()
    })?;
    let level = heading_level(lines[start])?;
    let end = lines[start + 1..]
        .iter()
        .position(|line| heading_level(line).is_some_and(|l| l <= level))
        .map(|offset| start + 1 + offset)
        .unwrap_or(lines.len());

    let mut out: Vec<String> = lines[..=start].iter().map(|s| s.to_string()).collect();
    out.push(String::new());
    out.push(replacement.trim_end().to_string());
    if end < lines.len() {
        out.push(String::new());
        out.extend(lines[end..].iter().map(|s| s.to_string()));
    }
    Some(out.join("\n"))
}
