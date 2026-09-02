//! The streamable HTTP transport (D18) behind a bearer token (D39), over a
//! real socket. Only built with `--features http`.
#![cfg(feature = "http")]

use atlassian_client::{Auth, Config, ServiceConfig};
use mcp_atlassian::http::router;
use mcp_atlassian::server::AtlassianServer;
use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;
use serde_json::json;
use wiremock::MockServer;

/// Serves the router on an ephemeral loopback port; returns its base URL.
async fn start(mock: &MockServer, token: Option<&str>) -> String {
    let config = Config {
        jira: Some(ServiceConfig {
            base_url: mock.uri(),
            auth: Auth::Pat { token: "t".into() },
            deployment: None,
        }),
        ..Config::default()
    };
    let server = AtlassianServer::new(&config).unwrap();
    let app = router(
        server,
        StreamableHttpServerConfig::default(),
        token.map(String::from),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn initialize() -> serde_json::Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0" }
        }
    })
}

fn post(base: &str) -> reqwest::RequestBuilder {
    reqwest::Client::new()
        .post(format!("{base}/mcp"))
        .header("Accept", "application/json, text/event-stream")
        .json(&initialize())
}

#[tokio::test]
async fn healthz_answers_without_a_token_and_without_touching_atlassian() {
    let mock = MockServer::start().await;
    let base = start(&mock, Some("s3cret")).await;
    let response = reqwest::get(format!("{base}/healthz")).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), "ok");
    assert!(mock.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn mcp_requires_the_bearer_token_when_one_is_configured() {
    let mock = MockServer::start().await;
    let base = start(&mock, Some("s3cret")).await;

    let response = post(&base).send().await.unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(
        response.headers().get("www-authenticate").unwrap(),
        "Bearer"
    );
    let response = post(&base).bearer_auth("wrong").send().await.unwrap();
    assert_eq!(response.status(), 401);
    // Without the token nothing reached the protocol layer.
    let response = post(&base).bearer_auth("s3cret").send().await.unwrap();
    assert_eq!(response.status(), 200);
    let body = response.text().await.unwrap();
    assert!(body.contains("mcp-atlassian"), "{body}");
}

#[tokio::test]
async fn without_a_configured_token_mcp_is_open() {
    let mock = MockServer::start().await;
    let base = start(&mock, None).await;
    let response = post(&base).send().await.unwrap();
    assert_eq!(response.status(), 200);
}
