use atlassian_client::{Auth, ServiceConfig};
use atlassian_jira::{CreateIssueParams, JiraClient, SearchParams};
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn cloud_client(server: &MockServer) -> JiraClient {
    JiraClient::new(&ServiceConfig {
        base_url: server.uri(),
        auth: Auth::Basic {
            username: "u@example.com".into(),
            token: "secret".into(),
        },
        deployment: None,
    })
    .unwrap()
}

fn dc_client(server: &MockServer) -> JiraClient {
    JiraClient::new(&ServiceConfig {
        base_url: server.uri(),
        auth: Auth::Pat {
            token: "pat".into(),
        },
        deployment: None,
    })
    .unwrap()
}

fn issue_json(key: &str) -> serde_json::Value {
    json!({
        "id": "10001",
        "key": key,
        "fields": {
            "summary": "Fix login bug",
            "status": { "name": "In Progress" },
            "issuetype": { "name": "Bug" },
            "labels": ["auth"]
        }
    })
}

#[tokio::test]
async fn search_cloud_uses_jql_endpoint_with_token_pagination() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search/jql"))
        .and(query_param("jql", "project = PROJ"))
        .and(query_param("maxResults", "10"))
        .and(query_param("nextPageToken", "tok123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issues": [issue_json("PROJ-1")],
            "nextPageToken": "tok456",
            "isLast": false
        })))
        .expect(1)
        .mount(&server)
        .await;

    let page = cloud_client(&server)
        .search(&SearchParams {
            jql: "project = PROJ".into(),
            max_results: 10,
            next_page_token: Some("tok123".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(page.issues.len(), 1);
    assert_eq!(page.issues[0].key, "PROJ-1");
    assert_eq!(page.next_page_token.as_deref(), Some("tok456"));
}

#[tokio::test]
async fn search_dc_uses_legacy_endpoint_with_offset_pagination() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search"))
        .and(query_param("jql", "assignee = admin"))
        .and(query_param("startAt", "20"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issues": [issue_json("PROJ-2")],
            "total": 42,
            "startAt": 20
        })))
        .expect(1)
        .mount(&server)
        .await;

    let page = dc_client(&server)
        .search(&SearchParams {
            jql: "assignee = admin".into(),
            max_results: 10,
            start_at: Some(20),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(page.total, Some(42));
    assert_eq!(
        page.issues[0].fields.summary.as_deref(),
        Some("Fix login bug")
    );
}

#[tokio::test]
async fn create_issue_cloud_sends_account_id_assignee() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue"))
        .and(body_partial_json(json!({
            "fields": {
                "project": { "key": "PROJ" },
                "issuetype": { "name": "Task" },
                "summary": "Do the thing",
                "assignee": { "accountId": "acc-1" },
                "labels": ["ops"]
            }
        })))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({ "id": "10002", "key": "PROJ-3" })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let created = cloud_client(&server)
        .create_issue(&CreateIssueParams {
            project_key: "PROJ".into(),
            issue_type: "Task".into(),
            summary: "Do the thing".into(),
            assignee: Some("acc-1".into()),
            labels: vec!["ops".into()],
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(created.key, "PROJ-3");
}

#[tokio::test]
async fn create_issue_dc_sends_name_assignee() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue"))
        .and(body_partial_json(json!({
            "fields": { "assignee": { "name": "admin" } }
        })))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({ "id": "1", "key": "PROJ-4" })),
        )
        .expect(1)
        .mount(&server)
        .await;

    dc_client(&server)
        .create_issue(&CreateIssueParams {
            project_key: "PROJ".into(),
            issue_type: "Task".into(),
            summary: "s".into(),
            assignee: Some("admin".into()),
            ..Default::default()
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn update_issue_puts_fields_and_accepts_204() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/2/issue/PROJ-1"))
        .and(body_partial_json(json!({
            "fields": { "summary": "New title" }
        })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let mut fields = serde_json::Map::new();
    fields.insert("summary".into(), json!("New title"));
    cloud_client(&server)
        .update_issue("PROJ-1", &fields)
        .await
        .unwrap();
}

#[tokio::test]
async fn transition_issue_posts_transition_with_comment() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue/PROJ-1/transitions"))
        .and(body_partial_json(json!({
            "transition": { "id": "31" },
            "update": { "comment": [{ "add": { "body": "moving on" } }] }
        })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    cloud_client(&server)
        .transition_issue("PROJ-1", "31", Some("moving on"))
        .await
        .unwrap();
}

#[tokio::test]
async fn get_transitions_unwraps_envelope() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-1/transitions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transitions": [
                { "id": "31", "name": "Done", "to": { "name": "Done" } }
            ]
        })))
        .mount(&server)
        .await;

    let transitions = cloud_client(&server)
        .get_transitions("PROJ-1")
        .await
        .unwrap();
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].to.name, "Done");
}

#[tokio::test]
async fn not_found_error_names_the_issue() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-404"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "errorMessages": ["Issue does not exist or you do not have permission to see it."]
        })))
        .mount(&server)
        .await;

    let err = cloud_client(&server)
        .get_issue("PROJ-404", None)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("PROJ-404"),
        "message should name the issue: {msg}"
    );
    assert!(
        msg.contains("does not exist"),
        "message should carry API text: {msg}"
    );
}

#[tokio::test]
async fn projects_route_by_deployment() {
    // Cloud: paginated /project/search with {values}
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/project/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "values": [{ "id": "1", "key": "PROJ", "name": "Project" }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let projects = cloud_client(&server).get_projects().await.unwrap();
    assert_eq!(projects[0].key, "PROJ");

    // Server/DC: plain array from /project
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/project"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": "2", "key": "OPS", "name": "Operations" }
        ])))
        .expect(1)
        .mount(&server)
        .await;
    let projects = dc_client(&server).get_projects().await.unwrap();
    assert_eq!(projects[0].key, "OPS");
}

/// Wiremock catch-all that fails loudly on unexpected requests — guards the
/// deployment routing (cloud client must never hit DC endpoints and vice versa).
#[tokio::test]
async fn cloud_search_never_hits_legacy_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search"))
        .respond_with(move |_: &Request| -> ResponseTemplate {
            panic!("cloud client must not call legacy /rest/api/2/search")
        })
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search/jql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "issues": [] })))
        .mount(&server)
        .await;

    cloud_client(&server)
        .search(&SearchParams {
            jql: "x = y".into(),
            max_results: 5,
            ..Default::default()
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn search_users_cloud_uses_query_param() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/user/search"))
        .and(query_param("query", "alice"))
        .and(query_param("maxResults", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "accountId": "acc-1",
            "displayName": "Alice Smith",
            "emailAddress": "alice@example.com",
            "active": true
        }])))
        .expect(1)
        .mount(&server)
        .await;

    let users = cloud_client(&server)
        .search_users("alice", 10)
        .await
        .unwrap();
    assert_eq!(users[0].account_id.as_deref(), Some("acc-1"));
    assert_eq!(users[0].display_name, "Alice Smith");
    assert_eq!(users[0].active, Some(true));
}

#[tokio::test]
async fn search_users_dc_uses_username_param() {
    let server = MockServer::start().await;
    // A `query` param here would mean the Cloud path leaked into Server/DC.
    Mock::given(method("GET"))
        .and(path("/rest/api/2/user/search"))
        .and(query_param("username", "alice"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "name": "alice",
            "displayName": "Alice Smith"
        }])))
        .expect(1)
        .mount(&server)
        .await;

    let users = dc_client(&server).search_users("alice", 10).await.unwrap();
    assert_eq!(users[0].name.as_deref(), Some("alice"));
    // Privacy-restricted instances omit the email entirely.
    assert_eq!(users[0].email_address, None);
}

#[tokio::test]
async fn get_issue_requests_the_standard_set_and_models_structure() {
    // Omitting `fields` must not mean "everything": that drags in every
    // custom field of the instance. And what comes back must carry the
    // structure a plan needs — parent, subtasks, links with ids.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-1"))
        .and(query_param("fields", atlassian_jira::DEFAULT_ISSUE_FIELDS))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "10001", "key": "PROJ-1",
            "fields": {
                "summary": "Child",
                "parent": { "key": "PROJ-9", "fields": { "summary": "Epic", "status": { "name": "Open" } } },
                "subtasks": [{ "key": "PROJ-2", "fields": { "summary": "Sub" } }],
                "issuelinks": [{
                    "id": "10500",
                    "type": { "name": "Blocks", "inward": "is blocked by", "outward": "blocks" },
                    "outwardIssue": { "key": "PROJ-3", "fields": { "summary": "Blocked one" } }
                }],
                "components": [{ "name": "api" }],
                "fixVersions": [{ "name": "1.2" }],
                "resolution": null,
                "duedate": "2026-10-01"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let issue = cloud_client(&server)
        .get_issue("PROJ-1", None)
        .await
        .unwrap();
    let fields = issue.fields;
    assert_eq!(
        fields.parent.as_ref().map(|p| p.key.as_str()),
        Some("PROJ-9")
    );
    assert_eq!(fields.subtasks[0].key, "PROJ-2");
    assert_eq!(fields.issuelinks[0].id, "10500");
    assert_eq!(fields.issuelinks[0].link_type.outward, "blocks");
    assert_eq!(
        fields.issuelinks[0]
            .outward_issue
            .as_ref()
            .map(|i| i.key.as_str()),
        Some("PROJ-3")
    );
    assert_eq!(fields.components[0].name, "api");
    assert_eq!(fields.fix_versions[0].name, "1.2");
    assert_eq!(fields.duedate.as_deref(), Some("2026-10-01"));
    assert!(fields.extra.is_empty(), "{:?}", fields.extra);
}

#[tokio::test]
async fn custom_fields_are_kept_only_when_asked_for_and_not_when_null() {
    let server = MockServer::start().await;
    let body = json!({
        "id": "10001", "key": "PROJ-1",
        "fields": {
            "summary": "s",
            "customfield_10011": 5,
            "customfield_10012": null,
            "customfield_10013": "unrequested"
        }
    });
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let jira = cloud_client(&server);

    // Named: exactly that field, even though the mock returned more.
    let issue = jira
        .get_issue("PROJ-1", Some("summary,customfield_10011"))
        .await
        .unwrap();
    assert_eq!(issue.fields.extra.get("customfield_10011"), Some(&json!(5)));
    assert!(!issue.fields.extra.contains_key("customfield_10013"));

    // `*all`: every non-null field the instance returned.
    let issue = jira.get_issue("PROJ-1", Some("*all")).await.unwrap();
    assert_eq!(issue.fields.extra.len(), 2, "{:?}", issue.fields.extra);
    assert!(!issue.fields.extra.contains_key("customfield_10012"));

    // Serialized, custom fields sit at the top level of `fields`, as in Jira.
    let value = serde_json::to_value(&issue).unwrap();
    assert_eq!(value["fields"]["customfield_10011"], 5);
}

#[tokio::test]
async fn the_other_deployments_paging_parameter_is_refused_not_ignored() {
    let server = MockServer::start().await;
    let error = cloud_client(&server)
        .search(&SearchParams {
            jql: "x = y".into(),
            max_results: 5,
            start_at: Some(10),
            ..Default::default()
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("next_page_token"), "{error}");

    let error = dc_client(&server)
        .search(&SearchParams {
            jql: "x = y".into(),
            max_results: 5,
            next_page_token: Some("tok".into()),
            ..Default::default()
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("start_at"), "{error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn an_explicit_deployment_overrides_the_auth_mode_inference() {
    // Data Center behind Basic auth (D41): Basic alone would mean Cloud and
    // route search to `/search/jql`, which DC does not have.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search"))
        .and(query_param("startAt", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "issues": [], "total": 0 })))
        .expect(1)
        .mount(&server)
        .await;
    let jira = JiraClient::new(&ServiceConfig {
        base_url: server.uri(),
        auth: Auth::Basic {
            username: "u".into(),
            token: "t".into(),
        },
        deployment: Some(atlassian_client::Deployment::Server),
    })
    .unwrap();
    jira.search(&SearchParams {
        jql: "x = y".into(),
        max_results: 5,
        ..Default::default()
    })
    .await
    .unwrap();
}
