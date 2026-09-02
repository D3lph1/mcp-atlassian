//! `DRY_RUN`: write tools stay advertised, are validated, and never reach
//! Atlassian. Exercised end to end over an in-memory transport against a mock
//! that would answer if a request ever arrived — so "nothing was sent" is
//! asserted against the wire, not against our own bookkeeping.

use std::fs;
use std::path::{Path, PathBuf};

use atlassian_client::{Auth, Config, ServiceConfig};
use mcp_atlassian::server::AtlassianServer;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config(mock: &MockServer, dry_run: bool, audit_log: Option<PathBuf>) -> Config {
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
        dry_run,
        audit_log,
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

fn args(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

/// A mock that answers issue creation, so a call that leaks through succeeds
/// visibly instead of failing for an unrelated reason.
async fn mock_create_issue() -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "10001", "key": "PROJ-1", "self": "https://example.atlassian.net/rest/api/2/issue/10001"
        })))
        .mount(&mock)
        .await;
    mock
}

fn create_issue_args() -> Map<String, Value> {
    args(json!({
        "project_key": "PROJ",
        "issue_type": "Task",
        "summary": "Rehearsed, not created"
    }))
}

#[tokio::test]
async fn write_tools_stay_listed_unlike_in_read_only_mode() {
    let mock = MockServer::start().await;
    let client = connect(config(&mock, true, None)).await;

    let names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    assert!(names.iter().any(|n| n == "jira_create_issue"), "{names:?}");
    assert!(names.iter().any(|n| n == "jira_get_issue"), "{names:?}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_write_call_is_described_and_never_sent() {
    let mock = mock_create_issue().await;
    let client = connect(config(&mock, true, None)).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("jira_create_issue").with_arguments(create_issue_args()),
        )
        .await
        .unwrap();

    let structured = result.structured_content.expect("no structuredContent");
    assert_eq!(structured["dry_run"], json!(true));
    assert_eq!(structured["tool"], json!("jira_create_issue"));
    assert_eq!(
        structured["arguments"]["summary"],
        json!("Rehearsed, not created")
    );
    assert_eq!(structured["warnings"], json!([]));

    // The text block is what a text-only client renders.
    let text = result
        .content
        .iter()
        .find_map(|block| block.as_text().map(|t| t.text.clone()))
        .expect("no text content");
    assert!(text.contains("DRY RUN"), "{text}");
    assert!(text.contains("jira_create_issue"), "{text}");

    assert!(
        mock.received_requests().await.unwrap().is_empty(),
        "the dry run reached Atlassian"
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_destructive_tool_says_so_in_its_report() {
    let mock = MockServer::start().await;
    let client = connect(config(&mock, true, None)).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("jira_delete_issue")
                .with_arguments(args(json!({ "issue_key": "PROJ-1" }))),
        )
        .await
        .unwrap();

    let structured = result.structured_content.unwrap();
    assert_eq!(structured["destructive"], json!(true));
    assert!(mock.received_requests().await.unwrap().is_empty());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn read_tools_are_not_intercepted() {
    // Reads still happen for real: rehearsing a prompt is only useful if the
    // model sees the instance's actual data.
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "10001", "key": "PROJ-1", "fields": { "summary": "Real" }
        })))
        .mount(&mock)
        .await;
    let client = connect(config(&mock, true, None)).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("jira_get_issue")
                .with_arguments(args(json!({ "issue_key": "PROJ-1" }))),
        )
        .await
        .unwrap();

    let structured = result.structured_content.unwrap();
    assert_eq!(structured["key"], json!("PROJ-1"));
    assert!(structured.get("dry_run").is_none());
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_missing_required_argument_fails_as_the_real_call_would() {
    let mock = mock_create_issue().await;
    let client = connect(config(&mock, true, None)).await;

    let error = client
        .call_tool(
            CallToolRequestParams::new("jira_create_issue")
                .with_arguments(args(json!({ "project_key": "PROJ" }))),
        )
        .await
        .expect_err("a call that cannot succeed reported success");

    let error = error.to_string();
    assert!(error.contains("issue_type"), "{error}");
    assert!(error.contains("summary"), "{error}");
    assert!(mock.received_requests().await.unwrap().is_empty());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn an_argument_the_tool_does_not_declare_is_a_warning() {
    // serde drops unknown fields, so the real call would have run — but
    // silently without the argument, which is exactly the mistake worth
    // surfacing while rehearsing.
    let mock = mock_create_issue().await;
    let client = connect(config(&mock, true, None)).await;

    let mut arguments = create_issue_args();
    arguments.insert("projectKey".into(), json!("PROJ"));
    let result = client
        .call_tool(CallToolRequestParams::new("jira_create_issue").with_arguments(arguments))
        .await
        .unwrap();

    let warnings = result.structured_content.unwrap()["warnings"].clone();
    assert_eq!(warnings.as_array().map(Vec::len), Some(1), "{warnings}");
    assert!(
        warnings[0].as_str().unwrap().contains("projectKey"),
        "{warnings}"
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn every_tool_still_advertises_an_output_schema() {
    // D20 holds in this mode too: intercepted tools advertise the report's
    // schema, which is what they now return.
    let mock = MockServer::start().await;
    let client = connect(config(&mock, true, None)).await;

    for tool in client.list_all_tools().await.unwrap() {
        assert!(
            tool.output_schema.is_some(),
            "{} has no outputSchema",
            tool.name
        );
        let read_only = tool
            .annotations
            .as_ref()
            .and_then(|a| a.read_only_hint)
            .unwrap_or(false);
        // The mode is disclosed rather than hidden, so a model does not report
        // writes that never happened.
        let describes_dry_run = tool
            .description
            .as_deref()
            .is_some_and(|d| d.contains("DRY_RUN"));
        assert_eq!(describes_dry_run, !read_only, "{}", tool.name);
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn the_audit_log_marks_intercepted_calls() {
    // Auditing sits outside the interception, so the attempt is still
    // recorded — but a record that claimed the write happened would be a lie.
    let log = std::env::temp_dir().join(format!(
        "mcp-atlassian-dry-run-{}.jsonl",
        std::process::id()
    ));
    let _ = fs::remove_file(&log);
    let mock = mock_create_issue().await;
    let client = connect(config(&mock, true, Some(log.clone()))).await;

    client
        .call_tool(
            CallToolRequestParams::new("jira_create_issue").with_arguments(create_issue_args()),
        )
        .await
        .unwrap();

    let records = read_log(&log);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0]["tool"], json!("jira_create_issue"));
    assert_eq!(records[0]["outcome"], json!("ok"));
    assert_eq!(records[0]["dry_run"], json!(true));
    client.cancel().await.unwrap();
    let _ = fs::remove_file(&log);
}

#[tokio::test]
async fn without_dry_run_the_log_does_not_mention_it() {
    let log = std::env::temp_dir().join(format!(
        "mcp-atlassian-wet-run-{}.jsonl",
        std::process::id()
    ));
    let _ = fs::remove_file(&log);
    let mock = mock_create_issue().await;
    let client = connect(config(&mock, false, Some(log.clone()))).await;

    client
        .call_tool(
            CallToolRequestParams::new("jira_create_issue").with_arguments(create_issue_args()),
        )
        .await
        .unwrap();

    let records = read_log(&log);
    assert_eq!(records.len(), 1, "{records:?}");
    assert!(records[0].get("dry_run").is_none(), "{:?}", records[0]);
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);
    client.cancel().await.unwrap();
    let _ = fs::remove_file(&log);
}

fn read_log(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("the audit log is not valid JSONL"))
        .collect()
}
