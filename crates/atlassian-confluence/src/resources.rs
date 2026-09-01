//! Confluence pages as MCP resources: `confluence://123456`.
//!
//! The body crosses the boundary as Markdown, like everywhere else in this
//! crate (D10) — raw storage XHTML would burn the client's context.

use rmcp::model::{ResourceContents, ResourceTemplate};
use rmcp::ErrorData as McpError;

use atlassian_client::mcp::to_mcp_error;

use crate::tools::storage::page_to_markdown_view;
use crate::ConfluenceTools;

/// URI prefix this product answers for.
pub const URI_PREFIX: &str = "confluence://";

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
    ]
}

impl ConfluenceTools {
    /// Reads `confluence://{page_id}`.
    pub async fn read_resource(&self, uri: &str) -> Result<Vec<ResourceContents>, McpError> {
        let page_id = page_id(uri)?;
        let page = self
            .client()
            .get_page(page_id)
            .await
            .map_err(to_mcp_error)?;
        let view = page_to_markdown_view(&page);
        // The title is metadata in `PageView`, but a resource carries only its
        // contents — so it becomes the document's heading.
        let mut text = format!("# {}\n", view.title);
        if let Some(body) = view.body_markdown {
            text.push('\n');
            text.push_str(&body);
        }
        Ok(vec![ResourceContents::TextResourceContents {
            uri: uri.to_string(),
            mime_type: Some("text/markdown".into()),
            text,
            meta: None,
        }])
    }
}

/// Pulls the page id out of the URI. Not `Url::parse` — see the note in the
/// Jira counterpart; the shapes are kept identical on purpose.
fn page_id(uri: &str) -> Result<&str, McpError> {
    let page_id = uri
        .strip_prefix(URI_PREFIX)
        .unwrap_or_default()
        .trim_end_matches('/');
    if page_id.is_empty() || page_id.contains(['/', '?', '#']) {
        return Err(McpError::invalid_params(
            format!(
                "`{uri}` is not a Confluence page resource: expected `confluence://PAGE_ID`, \
                 e.g. `confluence://123456`"
            ),
            None,
        ));
    }
    Ok(page_id)
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
