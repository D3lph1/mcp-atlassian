//! `completion/complete` for `issue_key` (D44): project keys, then the
//! project's recent issues.

use atlassian_client::{Auth, Config, ServiceConfig};
use mcp_atlassian::server::AtlassianServer;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config(mock: &MockServer) -> Config {
    Config {
        jira: Some(ServiceConfig {
            base_url: mock.uri(),
            auth: Auth::Basic {
                username: "u@example.com".into(),
                token: "t".into(),
            },
            deployment: None,
        }),
        ..Config::default()
    }
}

async fn connect(mock: &MockServer) -> RunningService<RoleClient, ()> {
    Mock::given(method("GET"))
        .and(path("/rest/api/2/project/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "values": [
                { "id": "1", "key": "PROJ", "name": "Project" },
                { "id": "2", "key": "OPS", "name": "Operations" }
            ]
        })))
        .mount(mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search/jql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issues": [
                { "id": "1", "key": "PROJ-12", "fields": {} },
                { "id": "2", "key": "PROJ-1", "fields": {} },
                { "id": "3", "key": "PROJ-7", "fields": {} }
            ]
        })))
        .mount(mock)
        .await;
    let server = AtlassianServer::new(&config(mock)).unwrap();
    let (client_io, server_io) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    ().serve(client_io).await.unwrap()
}

#[tokio::test]
async fn the_server_advertises_completions() {
    let mock = MockServer::start().await;
    let client = connect(&mock).await;
    let info = client.peer_info().expect("initialized");
    assert!(info.capabilities.completions.is_some());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn an_issue_key_completes_from_projects_then_from_the_projects_issues() {
    let mock = MockServer::start().await;
    let client = connect(&mock).await;

    // Before the dash: project keys, case-insensitively.
    let keys = client
        .complete_prompt_argument("jira_issue", "issue_key", "pr", None)
        .await
        .unwrap();
    assert_eq!(keys.values, ["PROJ-"]);

    // After it: the project's issues that match what was typed so far.
    let keys = client
        .complete_resource_argument("jira://{issue_key}", "issue_key", "PROJ-1", None)
        .await
        .unwrap();
    assert_eq!(keys.values, ["PROJ-12", "PROJ-1"]);

    // Arguments with nothing to offer answer with nothing, not an error.
    let none = client
        .complete_prompt_argument("jira_triage", "project_key", "P", None)
        .await
        .unwrap();
    assert!(none.values.is_empty());
    client.cancel().await.unwrap();
}
