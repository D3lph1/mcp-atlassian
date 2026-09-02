//! Confluence prompts (D30): `/confluence_page 123456` briefs on a page.
//!
//! Like the Jira prompts, this fetches its own data — the page body as
//! Markdown and its newest comments — and ends with the ask.

use rmcp::{
    handler::server::{router::prompt::PromptRouter, wrapper::Parameters},
    model::{PromptMessage, Role},
    prompt, prompt_router, schemars, ErrorData as McpError,
};
use serde::Deserialize;

use atlassian_client::mcp::to_mcp_error;

use crate::tools::storage::{page_to_markdown_view, PageView};
use crate::ConfluenceTools;

/// Newest comments pulled into the briefing.
const COMMENTS: u32 = 5;
/// Character budgets; the model can read the full page as a resource.
const MAX_BODY: usize = 6000;
const MAX_COMMENT: usize = 600;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PageArgs {
    /// Numeric page id, e.g. `123456` (find it with confluence_search).
    pub page_id: String,
}

#[prompt_router(router = "confluence_prompts_router", vis = "pub(crate)")]
impl ConfluenceTools {
    /// Brief on one Confluence page: its content and newest comments, then
    /// the ask. Use when someone points at a page and wants the gist, the
    /// open questions, or the actions in it.
    #[prompt(name = "confluence_page")]
    pub async fn confluence_page_prompt(
        &self,
        Parameters(args): Parameters<PageArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        let page_id = args.page_id.trim();
        let page = self
            .client()
            .get_page(page_id)
            .await
            .map_err(to_mcp_error)?;
        // A page without comments is normal; a failure here must not lose
        // the briefing that already succeeded.
        let comments = match self.client().get_comments(page_id, COMMENTS, 0).await {
            Ok(page) => page.results.iter().map(page_to_markdown_view).collect(),
            Err(_) => Vec::new(),
        };
        Ok(vec![PromptMessage::new_text(
            Role::User,
            brief(&page_to_markdown_view(&page), &comments),
        )])
    }
}

/// All Confluence prompt routes.
pub fn router() -> PromptRouter<ConfluenceTools> {
    ConfluenceTools::confluence_prompts_router()
}

fn brief(page: &PageView, comments: &[PageView]) -> String {
    let mut out = format!("Confluence page {} — {}\n", page.id, page.title);
    if let Some(space) = &page.space {
        out.push_str(&format!("Space: {} ({})\n", space.name, space.key));
    }
    if let Some(version) = &page.version {
        out.push_str(&format!("Version: {}\n", version.number));
    }
    out.push_str("\nContent\n");
    match page.body_markdown.as_deref().map(str::trim) {
        Some(body) if !body.is_empty() => {
            out.push_str(&truncate(body, MAX_BODY));
            out.push('\n');
        }
        _ => out.push_str("(empty)\n"),
    }
    if comments.is_empty() {
        out.push_str("\nNo comments.\n");
    } else {
        out.push_str(&format!("\nNewest {} comment(s)\n", comments.len()));
        for comment in comments {
            out.push_str(&format!(
                "\n[comment {}]\n{}\n",
                comment.id,
                truncate(
                    comment.body_markdown.as_deref().unwrap_or("").trim(),
                    MAX_COMMENT
                )
            ));
        }
    }
    out.push_str(
        "\n---\n\
         Work this page:\n\
         1. Summarize it in three sentences for someone who will not read it.\n\
         2. List the open questions and the action items it contains, with owners \
         where named.\n\
         3. Say what is out of date or contradicts what you know, if anything.\n\n\
         Do not edit, move or comment on the page unless asked to.\n",
    );
    out
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!(
        "{kept}\n… truncated, {max} of {} characters; read `confluence://ID` for all of it",
        text.chars().count()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Space, Version};

    fn view(id: &str, title: &str, body: Option<&str>) -> PageView {
        PageView {
            id: id.into(),
            content_type: "page".into(),
            title: title.into(),
            space: Some(Space {
                key: "DEV".into(),
                name: "Development".into(),
            }),
            version: Some(Version { number: 4 }),
            body_markdown: body.map(String::from),
        }
    }

    #[test]
    fn the_briefing_carries_the_page_and_its_comments_then_the_ask() {
        let text = brief(
            &view("123", "Runbook", Some("## Deploy\n\nrun make")),
            &[view("9", "", Some("Looks stale"))],
        );
        for expected in [
            "Runbook",
            "Development (DEV)",
            "Version: 4",
            "run make",
            "Looks stale",
            "Work this page",
            "Do not edit",
        ] {
            assert!(text.contains(expected), "missing {expected:?}:\n{text}");
        }
        let empty = brief(&view("1", "Empty", None), &[]);
        assert!(
            empty.contains("(empty)") && empty.contains("No comments."),
            "{empty}"
        );
    }

    #[test]
    fn a_long_body_is_cut_and_points_at_the_resource() {
        let text = brief(&view("1", "Long", Some(&"x".repeat(MAX_BODY + 10))), &[]);
        assert!(text.contains("… truncated"), "{text}");
        assert!(text.contains("confluence://"), "{text}");
    }
}
