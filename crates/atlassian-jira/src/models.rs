use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The authenticated user, from `GET /rest/api/2/myself`.
///
/// Cloud identifies users by `accountId`, Server/DC by `name` — both are
/// optional so one model covers both deployments (D5).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Myself {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    #[serde(default)]
    pub active: bool,
}

/// A user reference inside issue fields (assignee, reporter, comment author).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct User {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub display_name: String,
    /// Often hidden by the instance's privacy settings — absent, not empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
}

/// A named entity (status, priority, issue type) — we only ever need `name`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Named {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub key: String,
    pub id: String,
    pub fields: IssueFields,
}

/// The subset of issue fields we expose to the LLM. Everything is optional —
/// search responses only carry the fields that were requested (D4).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IssueFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Plain text / wiki markup on API v2 (D5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Named>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Named>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuetype: Option<Named>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<User>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reporter: Option<User>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
}

/// Unified search page over the Cloud (`/search/jql`, token-paginated) and
/// Server/DC (`/search`, offset-paginated) response shapes.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub issues: Vec<Issue>,
    /// Server/DC only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// Cloud only: pass back to continue pagination.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreatedIssue {
    pub id: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Transition {
    pub id: String,
    pub name: String,
    /// Target status.
    pub to: Named,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TransitionsResponse {
    pub transitions: Vec<Transition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<User>,
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CommentPage {
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Worklog {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_spent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Project {
    pub id: String,
    pub key: String,
    pub name: String,
}

/// Cloud `GET /project/search` paginated envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct ProjectPage {
    pub values: Vec<Project>,
}

/// An issue attachment, from the `attachment` field of an issue.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub filename: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Absolute URL of the attachment binary.
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<User>,
}

/// Envelope for reading just the attachments of an issue.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct IssueAttachments {
    pub fields: AttachmentFields,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AttachmentFields {
    #[serde(default)]
    pub attachment: Vec<Attachment>,
}

/// Agile board, from `GET /rest/agile/1.0/board`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub id: u64,
    pub name: String,
    /// `scrum` or `kanban`.
    #[serde(rename = "type", default)]
    pub board_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Sprint {
    pub id: u64,
    pub name: String,
    /// `future`, `active` or `closed`.
    #[serde(default)]
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
}

/// Agile API paginated envelope: `{values, isLast, startAt, maxResults}`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgilePage<T> {
    pub values: Vec<T>,
    #[serde(default)]
    pub is_last: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct IssueType {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub subtask: bool,
}

/// An issue link type, e.g. `Blocks` with inward "is blocked by" and outward
/// "blocks" phrasing.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LinkType {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub inward: String,
    #[serde(default)]
    pub outward: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LinkTypesResponse {
    #[serde(rename = "issueLinkTypes")]
    pub issue_link_types: Vec<LinkType>,
}

/// A field definition from `GET /rest/api/2/field`. Custom fields carry a
/// `customfield_*` id, which is what update payloads need.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Field {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub custom: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<FieldSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FieldSchema {
    #[serde(rename = "type", default)]
    pub field_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<String>,
}

/// One selectable option of a custom field.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FieldOption {
    pub id: String,
    #[serde(default)]
    pub value: String,
}

/// Cloud paginates field options; Server/DC returns them inline on the field
/// meta. `values` covers the Cloud envelope.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FieldOptionsPage {
    pub values: Vec<FieldOption>,
}

/// One entry of an issue's change history.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChangelogEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<User>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default)]
    pub items: Vec<ChangelogItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChangelogItem {
    #[serde(default)]
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_string: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_string: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ChangelogPage {
    #[serde(default)]
    pub values: Vec<ChangelogEntry>,
    #[serde(default)]
    pub histories: Vec<ChangelogEntry>,
}

/// Watchers of an issue.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Watchers {
    #[serde(default)]
    pub watch_count: u32,
    #[serde(default)]
    pub is_watching: bool,
    #[serde(default)]
    pub watchers: Vec<User>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WorklogPage {
    pub worklogs: Vec<WorklogEntry>,
}

/// A worklog entry as returned when reading an issue's time tracking.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorklogEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<User>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_spent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_spent_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// Result of a batch create — Jira reports successes and failures separately.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BatchCreateResult {
    #[serde(default)]
    pub issues: Vec<CreatedIssue>,
    #[serde(default)]
    pub errors: Vec<Value>,
}
