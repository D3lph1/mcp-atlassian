//! Confluence REST API v1 client (`/rest/api/...`).
//!
//! The v1 content API works on both Cloud (under the `/wiki` base-URL prefix)
//! and Server/Data Center — one code path, mirroring the Jira v2 decision
//! (DECISIONS.md D5). Bodies are exchanged in storage format; conversion to
//! and from Markdown happens in the MCP layer via `storage-markdown` (D10).

mod models;

#[cfg(feature = "mcp")]
pub mod resources;
#[cfg(feature = "mcp")]
pub mod tools;

use atlassian_client::{AtlassianClient, Result, ServiceConfig};
use serde_json::{json, Value};

pub use models::{
    Ancestor, AttachmentExtensions, AttachmentLinks, Body, ConfluenceAttachment, Content, Group,
    Label, Person, RestrictionEntry, RestrictionSubjects, Restrictions, ResultsPage, Space,
    Storage, Template, Version, VersionInfo,
};

#[derive(Debug, Clone)]
pub struct ConfluenceClient {
    client: AtlassianClient,
}

impl ConfluenceClient {
    pub fn new(config: &ServiceConfig) -> Result<Self> {
        Ok(Self {
            client: AtlassianClient::new(config)?,
        })
    }

    /// CQL search over content, e.g. `space = DEV AND title ~ "runbook"`.
    pub async fn search(&self, cql: &str, limit: u32, start: u32) -> Result<ResultsPage<Content>> {
        let limit = limit.to_string();
        let start = start.to_string();
        self.client
            .get(
                "/rest/api/content/search",
                &[
                    ("cql", cql),
                    ("limit", &limit),
                    ("start", &start),
                    ("expand", "space,version"),
                ],
            )
            .await
    }

    /// Fetches a page with its storage body and version.
    pub async fn get_page(&self, page_id: &str) -> Result<Content> {
        let path = format!("/rest/api/content/{page_id}");
        self.client
            .get(&path, &[("expand", "body.storage,version,space")])
            .await
    }

    pub async fn get_page_children(
        &self,
        page_id: &str,
        limit: u32,
    ) -> Result<ResultsPage<Content>> {
        let path = format!("/rest/api/content/{page_id}/child/page");
        let limit = limit.to_string();
        self.client
            .get(&path, &[("limit", &limit), ("expand", "version")])
            .await
    }

    /// `storage_body` must already be in storage representation (XHTML).
    pub async fn create_page(
        &self,
        space_key: &str,
        title: &str,
        storage_body: &str,
        parent_id: Option<&str>,
    ) -> Result<Content> {
        let mut body = json!({
            "type": "page",
            "title": title,
            "space": { "key": space_key },
            "body": {
                "storage": { "value": storage_body, "representation": "storage" }
            }
        });
        if let Some(parent) = parent_id {
            body["ancestors"] = json!([{ "id": parent }]);
        }
        self.client.post("/rest/api/content", &body).await
    }

    /// Updates title and/or body. Confluence requires the next version number
    /// on every update — we fetch the current one and increment.
    pub async fn update_page(
        &self,
        page_id: &str,
        title: Option<&str>,
        storage_body: Option<&str>,
    ) -> Result<Content> {
        let current = self.get_page(page_id).await?;
        let version = current.version.as_ref().map(|v| v.number).unwrap_or(1) + 1;
        let title = title.unwrap_or(&current.title);
        let storage = match storage_body {
            Some(s) => s.to_string(),
            None => current
                .body
                .as_ref()
                .and_then(|b| b.storage.as_ref())
                .map(|s| s.value.clone())
                .unwrap_or_default(),
        };
        let path = format!("/rest/api/content/{page_id}");
        self.client
            .put(
                &path,
                &json!({
                    "type": "page",
                    "title": title,
                    "version": { "number": version },
                    "body": {
                        "storage": { "value": storage, "representation": "storage" }
                    }
                }),
            )
            .await
    }

    pub async fn delete_page(&self, page_id: &str) -> Result<()> {
        let path = format!("/rest/api/content/{page_id}");
        self.client.delete(&path, &[]).await
    }

    pub async fn get_comments(&self, page_id: &str, limit: u32) -> Result<ResultsPage<Content>> {
        let path = format!("/rest/api/content/{page_id}/child/comment");
        let limit = limit.to_string();
        self.client
            .get(
                &path,
                &[("limit", &limit), ("expand", "body.storage,version")],
            )
            .await
    }

    /// `storage_body` must already be in storage representation (XHTML).
    pub async fn add_comment(&self, page_id: &str, storage_body: &str) -> Result<Content> {
        self.client
            .post(
                "/rest/api/content",
                &json!({
                    "type": "comment",
                    "container": { "id": page_id, "type": "page" },
                    "body": {
                        "storage": { "value": storage_body, "representation": "storage" }
                    }
                }),
            )
            .await
    }

    pub async fn get_spaces(&self, limit: u32) -> Result<ResultsPage<Space>> {
        let limit = limit.to_string();
        self.client
            .get("/rest/api/space", &[("limit", &limit)])
            .await
    }

    pub async fn get_labels(&self, page_id: &str) -> Result<ResultsPage<Label>> {
        let path = format!("/rest/api/content/{page_id}/label");
        self.client.get(&path, &[]).await
    }

    pub async fn add_label(&self, page_id: &str, label: &str) -> Result<ResultsPage<Label>> {
        let path = format!("/rest/api/content/{page_id}/label");
        self.client
            .post(&path, &json!([{ "prefix": "global", "name": label }]))
            .await
    }

    // ---- Page structure ----------------------------------------------------

    /// Moves a page under a different parent and/or into another space.
    /// Confluence has no dedicated move endpoint on v1 — the update carries
    /// the new ancestor and space.
    pub async fn move_page(
        &self,
        page_id: &str,
        target_parent_id: Option<&str>,
        target_space_key: Option<&str>,
    ) -> Result<Content> {
        let current = self.get_page(page_id).await?;
        let version = current.version.as_ref().map(|v| v.number).unwrap_or(1) + 1;
        let storage = current
            .body
            .as_ref()
            .and_then(|b| b.storage.as_ref())
            .map(|s| s.value.clone())
            .unwrap_or_default();
        let mut body = json!({
            "type": "page",
            "title": current.title,
            "version": { "number": version },
            "body": { "storage": { "value": storage, "representation": "storage" } },
        });
        if let Some(parent) = target_parent_id {
            body["ancestors"] = json!([{ "id": parent }]);
        }
        if let Some(space) = target_space_key {
            body["space"] = json!({ "key": space });
        }
        let path = format!("/rest/api/content/{page_id}");
        self.client.put(&path, &body).await
    }

    /// Every page of a space, flat. Callers build the tree from each page's
    /// ancestors; CQL cannot return a nested structure.
    pub async fn get_space_pages(
        &self,
        space_key: &str,
        limit: u32,
    ) -> Result<ResultsPage<Content>> {
        let limit = limit.to_string();
        let cql = format!("space = \"{space_key}\" AND type = page ORDER BY title");
        self.client
            .get(
                "/rest/api/content/search",
                &[
                    ("cql", &cql),
                    ("limit", &limit),
                    ("expand", "ancestors,version"),
                ],
            )
            .await
    }

    // ---- Versions ----------------------------------------------------------

    pub async fn get_page_versions(
        &self,
        page_id: &str,
        limit: u32,
    ) -> Result<ResultsPage<VersionInfo>> {
        let path = format!("/rest/api/content/{page_id}/version");
        let limit = limit.to_string();
        self.client.get(&path, &[("limit", &limit)]).await
    }

    /// Body of one historical version, in storage format.
    pub async fn get_page_version_body(&self, page_id: &str, version: u64) -> Result<Content> {
        let path = format!("/rest/api/content/{page_id}");
        let version = version.to_string();
        self.client
            .get(
                &path,
                &[
                    ("status", "historical"),
                    ("version", &version),
                    ("expand", "body.storage,version"),
                ],
            )
            .await
    }

    // ---- Comments ----------------------------------------------------------

    /// Replies to an existing comment by making it the container's parent.
    pub async fn reply_to_comment(&self, comment_id: &str, storage_body: &str) -> Result<Content> {
        self.client
            .post(
                "/rest/api/content",
                &json!({
                    "type": "comment",
                    "ancestors": [{ "id": comment_id }],
                    "body": { "storage": { "value": storage_body, "representation": "storage" } },
                }),
            )
            .await
    }

    /// Inline comments are anchored to a text selection; they come back from
    /// the same child endpoint with an inline-specific location.
    pub async fn get_inline_comments(
        &self,
        page_id: &str,
        limit: u32,
    ) -> Result<ResultsPage<Content>> {
        let path = format!("/rest/api/content/{page_id}/child/comment");
        let limit = limit.to_string();
        self.client
            .get(
                &path,
                &[
                    ("limit", &limit),
                    ("location", "inline"),
                    ("expand", "body.storage,version,extensions.inlineProperties"),
                ],
            )
            .await
    }

    /// Creates a comment anchored to `text_selection` within the page.
    pub async fn add_inline_comment(
        &self,
        page_id: &str,
        storage_body: &str,
        text_selection: &str,
    ) -> Result<Content> {
        self.client
            .post(
                "/rest/api/content",
                &json!({
                    "type": "comment",
                    "container": { "id": page_id, "type": "page" },
                    "body": { "storage": { "value": storage_body, "representation": "storage" } },
                    "extensions": {
                        "location": "inline",
                        "inlineProperties": {
                            "originalSelection": text_selection,
                        }
                    },
                }),
            )
            .await
    }

    // ---- Users -------------------------------------------------------------

    /// Finds users through CQL. Confluence has no plain user-search endpoint
    /// on v1, so this goes through the search API with a `user.fullname` term.
    pub async fn search_users(&self, query: &str, limit: u32) -> Result<Vec<Person>> {
        let limit = limit.to_string();
        let cql = format!("user.fullname ~ \"{query}\"");
        let page: ResultsPage<models::UserSearchResult> = self
            .client
            .get("/rest/api/search/user", &[("cql", &cql), ("limit", &limit)])
            .await?;
        Ok(page.results.into_iter().map(|r| r.user).collect())
    }

    // ---- Attachments -------------------------------------------------------

    pub async fn get_attachments(
        &self,
        page_id: &str,
        limit: u32,
    ) -> Result<ResultsPage<ConfluenceAttachment>> {
        let path = format!("/rest/api/content/{page_id}/child/attachment");
        let limit = limit.to_string();
        self.client
            .get(&path, &[("limit", &limit), ("expand", "extensions")])
            .await
    }

    /// Downloads an attachment binary. `download_path` is the instance-relative
    /// path from the attachment's `_links.download`.
    pub async fn download_attachment(&self, download_path: &str) -> Result<Vec<u8>> {
        self.client.get_bytes(download_path).await
    }

    pub async fn upload_attachment(
        &self,
        page_id: &str,
        file_name: &str,
        bytes: Vec<u8>,
    ) -> Result<ResultsPage<ConfluenceAttachment>> {
        let path = format!("/rest/api/content/{page_id}/child/attachment");
        self.client.post_multipart(&path, file_name, bytes).await
    }

    pub async fn delete_attachment(&self, attachment_id: &str) -> Result<()> {
        let path = format!("/rest/api/content/{attachment_id}");
        self.client.delete(&path, &[]).await
    }

    // ---- Templates ---------------------------------------------------------

    pub async fn list_templates(
        &self,
        space_key: Option<&str>,
        limit: u32,
    ) -> Result<ResultsPage<Template>> {
        let limit = limit.to_string();
        let mut params: Vec<(&str, &str)> = vec![("limit", &limit)];
        if let Some(space) = space_key {
            params.push(("spaceKey", space));
        }
        self.client.get("/rest/api/template/page", &params).await
    }

    pub async fn get_template(&self, template_id: &str) -> Result<Template> {
        let path = format!("/rest/api/template/{template_id}");
        self.client.get(&path, &[]).await
    }

    // ---- Restrictions ------------------------------------------------------

    pub async fn get_restrictions(&self, page_id: &str) -> Result<Restrictions> {
        let path = format!("/rest/api/content/{page_id}/restriction");
        self.client
            .get(&path, &[("expand", "restrictions.user,restrictions.group")])
            .await
    }

    /// Replaces the read/update restrictions of a page. Empty lists clear the
    /// corresponding restriction, making the page inherit space permissions.
    pub async fn set_restrictions(
        &self,
        page_id: &str,
        read_users: &[String],
        update_users: &[String],
        read_groups: &[String],
        update_groups: &[String],
    ) -> Result<Restrictions> {
        let path = format!("/rest/api/content/{page_id}/restriction");
        let body = json!([
            restriction_entry("read", read_users, read_groups),
            restriction_entry("update", update_users, update_groups),
        ]);
        self.client.put(&path, &body).await
    }
}

/// Builds one `{operation, restrictions: {user, group}}` entry. User ids are
/// account ids on Cloud and usernames on Server/DC; both are sent under the
/// key the deployment expects by including each form.
fn restriction_entry(operation: &str, users: &[String], groups: &[String]) -> Value {
    let user_entries: Vec<Value> = users
        .iter()
        .map(|u| json!({ "type": "known", "accountId": u, "username": u }))
        .collect();
    let group_entries: Vec<Value> = groups
        .iter()
        .map(|g| json!({ "type": "group", "name": g }))
        .collect();
    json!({
        "operation": operation,
        "restrictions": {
            "user": { "results": user_entries },
            "group": { "results": group_entries },
        }
    })
}

/// MCP tool state for Confluence: the client the tools operate on.
///
/// Tools are inherent methods on this type (`#[tool_router]` requires that),
/// so they live in this crate next to the client they call. The MCP server
/// composes the resulting router onto its own state — see the `mcp-atlassian`
/// crate.
#[cfg(feature = "mcp")]
#[derive(Debug, Clone)]
pub struct ConfluenceTools {
    client: std::sync::Arc<ConfluenceClient>,
}

#[cfg(feature = "mcp")]
impl ConfluenceTools {
    pub fn new(client: std::sync::Arc<ConfluenceClient>) -> Self {
        Self { client }
    }

    pub(crate) fn client(&self) -> &ConfluenceClient {
        &self.client
    }
}
