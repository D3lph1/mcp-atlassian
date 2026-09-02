//! Guards that hold for every tool rather than for one: a request path cannot
//! be steered somewhere else by an interpolated identifier, and a page size
//! cannot be inflated past the cap.

use atlassian_client::{Auth, Config, ServiceConfig};
use mcp_atlassian::server::AtlassianServer;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config(mock: &MockServer) -> Config {
    let service = ServiceConfig {
        base_url: mock.uri(),
        auth: Auth::Basic {
            username: "u@example.com".into(),
            token: "t".into(),
        },
        deployment: None,
    };
    Config {
        jira: Some(service.clone()),
        confluence: Some(service),
        ..Config::default()
    }
}

async fn connect(mock: &MockServer) -> RunningService<RoleClient, ()> {
    let server = AtlassianServer::new(&config(mock)).unwrap();
    let (client_io, server_io) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    ().serve(client_io).await.unwrap()
}

fn args(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

#[tokio::test]
async fn an_issue_key_cannot_steer_the_request_to_another_endpoint() {
    // The key reaches the URL through `format!`, and `Url::join` normalizes
    // `..` — so without a guard this would call /rest/api/2/myself with the
    // user's credentials instead of fetching an issue.
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/myself"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "displayName": "hacked" })))
        .mount(&mock)
        .await;
    let client = connect(&mock).await;

    for key in [
        "../../../rest/api/2/myself",
        "PROJ-1?expand=changelog",
        "PROJ-1#fragment",
    ] {
        let error = client
            .call_tool(
                CallToolRequestParams::new("jira_get_issue")
                    .with_arguments(args(json!({ "issue_key": key }))),
            )
            .await
            .expect_err("a crafted issue key was accepted");
        assert!(error.to_string().contains("issue key"), "{key}: {error}");
    }

    assert!(
        mock.received_requests().await.unwrap().is_empty(),
        "a crafted key reached the network"
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_page_id_is_guarded_the_same_way() {
    let mock = MockServer::start().await;
    let client = connect(&mock).await;

    let error = client
        .call_tool(
            CallToolRequestParams::new("confluence_get_page")
                .with_arguments(args(json!({ "page_id": "../../../rest/api/user/current" }))),
        )
        .await
        .expect_err("a crafted page id was accepted");
    assert!(error.to_string().contains("page id"), "{error}");
    assert!(mock.received_requests().await.unwrap().is_empty());
    client.cancel().await.unwrap();
}

/// The `limit` a tool sends on, read back off the wire.
async fn limit_sent(mock: &MockServer) -> String {
    let requests = mock.received_requests().await.unwrap();
    let request = requests.last().expect("no request reached the API");
    request
        .url
        .query_pairs()
        .find(|(k, _)| k == "limit" || k == "maxResults")
        .map(|(_, v)| v.to_string())
        .expect("no limit in the query")
}

#[tokio::test]
async fn an_inflated_limit_is_capped_before_it_reaches_atlassian() {
    // confluence_get_page_children took its limit straight from the caller;
    // 100000 would have been passed through and flooded the context.
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/content/123/child/page"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": [] })))
        .mount(&mock)
        .await;
    let client = connect(&mock).await;

    client
        .call_tool(
            CallToolRequestParams::new("confluence_get_page_children")
                .with_arguments(args(json!({ "page_id": "123", "limit": 100_000 }))),
        )
        .await
        .unwrap();

    assert_eq!(limit_sent(&mock).await, "50");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_jira_limit_is_capped_too() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-1/comment"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "comments": [] })))
        .mount(&mock)
        .await;
    let client = connect(&mock).await;

    client
        .call_tool(
            CallToolRequestParams::new("jira_get_comments")
                .with_arguments(args(json!({ "issue_key": "PROJ-1", "max_results": 9999 }))),
        )
        .await
        .unwrap();

    assert_eq!(limit_sent(&mock).await, "50");
    client.cancel().await.unwrap();
}
