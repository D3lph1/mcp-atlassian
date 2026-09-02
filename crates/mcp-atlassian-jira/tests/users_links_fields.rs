use mcp_atlassian_client::{Auth, ServiceConfig};
use mcp_atlassian_jira::{FieldOptionsScope, JiraClient};
use serde_json::json;
use wiremock::matchers::{body_json, body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn cloud(server: &MockServer) -> JiraClient {
    JiraClient::new(&ServiceConfig {
        base_url: server.uri(),
        auth: Auth::Basic {
            username: "u@example.com".into(),
            token: "t".into(),
        },
        deployment: None,
    })
    .unwrap()
}

fn dc(server: &MockServer) -> JiraClient {
    JiraClient::new(&ServiceConfig {
        base_url: server.uri(),
        auth: Auth::Pat { token: "p".into() },
        deployment: None,
    })
    .unwrap()
}

#[tokio::test]
async fn user_profile_param_differs_by_deployment() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/user"))
        .and(query_param("accountId", "acc-1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "accountId": "acc-1", "displayName": "Alice" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        cloud(&server)
            .get_user_profile("acc-1")
            .await
            .unwrap()
            .display_name,
        "Alice"
    );

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/user"))
        .and(query_param("username", "alice"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "name": "alice", "displayName": "Alice" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        dc(&server)
            .get_user_profile("alice")
            .await
            .unwrap()
            .name
            .as_deref(),
        Some("alice")
    );
}

#[tokio::test]
async fn assignable_search_prefers_issue_over_project() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/user/assignable/search"))
        .and(query_param("issueKey", "PROJ-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;

    cloud(&server)
        .search_assignable_users("a", Some("PROJ"), Some("PROJ-1"), 10)
        .await
        .unwrap();
}

#[tokio::test]
async fn assign_issue_uses_deployment_user_shape_and_unassigns_with_null() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/2/issue/PROJ-1/assignee"))
        .and(body_json(json!({ "accountId": "acc-1" })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    cloud(&server)
        .assign_issue("PROJ-1", Some("acc-1"))
        .await
        .unwrap();

    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/2/issue/PROJ-1/assignee"))
        .and(body_json(json!({ "name": null })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    dc(&server).assign_issue("PROJ-1", None).await.unwrap();
}

#[tokio::test]
async fn watchers_post_a_bare_string_body() {
    let server = MockServer::start().await;
    // The watcher endpoint is unusual: the body is a bare JSON string.
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue/PROJ-1/watchers"))
        .and(body_json(json!("acc-1")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    cloud(&server).add_watcher("PROJ-1", "acc-1").await.unwrap();
}

#[tokio::test]
async fn search_fields_filters_client_side_on_name_and_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/field"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": "summary", "name": "Summary", "custom": false },
            { "id": "customfield_10011", "name": "Story Points", "custom": true },
            { "id": "customfield_10020", "name": "Sprint", "custom": true }
        ])))
        .mount(&server)
        .await;

    let jira = cloud(&server);
    let by_name = jira.search_fields(Some("story")).await.unwrap();
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].id, "customfield_10011");

    let by_id = jira.search_fields(Some("customfield_10020")).await.unwrap();
    assert_eq!(by_id[0].name, "Sprint");

    assert_eq!(jira.search_fields(None).await.unwrap().len(), 3);
}

#[tokio::test]
async fn issue_link_carries_type_and_direction() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issueLink"))
        .and(body_partial_json(json!({
            "type": { "name": "Blocks" },
            "inwardIssue": { "key": "PROJ-2" },
            "outwardIssue": { "key": "PROJ-1" },
            "comment": { "body": "ordering" }
        })))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;

    cloud(&server)
        .create_issue_link("Blocks", "PROJ-2", "PROJ-1", Some("ordering"))
        .await
        .unwrap();
}

#[tokio::test]
async fn link_to_epic_uses_parent_on_cloud() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/2/issue/PROJ-5"))
        .and(body_partial_json(json!({
            "fields": { "parent": { "key": "PROJ-1" } }
        })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    cloud(&server)
        .link_to_epic("PROJ-5", "PROJ-1")
        .await
        .unwrap();
}

#[tokio::test]
async fn link_to_epic_resolves_epic_link_field_on_dc() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/field"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": "customfield_10014", "name": "Epic Link", "custom": true }
        ])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/2/issue/PROJ-5"))
        .and(body_partial_json(json!({
            "fields": { "customfield_10014": "PROJ-1" }
        })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    dc(&server).link_to_epic("PROJ-5", "PROJ-1").await.unwrap();
}

#[tokio::test]
async fn changelog_reads_dedicated_endpoint_on_cloud_and_expand_on_dc() {
    let entry = json!({
        "id": "1",
        "created": "2026-01-01T00:00:00.000+0000",
        "items": [{ "field": "status", "fromString": "To Do", "toString": "Done" }]
    });

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-1/changelog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "values": [entry] })))
        .expect(1)
        .mount(&server)
        .await;
    let entries = cloud(&server).get_changelog("PROJ-1", 25).await.unwrap();
    assert_eq!(entries[0].items[0].field, "status");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-1"))
        .and(query_param("expand", "changelog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "1", "key": "PROJ-1",
            "changelog": { "histories": [entry, entry, entry] }
        })))
        .expect(1)
        .mount(&server)
        .await;
    // `expand=changelog` has no page size, so the cap is applied client-side.
    let entries = dc(&server).get_changelog("PROJ-1", 2).await.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].items[0].to_string.as_deref(), Some("Done"));
}

#[tokio::test]
async fn project_issues_quote_the_project_key() {
    // A key with a quote in it stays one string literal, not two clauses.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search/jql"))
        .and(query_param(
            "jql",
            "project = \"PR\\\"OJ\" ORDER BY created DESC",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "issues": [] })))
        .expect(1)
        .mount(&server)
        .await;
    cloud(&server)
        .get_project_issues("PR\"OJ", 10)
        .await
        .unwrap();
}

#[tokio::test]
async fn batch_create_wraps_each_entry_in_fields() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue/bulk"))
        .and(body_partial_json(json!({
            "issueUpdates": [
                { "fields": { "summary": "one" } },
                { "fields": { "summary": "two" } }
            ]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "issues": [{ "id": "1", "key": "PROJ-1" }, { "id": "2", "key": "PROJ-2" }],
            "errors": []
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut a = serde_json::Map::new();
    a.insert("summary".into(), json!("one"));
    let mut b = serde_json::Map::new();
    b.insert("summary".into(), json!("two"));

    let result = cloud(&server)
        .batch_create_issues(vec![a, b])
        .await
        .unwrap();
    assert_eq!(result.issues.len(), 2);
    assert!(result.errors.is_empty());
}

#[tokio::test]
async fn comments_are_newest_first_even_where_the_server_ignores_order_by() {
    // Server/DC answers oldest first regardless of `orderBy`.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-1/comment"))
        .and(query_param("startAt", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "comments": [
                { "id": "1", "body": "old", "created": "2026-01-01T00:00:00.000+0000" },
                { "id": "2", "body": "new", "created": "2026-02-01T00:00:00.000+0000" }
            ],
            "total": 2, "startAt": 0
        })))
        .mount(&server)
        .await;
    let page = dc(&server).get_comments("PROJ-1", 10, 0).await.unwrap();
    assert_eq!(page.comments[0].body, "new");
    assert_eq!(page.total, 2);
}

#[tokio::test]
async fn worklog_is_capped_even_where_the_server_returns_everything() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-1/worklog"))
        .and(query_param("maxResults", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "worklogs": [
                { "id": "1", "timeSpent": "1h" },
                { "id": "2", "timeSpent": "2h" },
                { "id": "3", "timeSpent": "3h" }
            ]
        })))
        .mount(&server)
        .await;
    let entries = dc(&server).get_worklog("PROJ-1", 2).await.unwrap();
    assert_eq!(entries.len(), 2);
}

#[tokio::test]
async fn remote_links_are_typed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue/PROJ-1/remotelink"))
        .and(body_partial_json(
            json!({ "object": { "url": "https://x", "title": "X" } }),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": 10000, "self": "https://jira/rest/api/2/issue/PROJ-1/remotelink/10000"
        })))
        .mount(&server)
        .await;
    let link = cloud(&server)
        .create_remote_issue_link("PROJ-1", "https://x", "X", None)
        .await
        .unwrap();
    assert_eq!(link.id, 10000);
    assert!(link.self_url.ends_with("/10000"));
}

#[tokio::test]
async fn field_options_come_from_the_edit_screen_when_an_issue_is_named() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-1/editmeta"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "fields": {
                "customfield_10011": {
                    "name": "Team",
                    "allowedValues": [
                        { "id": "1", "value": "Platform" },
                        { "id": "2", "value": "Mobile" },
                        { "id": "3", "value": "Web" }
                    ]
                },
                "priority": { "allowedValues": [{ "id": "9", "name": "High" }] }
            }
        })))
        .mount(&server)
        .await;
    let jira = dc(&server);
    let scope = FieldOptionsScope {
        issue_key: Some("PROJ-1"),
        ..Default::default()
    };

    let options = jira
        .get_field_options("customfield_10011", scope, 2)
        .await
        .unwrap();
    assert_eq!(options.len(), 2);
    assert_eq!(options[0].value, "Platform");
    // `name`-shaped values land in `value` too.
    let options = jira.get_field_options("priority", scope, 50).await.unwrap();
    assert_eq!(options[0].value, "High");

    let error = jira
        .get_field_options("customfield_99", scope, 50)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("customfield_99"), "{error}");
    assert!(error.contains("PROJ-1"), "{error}");
}

#[tokio::test]
async fn field_options_come_from_the_create_screen_when_a_project_is_named() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/createmeta/PROJ/issuetypes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "values": [
                { "id": "10001", "name": "Task" },
                { "id": "10004", "name": "Bug" }
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/createmeta/PROJ/issuetypes/10004"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "values": [
                { "fieldId": "summary", "name": "Summary" },
                { "fieldId": "customfield_10011", "allowedValues": [{ "id": "1", "value": "Platform" }] }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let options = cloud(&server)
        .get_field_options(
            "customfield_10011",
            FieldOptionsScope {
                project_key: Some("PROJ"),
                issue_type: Some("bug"),
                ..Default::default()
            },
            50,
        )
        .await
        .unwrap();
    assert_eq!(options[0].value, "Platform");

    let error = cloud(&server)
        .get_field_options(
            "customfield_10011",
            FieldOptionsScope {
                project_key: Some("PROJ"),
                issue_type: Some("Epic"),
                ..Default::default()
            },
            50,
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("Task, Bug"), "{error}");
}

#[tokio::test]
async fn field_options_without_a_scope_use_the_context_api_on_cloud_only() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/field/customfield_10011/context"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "values": [{ "id": "10100", "name": "Default" }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/rest/api/2/field/customfield_10011/context/10100/option",
        ))
        .and(query_param("maxResults", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "values": [{ "id": "1", "value": "Platform" }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let options = cloud(&server)
        .get_field_options("customfield_10011", FieldOptionsScope::default(), 50)
        .await
        .unwrap();
    assert_eq!(options[0].value, "Platform");

    // Server/DC has no such API; the error says what to pass instead.
    let error = dc(&server)
        .get_field_options("customfield_10011", FieldOptionsScope::default(), 50)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("issue_key"), "{error}");
    assert!(error.contains("project_key"), "{error}");
}
