//! MCP resources: `jira://ISSUE-KEY` and `confluence://PAGE_ID`, end to end
//! over an in-memory transport.

use mcp_atlassian::server::AtlassianServer;
use mcp_atlassian_client::{Auth, Config, ServiceConfig, ToolFilter};
use rmcp::model::{ReadResourceRequestParams, ResourceContents};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn service(mock: &MockServer) -> ServiceConfig {
    ServiceConfig {
        base_url: mock.uri(),
        auth: Auth::Basic {
            username: "u@example.com".into(),
            token: "t".into(),
        },
        deployment: None,
    }
}

fn config(mock: &MockServer) -> Config {
    Config {
        jira: Some(service(mock)),
        confluence: Some(service(mock)),
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

/// The text of the single content block of a resource, with its MIME type.
fn text_of(contents: &ResourceContents) -> (&str, &str) {
    match contents {
        ResourceContents::TextResourceContents {
            text, mime_type, ..
        } => (text.as_str(), mime_type.as_deref().unwrap_or("")),
        other => panic!("expected a text resource, got {other:?}"),
    }
}

#[tokio::test]
async fn templates_describe_both_products() {
    let mock = MockServer::start().await;
    let client = connect(config(&mock)).await;

    let templates = client.list_all_resource_templates().await.unwrap();
    let uris: Vec<&str> = templates.iter().map(|t| t.uri_template.as_str()).collect();
    assert_eq!(
        uris,
        [
            "jira://{issue_key}",
            "jira://{issue_key}/comments",
            "confluence://{page_id}",
            "confluence://{page_id}/comments"
        ]
    );
    for template in &templates {
        assert!(template.description.is_some(), "{template:?}");
        assert!(template.mime_type.is_some(), "{template:?}");
    }

    // The concrete list stays empty: issues and pages are unbounded.
    assert!(client.list_all_resources().await.unwrap().is_empty());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn the_server_advertises_the_resources_capability() {
    let mock = MockServer::start().await;
    let client = connect(config(&mock)).await;
    let capabilities = client.peer_info().unwrap().capabilities.clone();
    assert!(capabilities.resources.is_some(), "{capabilities:?}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn only_configured_products_contribute_templates() {
    let mock = MockServer::start().await;
    let client = connect(Config {
        confluence: None,
        ..config(&mock)
    })
    .await;

    let templates = client.list_all_resource_templates().await.unwrap();
    assert_eq!(templates.len(), 2);
    assert_eq!(templates[0].uri_template, "jira://{issue_key}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn reading_an_issue_returns_json_with_the_key_case_preserved() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-123"))
        // The field list is explicit, so the response stays small (D4).
        .and(query_param(
            "fields",
            mcp_atlassian_jira::DEFAULT_ISSUE_FIELDS,
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "10001",
            "key": "PROJ-123",
            "fields": {
                "summary": "Fix login bug",
                "description": "Steps to reproduce...",
                "status": { "name": "In Progress" },
                "labels": ["auth"]
            }
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let client = connect(config(&mock)).await;
    let result = client
        .read_resource(ReadResourceRequestParams::new("jira://PROJ-123"))
        .await
        .unwrap();

    assert_eq!(result.contents.len(), 1);
    let (text, mime) = text_of(&result.contents[0]);
    assert_eq!(mime, "application/json");
    let issue: Value = serde_json::from_str(text).expect("the body is JSON");
    assert_eq!(issue["key"], "PROJ-123");
    assert_eq!(issue["fields"]["summary"], "Fix login bug");
    assert_eq!(issue["fields"]["description"], "Steps to reproduce...");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn reading_a_page_returns_markdown_under_its_title() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/content/123456"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "123456",
            "type": "page",
            "title": "Deployment runbook",
            "body": { "storage": { "value": "<h2>Rollback</h2><p>Run <code>make undo</code>.</p>" } }
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let client = connect(config(&mock)).await;
    let result = client
        .read_resource(ReadResourceRequestParams::new("confluence://123456"))
        .await
        .unwrap();

    let (text, mime) = text_of(&result.contents[0]);
    assert_eq!(mime, "text/markdown");
    assert!(text.starts_with("# Deployment runbook\n"), "{text}");
    assert!(
        text.contains("## Rollback"),
        "storage was not converted: {text}"
    );
    assert!(text.contains("`make undo`"), "{text}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_missing_issue_reports_what_is_missing() {
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
        .read_resource(ReadResourceRequestParams::new("jira://PROJ-404"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("404"), "{error}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn unknown_and_malformed_uris_are_rejected_with_the_expected_shape() {
    let mock = MockServer::start().await;
    let client = connect(config(&mock)).await;

    for (uri, expected) in [
        ("ftp://somewhere", "jira://ISSUE-KEY"),
        ("jira://", "jira://PROJ-123"),
        ("jira://PROJ-1/watchers", "jira://PROJ-123"),
        ("confluence://123/children", "confluence://123456"),
    ] {
        let error = client
            .read_resource(ReadResourceRequestParams::new(uri))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{uri} produced: {error}");
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn resources_follow_the_tool_allowlist() {
    // ENABLED_TOOLS that removes every Jira tool also removes `jira://` —
    // otherwise resources would be a way around the allowlist.
    let mock = MockServer::start().await;
    let client = connect(Config {
        enabled_tools: ToolFilter::parse("confluence_search"),
        disabled_tools: None,
        ..config(&mock)
    })
    .await;

    let templates = client.list_all_resource_templates().await.unwrap();
    assert_eq!(templates.len(), 2);
    assert_eq!(templates[0].uri_template, "confluence://{page_id}");

    let error = client
        .read_resource(ReadResourceRequestParams::new("jira://PROJ-1"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("ENABLED_TOOLS"), "{error}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn an_unconfigured_product_says_so() {
    let mock = MockServer::start().await;
    let client = connect(Config {
        confluence: None,
        ..config(&mock)
    })
    .await;

    let error = client
        .read_resource(ReadResourceRequestParams::new("confluence://123456"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("CONFLUENCE_URL"), "{error}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn comments_are_a_sub_resource_of_both_products() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-1/comment"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "comments": [{ "id": "5", "body": "first!", "created": "2026-01-01T00:00:00.000+0000" }],
            "total": 1
        })))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/content/123/child/comment"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{ "id": "9", "type": "comment", "title": "", "body": { "storage": { "value": "<p>looks <em>stale</em></p>" } } }],
            "size": 1
        })))
        .mount(&mock)
        .await;
    let client = connect(config(&mock)).await;

    let templates = client.list_all_resource_templates().await.unwrap();
    assert!(templates
        .iter()
        .any(|t| t.uri_template == "jira://{issue_key}/comments"));
    assert!(templates
        .iter()
        .any(|t| t.uri_template == "confluence://{page_id}/comments"));

    let result = client
        .read_resource(ReadResourceRequestParams::new("jira://PROJ-1/comments"))
        .await
        .unwrap();
    let (text, mime) = text_of(&result.contents[0]);
    assert_eq!(mime, "application/json");
    let page: Value = serde_json::from_str(text).unwrap();
    assert_eq!(page["comments"][0]["body"], "first!");

    let result = client
        .read_resource(ReadResourceRequestParams::new("confluence://123/comments"))
        .await
        .unwrap();
    let (text, mime) = text_of(&result.contents[0]);
    assert_eq!(mime, "text/markdown");
    assert!(text.contains("## Comment 9"), "{text}");
    assert!(text.contains("stale"), "{text}");
    assert!(!text.contains("<em>"), "storage was not converted: {text}");
    client.cancel().await.unwrap();
}
