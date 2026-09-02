//! Jira REST API v2 client.
//!
//! v2 is used for both Cloud and Server/Data Center — one code path, no ADF
//! (see DECISIONS.md D5). Models deserialize only the fields we actually use.
//!
//! The one endpoint that diverges is search: Cloud removed `/rest/api/2/search`
//! in favor of the token-paginated `/rest/api/2/search/jql`, while Server/DC
//! still uses the offset-paginated original. Deployment is inferred from the
//! auth mode — API token (Basic) => Cloud, PAT (Bearer) => Server/DC (D6) —
//! unless `JIRA_DEPLOYMENT` says otherwise (D41).

mod models;

#[cfg(feature = "mcp")]
pub mod prompts;
#[cfg(feature = "mcp")]
pub mod resources;
#[cfg(feature = "mcp")]
pub mod tools;

use std::sync::Arc;
use std::time::Duration;

use mcp_atlassian_client::query::quote;
use mcp_atlassian_client::{AtlassianClient, Deployment, Result, ServiceConfig, TtlCache, Upload};
use serde_json::{json, Map, Value};

pub use models::{
    AgilePage, Attachment, BatchCreateResult, Board, ChangelogEntry, ChangelogItem, Comment,
    CommentPage, CreatedIssue, Field, FieldOption, FieldSchema, Issue, IssueFields, IssueLink,
    IssueType, LinkType, LinkedIssue, LinkedIssueFields, Myself, Named, Project, RemoteLink,
    SearchPage, Sprint, Transition, User, Watchers, Worklog, WorklogEntry,
};

/// Default fields requested by search — compact but useful for an LLM.
pub const DEFAULT_SEARCH_FIELDS: &str =
    "summary,status,assignee,issuetype,priority,created,updated";

/// Default fields of a single issue: everything `IssueFields` models (D35).
/// Naming them keeps the payload small — omitting `fields` would return every
/// custom field the instance has.
pub const DEFAULT_ISSUE_FIELDS: &str = "summary,description,status,priority,issuetype,resolution,\
     assignee,reporter,labels,components,fixVersions,created,updated,duedate,parent,subtasks,\
     issuelinks";

#[derive(Debug, Clone)]
pub struct JiraClient {
    client: AtlassianClient,
    cloud: bool,
    /// Reference data only, and only when a TTL is configured (D25).
    cache: Option<Arc<TtlCache>>,
}

/// Parameters for [`JiraClient::search`].
#[derive(Debug, Clone, Default)]
pub struct SearchParams {
    pub jql: String,
    pub max_results: u32,
    /// Comma-separated field list; defaults to [`DEFAULT_SEARCH_FIELDS`].
    pub fields: Option<String>,
    /// Server/DC offset pagination.
    pub start_at: Option<u32>,
    /// Cloud token pagination.
    pub next_page_token: Option<String>,
}

/// Where [`JiraClient::get_field_options`] reads the options from.
#[derive(Debug, Clone, Copy, Default)]
pub struct FieldOptionsScope<'a> {
    /// The edit screen of this issue. Takes precedence.
    pub issue_key: Option<&'a str>,
    /// The create screen of this project…
    pub project_key: Option<&'a str>,
    /// …for this issue type (name); the project's first type when absent.
    pub issue_type: Option<&'a str>,
}

/// Parameters for [`JiraClient::create_issue`].
#[derive(Debug, Clone, Default)]
pub struct CreateIssueParams {
    pub project_key: String,
    pub issue_type: String,
    pub summary: String,
    pub description: Option<String>,
    /// Cloud: account id; Server/DC: username.
    pub assignee: Option<String>,
    pub priority: Option<String>,
    pub labels: Vec<String>,
    /// Extra raw fields merged into the `fields` object (e.g. custom fields).
    pub additional_fields: Option<Map<String, Value>>,
}

impl JiraClient {
    pub fn new(config: &ServiceConfig) -> Result<Self> {
        Ok(Self {
            client: AtlassianClient::new(config)?,
            cloud: config.deployment() == Deployment::Cloud,
            cache: None,
        })
    }

    /// Caches reference data — projects, issue types, boards, link types,
    /// field definitions and the current user — for `ttl`. Everything else
    /// keeps going to Jira on every call (D25).
    pub fn with_cache(mut self, ttl: Duration) -> Self {
        self.cache = Some(Arc::new(TtlCache::new(ttl)));
        self
    }

    /// Per-request timeout (`REQUEST_TIMEOUT`).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.client = self.client.with_timeout(timeout);
        self
    }

    /// Runs `fetch` through the cache when one is configured, unchanged
    /// otherwise. Keys are namespaced by endpoint and carry every argument
    /// that narrows the answer.
    async fn cached<T, F, Fut>(&self, key: &str, fetch: F) -> Result<T>
    where
        T: Clone + Send + Sync + 'static,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        mcp_atlassian_client::cached(self.cache.as_deref(), key, fetch).await
    }

    /// Returns the currently authenticated user. Cheap smoke-test endpoint —
    /// also useful for verifying credentials.
    pub async fn get_myself(&self) -> Result<Myself> {
        self.cached("myself", || self.client.get("/rest/api/2/myself", &[]))
            .await
    }

    /// JQL search. Routes to the deployment-appropriate endpoint.
    pub async fn search(&self, params: &SearchParams) -> Result<SearchPage> {
        let max_results = params.max_results.to_string();
        let fields = params.fields.as_deref().unwrap_or(DEFAULT_SEARCH_FIELDS);
        let mut query: Vec<(&str, &str)> = vec![
            ("jql", &params.jql),
            ("maxResults", &max_results),
            ("fields", fields),
        ];

        // A paging parameter of the other deployment is a mistake worth
        // reporting, not one worth ignoring: the caller would otherwise get
        // the first page again and conclude there is no second one.
        if self.cloud && params.start_at.is_some() {
            return Err(mcp_atlassian_client::Error::Config(
                "start_at is Server/Data Center pagination; this is Jira Cloud — pass the \
                 next_page_token from the previous page instead"
                    .into(),
            ));
        }
        if !self.cloud && params.next_page_token.is_some() {
            return Err(mcp_atlassian_client::Error::Config(
                "next_page_token is Jira Cloud pagination; this is Server/Data Center — pass \
                 start_at instead"
                    .into(),
            ));
        }

        let mut page: SearchPage = if self.cloud {
            if let Some(token) = &params.next_page_token {
                query.push(("nextPageToken", token));
            }
            self.client.get("/rest/api/2/search/jql", &query).await?
        } else {
            let start_at = params.start_at.unwrap_or(0).to_string();
            query.push(("startAt", &start_at));
            self.client.get("/rest/api/2/search", &query).await?
        };
        for issue in &mut page.issues {
            prune_extra(issue, fields);
        }
        Ok(page)
    }

    /// One issue. `fields` is Jira's comma-separated list; `None` requests
    /// [`DEFAULT_ISSUE_FIELDS`]. Fields that `IssueFields` does not model —
    /// custom fields, `*all` — are kept in `extra` when asked for by name.
    pub async fn get_issue(&self, key: &str, fields: Option<&str>) -> Result<Issue> {
        let path = format!("/rest/api/2/issue/{key}");
        let fields = fields.unwrap_or(DEFAULT_ISSUE_FIELDS);
        let mut issue: Issue = self.client.get(&path, &[("fields", fields)]).await?;
        prune_extra(&mut issue, fields);
        Ok(issue)
    }

    pub async fn create_issue(&self, params: &CreateIssueParams) -> Result<CreatedIssue> {
        let mut fields = Map::new();
        fields.insert("project".into(), json!({ "key": params.project_key }));
        fields.insert("issuetype".into(), json!({ "name": params.issue_type }));
        fields.insert("summary".into(), json!(params.summary));
        if let Some(description) = &params.description {
            fields.insert("description".into(), json!(description));
        }
        if let Some(assignee) = &params.assignee {
            fields.insert("assignee".into(), self.user_ref(assignee));
        }
        if let Some(priority) = &params.priority {
            fields.insert("priority".into(), json!({ "name": priority }));
        }
        if !params.labels.is_empty() {
            fields.insert("labels".into(), json!(params.labels));
        }
        if let Some(extra) = &params.additional_fields {
            fields.extend(extra.clone());
        }
        self.client
            .post("/rest/api/2/issue", &json!({ "fields": fields }))
            .await
    }

    /// Raw field update: `fields` is passed through as the `fields` object of
    /// `PUT /rest/api/2/issue/{key}`. Use for summary, description, labels,
    /// priority, custom fields, etc.
    pub async fn update_issue(&self, key: &str, fields: &Map<String, Value>) -> Result<()> {
        let path = format!("/rest/api/2/issue/{key}");
        self.client
            .put_no_content(&path, &json!({ "fields": fields }))
            .await
    }

    pub async fn delete_issue(&self, key: &str, delete_subtasks: bool) -> Result<()> {
        let path = format!("/rest/api/2/issue/{key}");
        let subtasks = delete_subtasks.to_string();
        self.client
            .delete(&path, &[("deleteSubtasks", &subtasks)])
            .await
    }

    pub async fn get_transitions(&self, key: &str) -> Result<Vec<Transition>> {
        let path = format!("/rest/api/2/issue/{key}/transitions");
        let resp: models::TransitionsResponse = self.client.get(&path, &[]).await?;
        Ok(resp.transitions)
    }

    pub async fn transition_issue(
        &self,
        key: &str,
        transition_id: &str,
        comment: Option<&str>,
    ) -> Result<()> {
        let path = format!("/rest/api/2/issue/{key}/transitions");
        let mut body = json!({ "transition": { "id": transition_id } });
        if let Some(comment) = comment {
            body["update"] = json!({ "comment": [{ "add": { "body": comment } }] });
        }
        self.client.post_no_content(&path, &body).await
    }

    pub async fn add_comment(&self, key: &str, body: &str) -> Result<Comment> {
        let path = format!("/rest/api/2/issue/{key}/comment");
        self.client.post(&path, &json!({ "body": body })).await
    }

    /// Comments, newest first. Cloud honours `orderBy=-created`; Server/DC
    /// ignores it and answers oldest first, so the page is sorted here as
    /// well — both deployments then match the tool's description.
    pub async fn get_comments(
        &self,
        key: &str,
        max_results: u32,
        start_at: u32,
    ) -> Result<CommentPage> {
        let path = format!("/rest/api/2/issue/{key}/comment");
        let max_results = max_results.to_string();
        let start_at = start_at.to_string();
        let mut page: CommentPage = self
            .client
            .get(
                &path,
                &[
                    ("maxResults", &max_results),
                    ("startAt", &start_at),
                    ("orderBy", "-created"),
                ],
            )
            .await?;
        page.comments.sort_by(|a, b| b.created.cmp(&a.created));
        Ok(page)
    }

    /// `time_spent` uses Jira duration syntax ("2h", "1d 4h", "30m").
    /// `started` is an ISO-8601 timestamp like `2026-01-15T10:00:00.000+0000`.
    pub async fn add_worklog(
        &self,
        key: &str,
        time_spent: &str,
        comment: Option<&str>,
        started: Option<&str>,
    ) -> Result<Worklog> {
        let path = format!("/rest/api/2/issue/{key}/worklog");
        let mut body = json!({ "timeSpent": time_spent });
        if let Some(comment) = comment {
            body["comment"] = json!(comment);
        }
        if let Some(started) = started {
            body["started"] = json!(started);
        }
        self.client.post(&path, &body).await
    }

    pub async fn get_projects(&self) -> Result<Vec<Project>> {
        self.cached("projects", || async {
            if self.cloud {
                // Cloud deprecated the plain list endpoint in favor of the
                // paginated one; 50 projects per page is plenty for tool use.
                let page: models::ProjectPage = self
                    .client
                    .get("/rest/api/2/project/search", &[("maxResults", "50")])
                    .await?;
                Ok(page.values)
            } else {
                self.client.get("/rest/api/2/project", &[]).await
            }
        })
        .await
    }

    pub async fn get_issue_types(&self) -> Result<Vec<IssueType>> {
        self.cached("issue-types", || {
            self.client.get("/rest/api/2/issuetype", &[])
        })
        .await
    }

    /// Searches users by name / email. The query parameter differs by
    /// deployment: Cloud matches display name and email via `query`,
    /// Server/DC matches the username via `username` (D16).
    pub async fn search_users(&self, query: &str, max_results: u32) -> Result<Vec<User>> {
        let max_results = max_results.to_string();
        let param = if self.cloud { "query" } else { "username" };
        self.client
            .get(
                "/rest/api/2/user/search",
                &[(param, query), ("maxResults", &max_results)],
            )
            .await
    }

    // ---- Agile API (/rest/agile/1.0, same shape on Cloud and Server/DC) ----

    /// Lists boards, optionally filtered by project key.
    pub async fn get_boards(
        &self,
        project_key: Option<&str>,
        max_results: u32,
    ) -> Result<AgilePage<Board>> {
        let max_results = max_results.to_string();
        let key = format!("boards:{}:{max_results}", project_key.unwrap_or(""));
        self.cached(&key, || async {
            let mut query: Vec<(&str, &str)> = vec![("maxResults", &max_results)];
            if let Some(project) = project_key {
                query.push(("projectKeyOrId", project));
            }
            self.client.get("/rest/agile/1.0/board", &query).await
        })
        .await
    }

    /// Lists sprints of a board; `state` filters by `active`, `future`, `closed`
    /// (comma-separated allowed).
    pub async fn get_sprints(
        &self,
        board_id: u64,
        state: Option<&str>,
        max_results: u32,
        start_at: u32,
    ) -> Result<AgilePage<Sprint>> {
        let path = format!("/rest/agile/1.0/board/{board_id}/sprint");
        let max_results = max_results.to_string();
        let start_at = start_at.to_string();
        let mut query: Vec<(&str, &str)> =
            vec![("maxResults", &max_results), ("startAt", &start_at)];
        if let Some(state) = state {
            query.push(("state", state));
        }
        self.client.get(&path, &query).await
    }

    pub async fn get_sprint_issues(
        &self,
        sprint_id: u64,
        max_results: u32,
        start_at: u32,
    ) -> Result<SearchPage> {
        let path = format!("/rest/agile/1.0/sprint/{sprint_id}/issue");
        let max_results = max_results.to_string();
        let start_at = start_at.to_string();
        let mut page: SearchPage = self
            .client
            .get(
                &path,
                &[
                    ("maxResults", &max_results),
                    ("startAt", &start_at),
                    ("fields", DEFAULT_SEARCH_FIELDS),
                ],
            )
            .await?;
        for issue in &mut page.issues {
            prune_extra(issue, DEFAULT_SEARCH_FIELDS);
        }
        Ok(page)
    }

    /// Moves issues into a sprint (max 50 per call, Agile API limit).
    pub async fn move_issues_to_sprint(&self, sprint_id: u64, issue_keys: &[String]) -> Result<()> {
        let path = format!("/rest/agile/1.0/sprint/{sprint_id}/issue");
        self.client
            .post_no_content(&path, &json!({ "issues": issue_keys }))
            .await
    }

    // ---- Attachments -------------------------------------------------------

    pub async fn get_attachments(&self, key: &str) -> Result<Vec<Attachment>> {
        let path = format!("/rest/api/2/issue/{key}");
        let resp: models::IssueAttachments =
            self.client.get(&path, &[("fields", "attachment")]).await?;
        Ok(resp.fields.attachment)
    }

    /// Downloads an attachment's binary.
    ///
    /// The `content` URL Jira reports is used when it points at the configured
    /// instance. Under OAuth it never does — the base is the
    /// `api.atlassian.com` gateway while `content` names the site — so Cloud
    /// falls back to the gateway's own `attachment/content/{id}` endpoint,
    /// which answers with a redirect to the binary (D33).
    pub async fn download_attachment(&self, attachment: &Attachment) -> Result<Vec<u8>> {
        self.client
            .get_bytes(&self.attachment_link(attachment))
            .await
    }

    /// Streams an attachment into `path` (D37); returns the size written.
    pub async fn download_attachment_to(
        &self,
        attachment: &Attachment,
        path: &std::path::Path,
        max_bytes: Option<u64>,
    ) -> Result<u64> {
        self.client
            .download_to_file(&self.attachment_link(attachment), path, max_bytes)
            .await
    }

    fn attachment_link(&self, attachment: &Attachment) -> String {
        if !self.client.same_origin(&attachment.content) && self.cloud {
            format!("/rest/api/2/attachment/content/{}", attachment.id)
        } else {
            attachment.content.clone()
        }
    }

    /// Uploads one file as an attachment; returns the created attachment(s).
    pub async fn upload_attachment(&self, key: &str, upload: Upload) -> Result<Vec<Attachment>> {
        let path = format!("/rest/api/2/issue/{key}/attachments");
        self.client.post_multipart(&path, upload).await
    }

    // ---- Users and watchers ------------------------------------------------

    /// Looks up one user by identifier: account id on Cloud, username on
    /// Server/DC.
    pub async fn get_user_profile(&self, identifier: &str) -> Result<User> {
        let param = if self.cloud { "accountId" } else { "username" };
        self.client
            .get("/rest/api/2/user", &[(param, identifier)])
            .await
    }

    /// Users assignable to a project (or to a specific issue when `issue_key`
    /// is given) — narrower and more correct than a plain user search, since
    /// it respects the project's assignable permission.
    pub async fn search_assignable_users(
        &self,
        query: &str,
        project_key: Option<&str>,
        issue_key: Option<&str>,
        max_results: u32,
    ) -> Result<Vec<User>> {
        let max_results = max_results.to_string();
        let query_param = if self.cloud { "query" } else { "username" };
        let mut params: Vec<(&str, &str)> =
            vec![(query_param, query), ("maxResults", &max_results)];
        if let Some(issue) = issue_key {
            params.push(("issueKey", issue));
        } else if let Some(project) = project_key {
            params.push(("project", project));
        }
        self.client
            .get("/rest/api/2/user/assignable/search", &params)
            .await
    }

    pub async fn get_watchers(&self, key: &str) -> Result<Watchers> {
        let path = format!("/rest/api/2/issue/{key}/watchers");
        self.client.get(&path, &[]).await
    }

    /// The watcher endpoint takes a bare JSON string body: an account id on
    /// Cloud, a username on Server/DC.
    pub async fn add_watcher(&self, key: &str, user: &str) -> Result<()> {
        let path = format!("/rest/api/2/issue/{key}/watchers");
        self.client.post_no_content(&path, &json!(user)).await
    }

    pub async fn remove_watcher(&self, key: &str, user: &str) -> Result<()> {
        let path = format!("/rest/api/2/issue/{key}/watchers");
        let param = if self.cloud { "accountId" } else { "username" };
        self.client.delete(&path, &[(param, user)]).await
    }

    /// Dedicated assignment endpoint. Passing `None` unassigns the issue.
    pub async fn assign_issue(&self, key: &str, assignee: Option<&str>) -> Result<()> {
        let path = format!("/rest/api/2/issue/{key}/assignee");
        let body = match assignee {
            Some(user) => self.user_ref(user),
            None if self.cloud => json!({ "accountId": null }),
            None => json!({ "name": null }),
        };
        self.client.put_no_content(&path, &body).await
    }

    // ---- Fields ------------------------------------------------------------

    /// All field definitions. Filter client-side by `query` (case-insensitive
    /// substring over name and id) — Jira has no server-side field search.
    pub async fn search_fields(&self, query: Option<&str>) -> Result<Vec<Field>> {
        // The filter runs here, so the cache holds the full field list and one
        // entry serves every query.
        let fields: Vec<Field> = self
            .cached("fields", || self.client.get("/rest/api/2/field", &[]))
            .await?;
        let Some(query) = query else {
            return Ok(fields);
        };
        let needle = query.to_lowercase();
        Ok(fields
            .into_iter()
            .filter(|f| {
                f.name.to_lowercase().contains(&needle) || f.id.to_lowercase().contains(&needle)
            })
            .collect())
    }

    /// Allowed values of a select-like field, read from the screen that
    /// would offer them (D34): the edit screen of `issue_key`, or the create
    /// screen of `project_key` (+ `issue_type`, else the project's first).
    /// Both work for any user on every deployment. With neither, Cloud falls
    /// back to the field-context API, which needs Jira administration.
    pub async fn get_field_options(
        &self,
        field_id: &str,
        scope: FieldOptionsScope<'_>,
        max_results: u32,
    ) -> Result<Vec<FieldOption>> {
        let not_offered = |screen: String| {
            mcp_atlassian_client::Error::Config(format!(
                "field {field_id} is not on {screen}, or has no fixed set of values; \
                 check the id with jira_search_fields"
            ))
        };
        let mut options = if let Some(key) = scope.issue_key {
            let path = format!("/rest/api/2/issue/{key}/editmeta");
            let mut meta: models::EditMeta = self.client.get(&path, &[]).await?;
            let field = meta
                .fields
                .remove(field_id)
                .ok_or_else(|| not_offered(format!("the edit screen of {key}")))?;
            let field: models::MetaField = serde_json::from_value(field)
                .map_err(|e| mcp_atlassian_client::Error::Decode(e.to_string()))?;
            field.allowed_values
        } else if let Some(project) = scope.project_key {
            let issue_type = self.create_issue_type(project, scope.issue_type).await?;
            let path = format!(
                "/rest/api/2/issue/createmeta/{project}/issuetypes/{}",
                issue_type.id
            );
            let page: models::CreateMetaFields =
                self.client.get(&path, &[("maxResults", "200")]).await?;
            page.values
                .into_iter()
                .find(|f| f.field_id == field_id)
                .ok_or_else(|| {
                    not_offered(format!(
                        "the create screen of {project} / {}",
                        issue_type.name
                    ))
                })?
                .allowed_values
        } else if self.cloud {
            let path = format!("/rest/api/2/field/{field_id}/context");
            let contexts: models::FieldContextPage = self.client.get(&path, &[]).await?;
            let context = contexts.values.into_iter().next().ok_or_else(|| {
                mcp_atlassian_client::Error::Config(format!(
                    "field {field_id} has no context; pass issue_key or project_key to read \
                     its options from a screen instead"
                ))
            })?;
            let path = format!("/rest/api/2/field/{field_id}/context/{}/option", context.id);
            let max_results = max_results.to_string();
            let page: models::FieldOptionsPage = self
                .client
                .get(&path, &[("maxResults", &max_results)])
                .await?;
            page.values
        } else {
            return Err(mcp_atlassian_client::Error::Config(
                "on Server/Data Center pass issue_key, or project_key (and issue_type), so the \
                 options can be read from that screen"
                    .into(),
            ));
        };
        options.truncate(max_results as usize);
        Ok(options)
    }

    /// The issue type a create screen is looked up for: by name when given,
    /// else the project's first.
    async fn create_issue_type(&self, project: &str, name: Option<&str>) -> Result<IssueType> {
        let path = format!("/rest/api/2/issue/createmeta/{project}/issuetypes");
        let page: models::CreateMetaIssueTypes = self.client.get(&path, &[]).await?;
        let names: Vec<&str> = page.values.iter().map(|t| t.name.as_str()).collect();
        let found = match name {
            Some(name) => page
                .values
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(name)),
            None => page.values.first(),
        };
        found.cloned().ok_or_else(|| {
            mcp_atlassian_client::Error::Config(match name {
                Some(name) => format!(
                    "issue type `{name}` cannot be created in {project}; available: {}",
                    names.join(", ")
                ),
                None => format!("no issue type can be created in {project}"),
            })
        })
    }

    // ---- Issue links -------------------------------------------------------

    pub async fn get_link_types(&self) -> Result<Vec<LinkType>> {
        self.cached("link-types", || async {
            let resp: models::LinkTypesResponse =
                self.client.get("/rest/api/2/issueLinkType", &[]).await?;
            Ok(resp.issue_link_types)
        })
        .await
    }

    /// Links two issues. `link_type` is a type *name* (see `get_link_types`);
    /// direction follows the type's inward/outward phrasing.
    pub async fn create_issue_link(
        &self,
        link_type: &str,
        inward_issue: &str,
        outward_issue: &str,
        comment: Option<&str>,
    ) -> Result<()> {
        let mut body = json!({
            "type": { "name": link_type },
            "inwardIssue": { "key": inward_issue },
            "outwardIssue": { "key": outward_issue },
        });
        if let Some(comment) = comment {
            body["comment"] = json!({ "body": comment });
        }
        self.client
            .post_no_content("/rest/api/2/issueLink", &body)
            .await
    }

    pub async fn remove_issue_link(&self, link_id: &str) -> Result<()> {
        let path = format!("/rest/api/2/issueLink/{link_id}");
        self.client.delete(&path, &[]).await
    }

    /// Attaches an external URL (or Confluence page) to an issue as a remote
    /// link.
    pub async fn create_remote_issue_link(
        &self,
        key: &str,
        url: &str,
        title: &str,
        summary: Option<&str>,
    ) -> Result<RemoteLink> {
        let path = format!("/rest/api/2/issue/{key}/remotelink");
        let mut object = json!({ "url": url, "title": title });
        if let Some(summary) = summary {
            object["summary"] = json!(summary);
        }
        self.client.post(&path, &json!({ "object": object })).await
    }

    /// Puts an issue under an epic. Cloud uses the `parent` field; Server/DC
    /// uses the "Epic Link" custom field, whose id varies per instance and is
    /// resolved through the field list.
    pub async fn link_to_epic(&self, key: &str, epic_key: &str) -> Result<()> {
        let mut fields = Map::new();
        if self.cloud {
            fields.insert("parent".into(), json!({ "key": epic_key }));
        } else {
            let epic_field = self
                .search_fields(Some("Epic Link"))
                .await?
                .into_iter()
                .find(|f| f.name.eq_ignore_ascii_case("Epic Link"))
                .ok_or_else(|| {
                    mcp_atlassian_client::Error::Config(
                        "this instance has no `Epic Link` field; link the issue via \
                         jira_create_issue_link instead"
                            .into(),
                    )
                })?;
            fields.insert(epic_field.id, json!(epic_key));
        }
        self.update_issue(key, &fields).await
    }

    // ---- Comments and worklog ----------------------------------------------

    pub async fn edit_comment(&self, key: &str, comment_id: &str, body: &str) -> Result<Comment> {
        let path = format!("/rest/api/2/issue/{key}/comment/{comment_id}");
        self.client.put(&path, &json!({ "body": body })).await
    }

    /// Worklog entries, oldest first. Cloud pages this endpoint; Server/DC
    /// returns everything, so the cap is applied here too.
    pub async fn get_worklog(&self, key: &str, max_results: u32) -> Result<Vec<WorklogEntry>> {
        let path = format!("/rest/api/2/issue/{key}/worklog");
        let max_results_param = max_results.to_string();
        let mut page: models::WorklogPage = self
            .client
            .get(&path, &[("maxResults", &max_results_param)])
            .await?;
        page.worklogs.truncate(max_results as usize);
        Ok(page.worklogs)
    }

    // ---- Batch and history -------------------------------------------------

    /// Creates several issues in one request. Each entry is a raw `fields`
    /// object, so custom fields pass straight through.
    pub async fn batch_create_issues(
        &self,
        issues: Vec<Map<String, Value>>,
    ) -> Result<BatchCreateResult> {
        let updates: Vec<Value> = issues
            .into_iter()
            .map(|fields| json!({ "fields": fields }))
            .collect();
        self.client
            .post(
                "/rest/api/2/issue/bulk",
                &json!({ "issueUpdates": updates }),
            )
            .await
    }

    /// Change history of one issue. Cloud has a dedicated paginated endpoint;
    /// Server/DC returns it via `expand=changelog` on the issue.
    pub async fn get_changelog(&self, key: &str, max_results: u32) -> Result<Vec<ChangelogEntry>> {
        if self.cloud {
            let path = format!("/rest/api/2/issue/{key}/changelog");
            let max_results = max_results.to_string();
            let page: models::ChangelogPage = self
                .client
                .get(&path, &[("maxResults", &max_results)])
                .await?;
            Ok(page.values)
        } else {
            let path = format!("/rest/api/2/issue/{key}");
            let issue: Value = self.client.get(&path, &[("expand", "changelog")]).await?;
            let mut page: models::ChangelogPage =
                serde_json::from_value(issue.get("changelog").cloned().unwrap_or(json!({})))
                    .map_err(|e| mcp_atlassian_client::Error::Decode(e.to_string()))?;
            // `expand=changelog` has no page size; cap here so both
            // deployments honour the argument the same way.
            page.histories.truncate(max_results as usize);
            Ok(page.histories)
        }
    }

    // ---- Project and board queries -----------------------------------------

    /// Convenience wrapper over JQL for "everything in this project".
    pub async fn get_project_issues(
        &self,
        project_key: &str,
        max_results: u32,
    ) -> Result<SearchPage> {
        self.search(&SearchParams {
            jql: format!("project = {} ORDER BY created DESC", quote(project_key)),
            max_results,
            ..Default::default()
        })
        .await
    }

    /// Issues on a board, optionally narrowed by JQL.
    pub async fn get_board_issues(
        &self,
        board_id: u64,
        jql: Option<&str>,
        max_results: u32,
        start_at: u32,
    ) -> Result<SearchPage> {
        let path = format!("/rest/agile/1.0/board/{board_id}/issue");
        let max_results = max_results.to_string();
        let start_at = start_at.to_string();
        let mut params: Vec<(&str, &str)> = vec![
            ("maxResults", &max_results),
            ("startAt", &start_at),
            ("fields", DEFAULT_SEARCH_FIELDS),
        ];
        if let Some(jql) = jql {
            params.push(("jql", jql));
        }
        let mut page: SearchPage = self.client.get(&path, &params).await?;
        for issue in &mut page.issues {
            prune_extra(issue, DEFAULT_SEARCH_FIELDS);
        }
        Ok(page)
    }

    /// Cloud references users by account id, Server/DC by username (D5/D6).
    fn user_ref(&self, user: &str) -> Value {
        if self.cloud {
            json!({ "accountId": user })
        } else {
            json!({ "name": user })
        }
    }
}

/// Keeps in `extra` only what the caller asked for by name (D35).
///
/// `#[serde(flatten)]` collects every field `IssueFields` does not model.
/// Jira answers a `fields` list with exactly those fields, so normally there
/// is nothing to prune — but `*all` / `*navigable` bring the whole schema,
/// most of it `null`, and the Agile endpoints add `sprint`, `epic` and friends
/// unasked. A null is dropped either way: it says nothing the absence of the
/// key does not.
fn prune_extra(issue: &mut Issue, requested: &str) {
    let requested: Vec<&str> = requested.split(',').map(str::trim).collect();
    let everything = requested.iter().any(|f| f.starts_with('*'));
    issue.fields.extra.retain(|name, value| {
        !value.is_null() && (everything || requested.contains(&name.as_str()))
    });
}

/// MCP tool state for Jira: the client the tools operate on.
///
/// Tools are inherent methods on this type (`#[tool_router]` requires that),
/// so they live in this crate next to the client they call. The MCP server
/// composes the resulting router onto its own state — see the `mcp-atlassian`
/// crate.
#[cfg(feature = "mcp")]
#[derive(Debug, Clone)]
pub struct JiraTools {
    client: std::sync::Arc<JiraClient>,
    files: mcp_atlassian_client::mcp::FileAccess,
}

#[cfg(feature = "mcp")]
impl JiraTools {
    pub fn new(
        client: std::sync::Arc<JiraClient>,
        files: mcp_atlassian_client::mcp::FileAccess,
    ) -> Self {
        Self { client, files }
    }

    pub(crate) fn client(&self) -> &JiraClient {
        &self.client
    }

    /// Where the attachment tools may read and write (D37).
    pub(crate) fn files(&self) -> &mcp_atlassian_client::mcp::FileAccess {
        &self.files
    }
}
