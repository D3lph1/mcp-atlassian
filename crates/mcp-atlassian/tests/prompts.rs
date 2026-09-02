//! Prompts, end to end over an in-memory transport: `prompts/list` advertises
//! them, `prompts/get` fetches the issue and returns a briefing, and a product
//! that lost its tools loses its prompts with them.

use atlassian_client::{Auth, Config, ServiceConfig, ToolFilter};
use mcp_atlassian::server::AtlassianServer;
use rmcp::model::GetPromptRequestParams;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
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
        confluence: None,
        ..Config::default()
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

fn args(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

/// An instance holding one issue with one comment.
async fn mock_issue() -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "10001",
            "key": "PROJ-123",
            "fields": {
                "summary": "Search times out on large projects",
                "description": "JQL search returns 504 above ~50k issues.",
                "status": { "name": "In Progress" },
                "priority": { "name": "High" },
                "issuetype": { "name": "Bug" },
                "assignee": { "displayName": "Jane Doe" },
                "labels": ["backend"]
            }
        })))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-123/comment"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "comments": [{
                "id": "1",
                "author": { "displayName": "John Smith" },
                "body": "Reproduced on staging.",
                "created": "2026-09-01T12:00:00.000+0000"
            }],
            "total": 1
        })))
        .mount(&mock)
        .await;
    mock
}

#[tokio::test]
async fn the_server_advertises_its_prompts() {
    let mock = MockServer::start().await;
    let client = connect(config(&mock)).await;

    let prompts = client.list_all_prompts().await.unwrap();
    let issue = prompts
        .iter()
        .find(|p| p.name == "jira_issue")
        .expect("jira_issue is not advertised");
    assert!(issue.description.is_some(), "{issue:?}");
    // The argument is what makes it usable as `/jira_issue PROJ-123`.
    let arguments = issue.arguments.as_ref().expect("no arguments");
    assert_eq!(arguments.len(), 1);
    assert_eq!(arguments[0].name, "issue_key");
    assert_eq!(arguments[0].required, Some(true));
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn getting_the_prompt_fetches_the_issue_and_briefs_on_it() {
    let mock = mock_issue().await;
    let client = connect(config(&mock)).await;

    let result = client
        .get_prompt(
            GetPromptRequestParams::new("jira_issue")
                .with_arguments(args(json!({ "issue_key": "PROJ-123" }))),
        )
        .await
        .unwrap();

    assert_eq!(result.messages.len(), 1);
    let text = result.messages[0]
        .content
        .as_text()
        .expect("not a text message")
        .text
        .clone();
    // Real data, not an instruction to go and fetch it.
    assert!(
        text.contains("Search times out on large projects"),
        "{text}"
    );
    assert!(text.contains("In Progress"), "{text}");
    assert!(text.contains("Jane Doe"), "{text}");
    assert!(text.contains("Reproduced on staging."), "{text}");
    assert!(text.contains("Work this issue"), "{text}");

    // Both endpoints were really called.
    assert_eq!(mock.received_requests().await.unwrap().len(), 2);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn an_issue_with_no_comments_still_briefs() {
    // The comment endpoint is not mocked, so it 404s — the briefing that
    // already succeeded must not be lost with it.
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "1", "key": "PROJ-1", "fields": { "summary": "Fresh ticket" }
        })))
        .mount(&mock)
        .await;
    let client = connect(config(&mock)).await;

    let result = client
        .get_prompt(
            GetPromptRequestParams::new("jira_issue")
                .with_arguments(args(json!({ "issue_key": "PROJ-1" }))),
        )
        .await
        .unwrap();

    let text = result.messages[0].content.as_text().unwrap().text.clone();
    assert!(text.contains("Fresh ticket"), "{text}");
    assert!(text.contains("No comments."), "{text}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_missing_issue_says_which_key_was_not_found() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-404"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "errorMessages": ["Issue does not exist or you do not have permission to see it."]
        })))
        .mount(&mock)
        .await;
    let client = connect(config(&mock)).await;

    let error = client
        .get_prompt(
            GetPromptRequestParams::new("jira_issue")
                .with_arguments(args(json!({ "issue_key": "PROJ-404" }))),
        )
        .await
        .expect_err("a missing issue produced a briefing");
    let error = error.to_string();
    assert!(error.contains("PROJ-404"), "{error}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn prompts_of_an_unconfigured_product_are_absent() {
    let mock = MockServer::start().await;
    let config = Config {
        jira: None,
        confluence: Some(ServiceConfig {
            base_url: mock.uri(),
            auth: Auth::Basic {
                username: "u@example.com".into(),
                token: "t".into(),
            },
            deployment: None,
        }),
        ..config(&mock)
    };
    let client = connect(config).await;
    assert!(client.list_all_prompts().await.unwrap().is_empty());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_product_filtered_down_to_nothing_loses_its_prompts() {
    // A prompt drives the tools; offering one whose tools were all removed by
    // ENABLED_TOOLS would be a way around the allowlist.
    let mock = MockServer::start().await;
    let config = Config {
        enabled_tools: ToolFilter::parse("confluence_*"),
        ..config(&mock)
    };
    let client = connect(config).await;
    assert!(client.list_all_prompts().await.unwrap().is_empty());
    client.cancel().await.unwrap();
}
