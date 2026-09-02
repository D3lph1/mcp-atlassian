use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Space {
    pub key: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Version {
    pub number: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Storage {
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Body {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<Storage>,
}

/// A content entity from the v1 REST API — page or comment; only the fields
/// we expose (D4).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Content {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(rename = "type", default)]
    pub content_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<Space>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<Version>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Body>,
    /// Parent chain, root first — only populated when `expand=ancestors` was
    /// requested. The last element is the direct parent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ancestors: Vec<Ancestor>,
}

/// A minimal reference to a parent page.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Ancestor {
    pub id: String,
    #[serde(default)]
    pub title: String,
}

/// v1 paginated envelope: `{results, size, start, limit, _links.next}`.
///
/// `has_more` is derived from `_links.next` (set when the server has another
/// page) with a size-equals-limit fallback for endpoints that omit the link,
/// so a caller can page with `start + size` without parsing Confluence's
/// link map.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(from = "RawResultsPage<T>")]
// The schema describes what is *serialized* — this struct — not the wire
// envelope it is read from.
#[schemars(!from)]
pub struct ResultsPage<T> {
    pub results: Vec<T>,
    /// Number of items in `results`.
    pub size: u64,
    /// Offset of the first item; pass `start + size` to get the next page.
    pub start: u64,
    /// Page size the server applied.
    pub limit: u64,
    /// Whether another page exists.
    pub has_more: bool,
}

#[derive(Deserialize)]
struct RawResultsPage<T> {
    #[serde(default = "Vec::new")]
    results: Vec<T>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    start: u64,
    #[serde(default)]
    limit: u64,
    #[serde(rename = "_links", default)]
    links: Option<PageLinks>,
}

#[derive(Deserialize, Default)]
struct PageLinks {
    #[serde(default)]
    next: Option<String>,
}

impl<T> From<RawResultsPage<T>> for ResultsPage<T> {
    fn from(raw: RawResultsPage<T>) -> Self {
        let size = raw.size.unwrap_or(raw.results.len() as u64);
        let has_more = match raw.links.and_then(|l| l.next) {
            Some(_) => true,
            None => raw.limit > 0 && size >= raw.limit,
        };
        Self {
            results: raw.results,
            size,
            start: raw.start,
            limit: raw.limit,
            has_more,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Label {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

/// An attachment on a page. `download` is a path relative to the instance
/// base URL, not an absolute URL (unlike Jira).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfluenceAttachment {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<AttachmentExtensions>,
    /// `rename` is required: the struct-level camelCase rule would otherwise
    /// mangle the leading underscore of Confluence's `_links`.
    #[serde(rename = "_links", default, skip_serializing_if = "Option::is_none")]
    pub links: Option<AttachmentLinks>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentExtensions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AttachmentLinks {
    /// Instance-relative path to the binary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<String>,
}

/// One historical version of a page.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    pub number: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Person>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default)]
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// A CQL user search hit — the user sits under a `user` key.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct UserSearchResult {
    pub user: Person,
}

/// A page template.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    #[serde(rename = "templateId", alias = "id")]
    pub template_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Body>,
}

/// Read/update restrictions of a page, keyed by operation.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Restrictions {
    #[serde(default)]
    pub results: Vec<RestrictionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RestrictionEntry {
    #[serde(default)]
    pub operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restrictions: Option<RestrictionSubjects>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RestrictionSubjects {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<ResultsPage<Person>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<ResultsPage<Group>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Group {
    #[serde(default)]
    pub name: String,
}
