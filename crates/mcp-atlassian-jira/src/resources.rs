//! Jira issues as MCP resources: `jira://PROJ-123`.
//!
//! Lives in the product crate for the same reason tools do (D15): the URI
//! shape, the field selection and the parsing of an issue key are Jira
//! knowledge, and the server crate holds none.

use rmcp::model::{ResourceContents, ResourceTemplate};
use rmcp::ErrorData as McpError;

use mcp_atlassian_client::mcp::to_mcp_error;

use crate::JiraTools;

/// URI prefix this product answers for.
pub const URI_PREFIX: &str = "jira://";

/// Comments carried by `jira://KEY/comments`.
const COMMENTS: u32 = 25;

/// The URI templates Jira contributes to `resources/templates/list`.
pub fn templates() -> Vec<ResourceTemplate> {
    vec![
        ResourceTemplate::new("jira://{issue_key}", "jira-issue")
            .with_title("Jira issue")
            .with_description(
                "A Jira issue as JSON: summary, description, status, assignee, labels, dates, \
                 parent, subtasks and links. The issue key is the one Jira displays, e.g. \
                 `jira://PROJ-123`.",
            )
            .with_mime_type("application/json"),
        ResourceTemplate::new("jira://{issue_key}/comments", "jira-issue-comments")
            .with_title("Jira issue comments")
            .with_description(
                "The newest comments of a Jira issue as JSON, e.g. `jira://PROJ-123/comments`.",
            )
            .with_mime_type("application/json"),
    ]
}

/// What a `jira://` URI names (D44).
enum Resource<'a> {
    Issue(&'a str),
    Comments(&'a str),
}

impl JiraTools {
    /// Reads `jira://{issue_key}` or `jira://{issue_key}/comments`.
    pub async fn read_resource(&self, uri: &str) -> Result<Vec<ResourceContents>, McpError> {
        let text = match resource(uri)? {
            Resource::Issue(key) => {
                let issue = self
                    .client()
                    .get_issue(key, None)
                    .await
                    .map_err(to_mcp_error)?;
                encode(key, &issue)?
            }
            Resource::Comments(key) => {
                let page = self
                    .client()
                    .get_comments(key, COMMENTS, 0)
                    .await
                    .map_err(to_mcp_error)?;
                encode(key, &page)?
            }
        };
        Ok(vec![ResourceContents::TextResourceContents {
            uri: uri.to_string(),
            mime_type: Some("application/json".into()),
            text,
            meta: None,
        }])
    }

    /// Completes the `issue_key` argument of prompts and resource templates
    /// (D44): project keys until the first `-`, then the project's most
    /// recently updated issues.
    pub async fn complete_issue_key(&self, partial: &str) -> Vec<String> {
        let partial = partial.trim();
        let Some((project, _)) = partial.split_once('-') else {
            let upper = partial.to_ascii_uppercase();
            return match self.client().get_projects().await {
                Ok(projects) => projects
                    .into_iter()
                    .filter(|p| p.key.starts_with(&upper))
                    .map(|p| format!("{}-", p.key))
                    .collect(),
                Err(error) => {
                    tracing::debug!(%error, "project list unavailable for completion");
                    Vec::new()
                }
            };
        };
        let upper = partial.to_ascii_uppercase();
        match self
            .client()
            .get_project_issues(&project.to_ascii_uppercase(), 25)
            .await
        {
            Ok(page) => page
                .issues
                .into_iter()
                .map(|issue| issue.key)
                .filter(|key| key.starts_with(&upper))
                .collect(),
            Err(error) => {
                tracing::debug!(%error, "issue list unavailable for completion");
                Vec::new()
            }
        }
    }
}

fn encode<T: serde::Serialize>(key: &str, value: &T) -> Result<String, McpError> {
    serde_json::to_string_pretty(value)
        .map_err(|e| McpError::internal_error(format!("failed to encode {key}: {e}"), None))
}

/// Pulls the issue key — and the optional `/comments` — out of the URI.
///
/// Deliberately not `Url::parse`: it treats everything before the first slash
/// as a host and lowercases it, so `jira://PROJ-123` would come back as
/// `proj-123` — a key Jira does not have.
fn resource(uri: &str) -> Result<Resource<'_>, McpError> {
    let malformed = || {
        McpError::invalid_params(
            format!(
                "`{uri}` is not a Jira resource: expected `jira://ISSUE-KEY` or \
                 `jira://ISSUE-KEY/comments`, e.g. `jira://PROJ-123`"
            ),
            None,
        )
    };
    let rest = uri
        .strip_prefix(URI_PREFIX)
        .unwrap_or_default()
        .trim_end_matches('/');
    let (key, sub) = match rest.split_once('/') {
        Some((key, sub)) => (key, Some(sub)),
        None => (rest, None),
    };
    if key.is_empty() || key.contains(['/', '?', '#']) {
        return Err(malformed());
    }
    match sub {
        None => Ok(Resource::Issue(key)),
        Some("comments") => Ok(Resource::Comments(key)),
        Some(_) => Err(malformed()),
    }
}

#[cfg(test)]
fn issue_key(uri: &str) -> Result<&str, McpError> {
    match resource(uri)? {
        Resource::Issue(key) | Resource::Comments(key) => Ok(key),
    }
}

#[cfg(test)]
mod tests {
    use super::issue_key;

    #[test]
    fn accepts_an_issue_key_and_keeps_its_case() {
        assert_eq!(issue_key("jira://PROJ-123").unwrap(), "PROJ-123");
        assert_eq!(issue_key("jira://PROJ-123/").unwrap(), "PROJ-123");
    }

    #[test]
    fn accepts_the_comments_sub_resource() {
        assert_eq!(issue_key("jira://PROJ-123/comments").unwrap(), "PROJ-123");
        assert!(matches!(
            super::resource("jira://PROJ-123/comments").unwrap(),
            super::Resource::Comments("PROJ-123")
        ));
    }

    #[test]
    fn rejects_anything_that_is_not_a_bare_key() {
        for uri in [
            "jira://",
            "jira://PROJ-123/watchers",
            "jira://PROJ-123?expand=changelog",
            "confluence://123",
        ] {
            let error = issue_key(uri).unwrap_err().message.to_string();
            assert!(error.contains("jira://PROJ-123"), "{uri}: {error}");
        }
    }
}
