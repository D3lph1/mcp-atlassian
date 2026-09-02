//! `AUDIT_LOG_FILE`: write operations append one JSONL record each, reads
//! append nothing. Exercised end to end over an in-memory transport, so the
//! records describe exactly what a real client asked for.

use std::fs;
use std::path::{Path, PathBuf};

use mcp_atlassian::server::AtlassianServer;
use mcp_atlassian_client::{Auth, Config, ServiceConfig};
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// One file per test: tests share a process, and the log is shared state.
fn log_path(test: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "mcp-atlassian-audit-{}-{test}.jsonl",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    path
}

fn config(mock: &MockServer, audit_log: &Path) -> Config {
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
        audit_log: Some(audit_log.to_path_buf()),
        ..Config::default()
    }
}

async fn connect(mock: &MockServer, audit_log: &Path) -> RunningService<RoleClient, ()> {
    let server = AtlassianServer::new(&config(mock, audit_log)).unwrap();
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

/// The log as parsed records. Reading is safe right after a call returns: the
/// record is appended before the response leaves the server.
fn records(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .expect("the audit log is created when the server starts")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is one JSON object"))
        .collect()
}

#[tokio::test]
async fn write_tool_records_its_arguments_and_outcome() {
    let log = log_path("write");
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue/PROJ-1/comment"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "10000",
            "body": "Looks good"
        })))
        .mount(&mock)
        .await;

    let client = connect(&mock, &log).await;
    client
        .call_tool(
            CallToolRequestParams::new("jira_add_comment")
                .with_arguments(args(json!({ "issue_key": "PROJ-1", "body": "Looks good" }))),
        )
        .await
        .unwrap();
    client.cancel().await.unwrap();

    let records = records(&log);
    assert_eq!(records.len(), 1, "{records:?}");
    let record = &records[0];
    assert_eq!(record["tool"], "jira_add_comment");
    assert_eq!(record["args"]["issue_key"], "PROJ-1");
    assert_eq!(record["args"]["body"], "Looks good");
    assert_eq!(record["outcome"], "ok");
    assert!(record["error"].is_null(), "a success carries no error");
    assert!(record["duration_ms"].is_number(), "{record}");
    // RFC 3339, UTC.
    let ts = record["ts"].as_str().unwrap();
    assert!(ts.ends_with('Z') && ts.contains('T'), "bad timestamp: {ts}");
    // `jira_add_comment` is a write but not a destructive one.
    assert!(record["destructive"].is_null(), "{record}");
    // What the write produced, so the line alone is enough to find it.
    assert_eq!(record["result"], "10000");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&log).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    fs::remove_file(&log).unwrap();
}

#[tokio::test]
async fn read_tools_are_not_recorded() {
    let log = log_path("read");
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search/jql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "issues": [] })))
        .mount(&mock)
        .await;

    let client = connect(&mock, &log).await;
    client
        .call_tool(
            CallToolRequestParams::new("jira_search")
                .with_arguments(args(json!({ "jql": "project = PROJ" }))),
        )
        .await
        .unwrap();
    client.cancel().await.unwrap();

    assert!(records(&log).is_empty(), "reads must not be audited");
    fs::remove_file(&log).unwrap();
}

#[tokio::test]
async fn destructive_tools_are_flagged() {
    let log = log_path("destructive");
    let mock = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/rest/api/2/issue/PROJ-1"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&mock)
        .await;

    let client = connect(&mock, &log).await;
    client
        .call_tool(
            CallToolRequestParams::new("jira_delete_issue")
                .with_arguments(args(json!({ "issue_key": "PROJ-1" }))),
        )
        .await
        .unwrap();
    client.cancel().await.unwrap();

    let records = records(&log);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0]["tool"], "jira_delete_issue");
    assert_eq!(records[0]["destructive"], true);
    assert_eq!(records[0]["outcome"], "ok");
    fs::remove_file(&log).unwrap();
}

#[tokio::test]
async fn failed_writes_are_recorded_with_the_error() {
    let log = log_path("failure");
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue/PROJ-9/comment"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "errorMessages": ["Issue does not exist or you do not have permission to see it."]
        })))
        .mount(&mock)
        .await;

    let client = connect(&mock, &log).await;
    // The tool reports the 404 as an MCP error, so the client sees a failed
    // call — the attempt still happened and must appear in the log.
    let result = client
        .call_tool(
            CallToolRequestParams::new("jira_add_comment")
                .with_arguments(args(json!({ "issue_key": "PROJ-9", "body": "hi" }))),
        )
        .await;
    assert!(result.is_err(), "the mock returned 404: {result:?}");
    client.cancel().await.unwrap();

    let records = records(&log);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0]["outcome"], "error");
    let error = records[0]["error"].as_str().expect("an error message");
    assert!(error.contains("404"), "unhelpful error text: {error}");
    fs::remove_file(&log).unwrap();
}

#[tokio::test]
async fn every_write_of_a_session_is_appended_in_order() {
    let log = log_path("append");
    let mock = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/2/issue/PROJ-1"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&mock)
        .await;

    let client = connect(&mock, &log).await;
    for summary in ["first", "second", "third"] {
        client
            .call_tool(
                CallToolRequestParams::new("jira_update_issue").with_arguments(args(
                    json!({ "issue_key": "PROJ-1", "fields": { "summary": summary } }),
                )),
            )
            .await
            .unwrap();
    }
    client.cancel().await.unwrap();

    let records = records(&log);
    let summaries: Vec<&str> = records
        .iter()
        .map(|r| r["args"]["fields"]["summary"].as_str().unwrap())
        .collect();
    assert_eq!(summaries, ["first", "second", "third"]);
    fs::remove_file(&log).unwrap();
}

#[test]
fn an_unwritable_audit_path_fails_at_startup() {
    // Fail fast: a server that cannot audit must not start and silently drop
    // records.
    let config = Config {
        jira: Some(ServiceConfig {
            base_url: "https://example.atlassian.net".into(),
            auth: Auth::Basic {
                username: "u@example.com".into(),
                token: "t".into(),
            },
            deployment: None,
        }),
        confluence: None,
        audit_log: Some(PathBuf::from("/nonexistent-directory/audit.jsonl")),
        ..Config::default()
    };
    let Err(error) = AtlassianServer::new(&config) else {
        panic!("the server started without a usable audit log");
    };
    let error = error.to_string();
    assert!(error.contains("audit log"), "unhelpful error: {error}");
}
