//! `cache_ttl` reaches the clients the server builds — the wiring the
//! per-crate cache tests cannot see.

use std::time::Duration;

use atlassian_client::{Auth, Config, ServiceConfig};
use mcp_atlassian::server::AtlassianServer;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config(mock: &MockServer, cache_ttl: Option<Duration>) -> Config {
    Config {
        jira: Some(ServiceConfig {
            base_url: mock.uri(),
            auth: Auth::Basic {
                username: "u@example.com".into(),
                token: "t".into(),
            },
        }),
        confluence: None,
        enabled_tools: None,
        read_only: false,
        dry_run: false,
        audit_log: None,
        cache_ttl,
    }
}

async fn connect(config: Config) -> RunningService<RoleClient, ()> {
    let server = AtlassianServer::new(&config).unwrap();
    let (client_io, server_io) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    ().serve(client_io).await.unwrap()
}

async fn mount_issue_types(mock: &MockServer, times: u64) {
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issuetype"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{ "id": "1", "name": "Task", "subtask": false }])),
        )
        .expect(times)
        .mount(mock)
        .await;
}

async fn call_twice(client: &RunningService<RoleClient, ()>) {
    for _ in 0..2 {
        client
            .call_tool(CallToolRequestParams::new("jira_get_issue_types"))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn a_configured_ttl_makes_reference_tools_reuse_one_fetch() {
    let mock = MockServer::start().await;
    mount_issue_types(&mock, 1).await;

    let client = connect(config(&mock, Some(Duration::from_secs(300)))).await;
    call_twice(&client).await;
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn caching_is_off_by_default() {
    let mock = MockServer::start().await;
    mount_issue_types(&mock, 2).await;

    let client = connect(config(&mock, None)).await;
    call_twice(&client).await;
    client.cancel().await.unwrap();
}
