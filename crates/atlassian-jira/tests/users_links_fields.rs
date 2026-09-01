use atlassian_client::{Auth, ServiceConfig};
use atlassian_jira::JiraClient;
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
    })
    .unwrap()
}

fn dc(server: &MockServer) -> JiraClient {
    JiraClient::new(&ServiceConfig {
        base_url: server.uri(),
        auth: Auth::Pat { token: "p".into() },
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
            "changelog": { "histories": [entry] }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let entries = dc(&server).get_changelog("PROJ-1", 25).await.unwrap();
    assert_eq!(entries[0].items[0].to_string.as_deref(), Some("Done"));
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
