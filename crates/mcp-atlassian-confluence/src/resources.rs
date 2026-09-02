//! Confluence pages as MCP resources: `confluence://123456`.
//!
//! The body crosses the boundary as Markdown, like everywhere else in this
//! crate (D10) — raw storage XHTML would burn the client's context.

use rmcp::model::{ResourceContents, ResourceTemplate};
use rmcp::ErrorData as McpError;

use mcp_atlassian_client::mcp::to_mcp_error;

use crate::tools::storage::page_to_markdown_view;
use crate::ConfluenceTools;

/// URI prefix this product answers for.
pub const URI_PREFIX: &str = "confluence://";

/// Comments carried by `confluence://ID/comments`.
const COMMENTS: u32 = 25;

/// The URI templates Confluence contributes to `resources/templates/list`.
pub fn templates() -> Vec<ResourceTemplate> {
    vec![
        ResourceTemplate::new("confluence://{page_id}", "confluence-page")
            .with_title("Confluence page")
            .with_description(
                "A Confluence page as Markdown, titled and converted from storage format. \
             The page id is numeric and appears in the page URL, e.g. `confluence://123456` \
             (find ids with confluence_search).",
            )
            .with_mime_type("text/markdown"),
        ResourceTemplate::new(
            "confluence://{page_id}/comments",
            "confluence-page-comments",
        )
        .with_title("Confluence page comments")
        .with_description(
            "The comments of a Confluence page as Markdown, newest last, e.g. \
                 `confluence://123456/comments`.",
        )
        .with_mime_type("text/markdown"),
    ]
}

/// What a `confluence://` URI names (D44).
enum Resource<'a> {
    Page(&'a str),
    Comments(&'a str),
}

impl ConfluenceTools {
    /// Reads `confluence://{page_id}` or `confluence://{page_id}/comments`.
    pub async fn read_resource(&self, uri: &str) -> Result<Vec<ResourceContents>, McpError> {
        let text = match resource(uri)? {
            Resource::Page(page_id) => {
                let page = self
                    .client()
                    .get_page(page_id)
                    .await
                    .map_err(to_mcp_error)?;
                let view = page_to_markdown_view(&page);
                // The title is metadata in `PageView`, but a resource carries
                // only its contents — so it becomes the document's heading.
                let mut text = format!("# {}\n", view.title);
                if let Some(body) = view.body_markdown {
                    text.push('\n');
                    text.push_str(&body);
                }
                text
            }
            Resource::Comments(page_id) => {
                let comments = self
                    .client()
                    .get_comments(page_id, COMMENTS, 0)
                    .await
                    .map_err(to_mcp_error)?;
                let mut text = format!("# Comments on page {page_id}\n");
                for comment in &comments.results {
                    let view = page_to_markdown_view(comment);
                    text.push_str(&format!("\n## Comment {}\n\n", view.id));
                    text.push_str(view.body_markdown.as_deref().unwrap_or("(empty)").trim());
                    text.push('\n');
                }
                if comments.results.is_empty() {
                    text.push_str("\nNo comments.\n");
                }
                text
            }
        };
        Ok(vec![ResourceContents::TextResourceContents {
            uri: uri.to_string(),
            mime_type: Some("text/markdown".into()),
            text,
            meta: None,
        }])
    }
}

/// Pulls the page id — and the optional `/comments` — out of the URI. Not
/// `Url::parse` — see the note in the Jira counterpart; the shapes are kept
/// identical on purpose.
fn resource(uri: &str) -> Result<Resource<'_>, McpError> {
    let malformed = || {
        McpError::invalid_params(
            format!(
                "`{uri}` is not a Confluence resource: expected `confluence://PAGE_ID` or \
                 `confluence://PAGE_ID/comments`, e.g. `confluence://123456`"
            ),
            None,
        )
    };
    let rest = uri
        .strip_prefix(URI_PREFIX)
        .unwrap_or_default()
        .trim_end_matches('/');
    let (page_id, sub) = match rest.split_once('/') {
        Some((id, sub)) => (id, Some(sub)),
        None => (rest, None),
    };
    if page_id.is_empty() || page_id.contains(['/', '?', '#']) {
        return Err(malformed());
    }
    match sub {
        None => Ok(Resource::Page(page_id)),
        Some("comments") => Ok(Resource::Comments(page_id)),
        Some(_) => Err(malformed()),
    }
}

#[cfg(test)]
fn page_id(uri: &str) -> Result<&str, McpError> {
    match resource(uri)? {
        Resource::Page(id) | Resource::Comments(id) => Ok(id),
    }
}

#[cfg(test)]
mod tests {
    use super::page_id;

    #[test]
    fn accepts_a_bare_page_id() {
        assert_eq!(page_id("confluence://123456").unwrap(), "123456");
        assert_eq!(page_id("confluence://123456/").unwrap(), "123456");
    }

    #[test]
    fn rejects_anything_else() {
        assert_eq!(page_id("confluence://123456/comments").unwrap(), "123456");
        for uri in [
            "confluence://",
            "confluence://123456/children",
            "confluence://123456?version=2",
            "jira://PROJ-1",
        ] {
            let error = page_id(uri).unwrap_err().message.to_string();
            assert!(error.contains("confluence://123456"), "{uri}: {error}");
        }
    }
}
