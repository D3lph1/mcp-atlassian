//! MCP resources: `jira://ISSUE-KEY` and `confluence://PAGE_ID`, end to end
//! over an in-memory transport.

use std::collections::HashSet;

use atlassian_client::{Auth, Config, ServiceConfig};
use mcp_atlassian::server::AtlassianServer;
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
    }
}

fn config(mock: &MockServer) -> Config {
    Config {
        jira: Some(service(mock)),
        confluence: Some(service(mock)),
        enabled_tools: None,
        read_only: false,
        audit_log: None,
        cache_ttl: None,
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
    assert_eq!(uris, ["jira://{issue_key}", "confluence://{page_id}"]);
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
    assert_eq!(templates.len(), 1);
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
            "summary,description,status,priority,issuetype,assignee,reporter,labels,created,updated",
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
        ("jira://PROJ-1/comments", "jira://PROJ-123"),
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
        enabled_tools: Some(HashSet::from(["confluence_search".to_string()])),
        ..config(&mock)
    })
    .await;

    let templates = client.list_all_resource_templates().await.unwrap();
    assert_eq!(templates.len(), 1);
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
