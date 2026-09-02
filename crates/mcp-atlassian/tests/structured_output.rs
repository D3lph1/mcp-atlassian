//! Structured output, end to end over an in-memory transport: every tool
//! advertises an `outputSchema`, and results carry `structuredContent`
//! alongside the legacy text block.

use atlassian_client::{Auth, Config, ServiceConfig};
use mcp_atlassian::server::AtlassianServer;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
use wiremock::MockServer;

fn config(mock: &MockServer) -> Config {
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
        disabled_tools: None,
        read_only: false,
        dry_run: false,
        audit_log: None,
        cache_ttl: None,
    }
}

/// Runs the server over an in-memory duplex and returns a connected client, so
/// assertions see exactly what a real MCP client would receive on the wire.
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

fn args(value: Value) -> Option<Map<String, Value>> {
    value.as_object().cloned()
}

#[tokio::test]
async fn every_tool_advertises_an_output_schema() {
    let mock = MockServer::start().await;
    let client = connect(&mock).await;

    let tools = client.list_all_tools().await.unwrap();
    assert!(!tools.is_empty());
    for tool in &tools {
        assert!(
            tool.output_schema.is_some(),
            "{} has no outputSchema",
            tool.name
        );
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn list_results_are_objects_not_bare_arrays() {
    // structuredContent must be a JSON object per the MCP spec, so list-shaped
    // results are wrapped in {items, count} rather than returned as arrays.
    let mock = MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/rest/api/2/project/search"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "values": [
                { "id": "1", "key": "PROJ", "name": "Project" },
                { "id": "2", "key": "OPS", "name": "Operations" }
            ]
        })))
        .mount(&mock)
        .await;

    let client = connect(&mock).await;
    let result = client
        .call_tool(
            CallToolRequestParams::new("jira_get_projects")
                .with_arguments(args(json!({})).unwrap_or_default()),
        )
        .await
        .unwrap();

    let structured = result
        .structured_content
        .expect("tool must return structuredContent");
    assert!(structured.is_object(), "got: {structured}");
    assert_eq!(structured["count"], 2);
    assert_eq!(structured["items"][0]["key"], "PROJ");

    // The text block is still present for clients that predate structured output.
    let text = result.content[0]
        .as_text()
        .expect("a text block for backwards compatibility")
        .text
        .clone();
    assert!(text.contains("PROJ"), "text fallback missing: {text}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn structured_content_satisfies_the_advertised_schema() {
    let mock = MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/rest/api/2/myself"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "accountId": "acc-1",
            "displayName": "Alice",
            "active": true
        })))
        .mount(&mock)
        .await;

    let client = connect(&mock).await;
    let schema = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.name == "jira_get_myself")
        .and_then(|t| t.output_schema.clone())
        .expect("jira_get_myself must advertise a schema");

    let structured = client
        .call_tool(
            CallToolRequestParams::new("jira_get_myself")
                .with_arguments(args(json!({})).unwrap_or_default()),
        )
        .await
        .unwrap()
        .structured_content
        .expect("structuredContent");

    // Every field the schema marks required must be present in the payload.
    let required = schema
        .get("required")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(!required.is_empty(), "schema declares no required fields");
    for field in required {
        let field = field.as_str().unwrap();
        assert!(
            structured.get(field).is_some(),
            "payload is missing required field `{field}`: {structured}"
        );
    }
    assert_eq!(structured["displayName"], "Alice");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn status_only_tools_return_ok_and_a_message() {
    let mock = MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("PUT"))
        .and(wiremock::matchers::path("/rest/api/2/issue/PROJ-1"))
        .respond_with(wiremock::ResponseTemplate::new(204))
        .mount(&mock)
        .await;

    let client = connect(&mock).await;
    let structured = client
        .call_tool(
            CallToolRequestParams::new("jira_update_issue").with_arguments(
                args(json!({ "issue_key": "PROJ-1", "fields": { "summary": "New" } }))
                    .unwrap_or_default(),
            ),
        )
        .await
        .unwrap()
        .structured_content
        .expect("structuredContent");

    assert_eq!(structured["ok"], true);
    assert!(
        structured["message"].as_str().unwrap().contains("PROJ-1"),
        "message should name the issue: {structured}"
    );
    client.cancel().await.unwrap();
}
