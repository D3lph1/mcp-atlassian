//! Jira issues as MCP resources: `jira://PROJ-123`.
//!
//! Lives in the product crate for the same reason tools do (D15): the URI
//! shape, the field selection and the parsing of an issue key are Jira
//! knowledge, and the server crate holds none.

use rmcp::model::{ResourceContents, ResourceTemplate};
use rmcp::ErrorData as McpError;

use atlassian_client::mcp::to_mcp_error;

use crate::JiraTools;

/// URI prefix this product answers for.
pub const URI_PREFIX: &str = "jira://";

/// Exactly the fields `IssueFields` models (D4). Naming them keeps the
/// payload small — omitting `fields` would return every custom field the
/// instance has.
const RESOURCE_FIELDS: &str =
    "summary,description,status,priority,issuetype,assignee,reporter,labels,created,updated";

/// The URI templates Jira contributes to `resources/templates/list`.
pub fn templates() -> Vec<ResourceTemplate> {
    vec![ResourceTemplate::new("jira://{issue_key}", "jira-issue")
        .with_title("Jira issue")
        .with_description(
            "A Jira issue as JSON: summary, description, status, assignee, labels and dates. \
             The issue key is the one Jira displays, e.g. `jira://PROJ-123`.",
        )
        .with_mime_type("application/json")]
}

impl JiraTools {
    /// Reads `jira://{issue_key}`.
    pub async fn read_resource(&self, uri: &str) -> Result<Vec<ResourceContents>, McpError> {
        let key = issue_key(uri)?;
        let issue = self
            .client()
            .get_issue(key, Some(RESOURCE_FIELDS))
            .await
            .map_err(to_mcp_error)?;
        let text = serde_json::to_string_pretty(&issue).map_err(|e| {
            McpError::internal_error(format!("failed to encode issue {key}: {e}"), None)
        })?;
        Ok(vec![ResourceContents::TextResourceContents {
            uri: uri.to_string(),
            mime_type: Some("application/json".into()),
            text,
            meta: None,
        }])
    }
}

/// Pulls the issue key out of the URI.
///
/// Deliberately not `Url::parse`: it treats everything before the first slash
/// as a host and lowercases it, so `jira://PROJ-123` would come back as
/// `proj-123` — a key Jira does not have.
fn issue_key(uri: &str) -> Result<&str, McpError> {
    let key = uri
        .strip_prefix(URI_PREFIX)
        .unwrap_or_default()
        .trim_end_matches('/');
    if key.is_empty() || key.contains(['/', '?', '#']) {
        return Err(McpError::invalid_params(
            format!(
                "`{uri}` is not a Jira issue resource: expected `jira://ISSUE-KEY`, \
                 e.g. `jira://PROJ-123`"
            ),
            None,
        ));
    }
    Ok(key)
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
    fn rejects_anything_that_is_not_a_bare_key() {
        for uri in [
            "jira://",
            "jira://PROJ-123/comments",
            "jira://PROJ-123?expand=changelog",
            "confluence://123",
        ] {
            let error = issue_key(uri).unwrap_err().message.to_string();
            assert!(error.contains("jira://PROJ-123"), "{uri}: {error}");
        }
    }
}
