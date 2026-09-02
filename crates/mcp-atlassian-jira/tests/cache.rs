//! `TtlCache` on the Jira client: reference data is reused, work data is not
//! (D25). Expectations are verified when the mock server drops.

use std::time::Duration;

use mcp_atlassian_client::{Auth, ServiceConfig};
use mcp_atlassian_jira::JiraClient;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> JiraClient {
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

fn cached_client(server: &MockServer) -> JiraClient {
    client(server).with_cache(Duration::from_secs(300))
}

async fn mount_projects(server: &MockServer, times: u64) {
    Mock::given(method("GET"))
        .and(path("/rest/api/2/project/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "values": [{ "id": "1", "key": "PROJ", "name": "Project" }]
        })))
        .expect(times)
        .mount(server)
        .await;
}

#[tokio::test]
async fn reference_data_is_fetched_once() {
    let server = MockServer::start().await;
    mount_projects(&server, 1).await;

    let client = cached_client(&server);
    let first = client.get_projects().await.unwrap();
    let second = client.get_projects().await.unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(second[0].key, "PROJ");
}

#[tokio::test]
async fn without_a_ttl_every_call_hits_the_api() {
    let server = MockServer::start().await;
    mount_projects(&server, 2).await;

    let client = client(&server);
    client.get_projects().await.unwrap();
    client.get_projects().await.unwrap();
}

#[tokio::test]
async fn issues_are_never_cached() {
    // The whole point of the opt-in: an issue read must never be stale.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "10001",
            "key": "PROJ-1",
            "fields": { "summary": "Fix login bug" }
        })))
        .expect(2)
        .mount(&server)
        .await;

    let client = cached_client(&server);
    client.get_issue("PROJ-1", None).await.unwrap();
    client.get_issue("PROJ-1", None).await.unwrap();
}

#[tokio::test]
async fn board_filters_are_part_of_the_cache_key() {
    let server = MockServer::start().await;
    for project in ["PROJ", "OPS"] {
        Mock::given(method("GET"))
            .and(path("/rest/agile/1.0/board"))
            .and(query_param("projectKeyOrId", project))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "values": [{ "id": 1, "name": format!("{project} board") }],
                "isLast": true
            })))
            .expect(1)
            .mount(&server)
            .await;
    }

    let client = cached_client(&server);
    client.get_boards(Some("PROJ"), 50).await.unwrap();
    client.get_boards(Some("OPS"), 50).await.unwrap();
    // Same arguments again: served from the cache, so the counts above hold.
    client.get_boards(Some("PROJ"), 50).await.unwrap();
}

#[tokio::test]
async fn one_cached_field_list_serves_every_query() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/field"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": "summary", "name": "Summary", "custom": false },
            { "id": "customfield_10011", "name": "Story Points", "custom": true }
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let client = cached_client(&server);
    assert_eq!(client.search_fields(None).await.unwrap().len(), 2);
    // The filter runs client-side, so a narrower query needs no second fetch.
    let points = client.search_fields(Some("story")).await.unwrap();
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].id, "customfield_10011");
}

#[tokio::test]
async fn a_failed_fetch_is_not_remembered() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issuetype"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issuetype"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{ "id": "1", "name": "Task", "subtask": false }])),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = cached_client(&server);
    assert!(client.get_issue_types().await.is_err());
    let types = client.get_issue_types().await.unwrap();
    assert_eq!(types[0].name, "Task");
}
