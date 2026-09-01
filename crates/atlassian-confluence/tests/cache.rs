//! `TtlCache` on the Confluence client: spaces are reference data, pages are
//! not (D25).

use std::time::Duration;

use atlassian_client::{Auth, ServiceConfig};
use atlassian_confluence::ConfluenceClient;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> ConfluenceClient {
    ConfluenceClient::new(&ServiceConfig {
        base_url: server.uri(),
        auth: Auth::Basic {
            username: "u@example.com".into(),
            token: "t".into(),
        },
    })
    .unwrap()
}

async fn mount_spaces(server: &MockServer, times: u64) {
    Mock::given(method("GET"))
        .and(path("/rest/api/space"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{ "id": 1, "key": "DEV", "name": "Development" }]
        })))
        .expect(times)
        .mount(server)
        .await;
}

#[tokio::test]
async fn spaces_are_fetched_once_with_a_ttl() {
    let server = MockServer::start().await;
    mount_spaces(&server, 1).await;

    let client = client(&server).with_cache(Duration::from_secs(300));
    let spaces = client.get_spaces(25).await.unwrap();
    assert_eq!(spaces.results[0].key, "DEV");
    client.get_spaces(25).await.unwrap();
}

#[tokio::test]
async fn the_limit_is_part_of_the_cache_key() {
    // A wider page is a different answer, not a cache hit on a narrower one.
    let server = MockServer::start().await;
    mount_spaces(&server, 2).await;

    let client = client(&server).with_cache(Duration::from_secs(300));
    client.get_spaces(25).await.unwrap();
    client.get_spaces(50).await.unwrap();
}

#[tokio::test]
async fn without_a_ttl_spaces_are_refetched() {
    let server = MockServer::start().await;
    mount_spaces(&server, 2).await;

    let client = client(&server);
    client.get_spaces(25).await.unwrap();
    client.get_spaces(25).await.unwrap();
}

#[tokio::test]
async fn page_content_is_never_cached() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/content/123456"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "123456",
            "type": "page",
            "title": "Runbook",
            "body": { "storage": { "value": "<p>text</p>" } }
        })))
        .expect(2)
        .mount(&server)
        .await;

    let client = client(&server).with_cache(Duration::from_secs(300));
    client.get_page("123456").await.unwrap();
    client.get_page("123456").await.unwrap();
}
