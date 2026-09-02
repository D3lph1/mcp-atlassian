//! Invariants that must hold for *every* registered tool, not for the handful
//! a hand-written test happens to cover.
//!
//! The tool wrappers are thin — parse arguments, call the client, wrap the
//! result — and that thinness is why they went untested: each one looks too
//! trivial to break. But they are also where the decisions live that no
//! compiler checks, and all three defects found in the 2026-09-02 review were
//! in this layer (D31). These tests exercise each tool once, against a mock
//! that answers everything, and assert what must be true of all of them.
//!
//! The mock's `{}` response deserializes into almost nothing, so most calls
//! end in an error. That is fine and deliberate: what is under test is the
//! request the tool *issued*, not the response it could not parse.

use std::collections::BTreeMap;

use atlassian_client::{Auth, Config, ServiceConfig};
use mcp_atlassian::server::AtlassianServer;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Page sizes are capped here (`atlassian_client::mcp::MAX_SEARCH_RESULTS`).
const CAP: u64 = 50;
/// What a tool is asked for when the test wants to see the cap applied.
const ABSURD: u64 = 100_000;

fn config(mock: &MockServer) -> Config {
    let service = ServiceConfig {
        base_url: mock.uri(),
        auth: Auth::Basic {
            username: "u@example.com".into(),
            token: "t".into(),
        },
    };
    Config {
        jira: Some(service.clone()),
        confluence: Some(service),
        enabled_tools: None,
        disabled_tools: None,
        read_only: false,
        dry_run: false,
        audit_log: None,
        cache_ttl: None,
    }
}

/// A server whose Atlassian answers everything with `{}`.
async fn connect() -> (MockServer, RunningService<RoleClient, ()>) {
    let mock = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&mock)
        .await;
    let server = AtlassianServer::new(&config(&mock)).unwrap();
    let (client_io, server_io) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    let client = ().serve(client_io).await.unwrap();
    (mock, client)
}

/// A real file, so the upload tools get past reading it and actually issue
/// their request.
fn upload_source() -> String {
    let path = std::env::temp_dir().join(format!("mcp-every-tool-{}.txt", std::process::id()));
    std::fs::write(&path, b"payload").unwrap();
    path.to_string_lossy().into_owned()
}

/// Builds arguments that satisfy a tool's input schema.
///
/// Values are shaped by property name where the name implies one (an issue key
/// looks like `PROJ-1`), and by JSON type otherwise. Only required properties
/// are filled unless `page_size` asks for the paging ones too — an optional
/// argument left out is the case worth exercising.
fn arguments(schema: &Value, page_size: Option<u64>) -> Map<String, Value> {
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut arguments = Map::new();
    for (name, property) in &properties {
        let is_page_size = matches!(name.as_str(), "limit" | "max_results");
        let wanted = required.contains(&name.as_str()) || (is_page_size && page_size.is_some());
        if !wanted {
            continue;
        }
        if is_page_size {
            if let Some(size) = page_size {
                arguments.insert(name.clone(), json!(size));
                continue;
            }
        }
        arguments.insert(name.clone(), value_for(name, property));
    }
    arguments
}

fn value_for(name: &str, property: &Value) -> Value {
    match name {
        "issue_key" | "inward_issue_key" | "outward_issue_key" => json!("PROJ-1"),
        "epic_key" => json!("PROJ-9"),
        "project_key" => json!("PROJ"),
        "space_key" => json!("DEV"),
        "file_path" => json!(upload_source()),
        "save_path" => json!(std::env::temp_dir()
            .join("mcp-every-tool-download.bin")
            .to_string_lossy()),
        "jql" => json!("project = PROJ"),
        "cql" => json!("space = DEV"),
        "issue_type" => json!("Task"),
        "time_spent" => json!("1h"),
        _ => by_type(property),
    }
}

fn by_type(property: &Value) -> Value {
    match property.get("type").and_then(Value::as_str) {
        Some("integer") | Some("number") => json!(1),
        Some("boolean") => json!(false),
        // An array of the wrong element type fails to deserialize before the
        // tool runs, which would look exactly like a tool that issues no
        // request — so the item type matters here.
        Some("array") => json!([by_type(property.get("items").unwrap_or(&Value::Null))]),
        Some("object") => json!({}),
        // Strings, and anything schemars expressed as a `$ref` or `anyOf`.
        _ => json!("PROJ-1"),
    }
}

/// Tools that legitimately issue nothing for the arguments this test builds.
///
/// `arguments` fills only the *required* properties, and this tool requires
/// at least one of two optional ones — refusing early rather than moving a
/// page nowhere. That refusal is the behaviour, not a gap.
const NO_REQUEST_EXPECTED: &[&str] = &["confluence_move_page"];

/// Whether a tool takes a page size from its caller at all. A constant a tool
/// chose for an internal lookup did not come from the caller and is not what
/// the cap governs.
fn takes_a_page_size(schema: &Value) -> bool {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| {
            properties.contains_key("limit") || properties.contains_key("max_results")
        })
}

/// Calls every tool once and returns the requests each one issued.
async fn call_every_tool(page_size: Option<u64>) -> BTreeMap<String, Vec<wiremock::Request>> {
    let (mock, client) = connect().await;
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 70, "the tool set changed; update this test");

    let mut issued = BTreeMap::new();
    let mut seen = 0;
    for tool in tools {
        let schema = serde_json::to_value(&*tool.input_schema).unwrap();
        let name = tool.name.to_string();
        // The result is deliberately ignored: the mock cannot answer 70
        // different response shapes, and the request is what is under test.
        let asked_for_a_page_size = page_size.is_some() && takes_a_page_size(&schema);
        let _ = client
            .call_tool(
                CallToolRequestParams::new(name.clone())
                    .with_arguments(arguments(&schema, page_size)),
            )
            .await;
        let all = mock.received_requests().await.unwrap();
        let requests = all[seen..].to_vec();
        seen = all.len();
        if page_size.is_none() || asked_for_a_page_size {
            issued.insert(name, requests);
        }
    }
    client.cancel().await.unwrap();
    issued
}

#[tokio::test]
async fn every_tool_reaches_the_api_with_a_well_formed_path() {
    // Catches what a thin wrapper gets wrong: a typo in the endpoint, an
    // unsubstituted `format!` placeholder, an argument that never reaches the
    // path. A tool that issues nothing is not wired to its client at all.
    let issued = call_every_tool(None).await;

    let silent: Vec<&String> = issued
        .iter()
        .filter(|(name, requests)| {
            requests.is_empty() && !NO_REQUEST_EXPECTED.contains(&name.as_str())
        })
        .map(|(name, _)| name)
        .collect();
    assert!(
        silent.is_empty(),
        "these tools issued no request: {silent:?}"
    );

    for (name, requests) in &issued {
        for request in requests {
            let path = request.url.path();
            assert!(
                !path.contains('{'),
                "{name}: unsubstituted placeholder in {path}"
            );
            assert!(!path.contains("//"), "{name}: empty path segment in {path}");
            assert!(
                !path.split('/').any(|segment| segment == ".."),
                "{name}: `..` segment in {path}"
            );
            assert!(
                path.starts_with("/rest/"),
                "{name}: unexpected endpoint {path}"
            );
        }
    }
}

#[tokio::test]
async fn no_tool_passes_an_uncapped_page_size_to_atlassian() {
    // The invariant `mcp::page_size` exists to hold. Ten tools had grown an
    // `unwrap_or` without the cap (D31); a per-tool test would have to be
    // remembered for each new tool, and this one cannot be forgotten.
    let issued = call_every_tool(Some(ABSURD)).await;

    let mut checked = 0;
    for (name, requests) in &issued {
        for request in requests {
            for (key, value) in request.url.query_pairs() {
                if !matches!(key.as_ref(), "limit" | "maxResults") {
                    continue;
                }
                let sent: u64 = value.parse().unwrap_or_else(|_| {
                    panic!("{name}: non-numeric {key}={value}");
                });
                assert!(
                    sent <= CAP,
                    "{name}: asked for {ABSURD}, sent {key}={sent} — missing page_size()"
                );
                checked += 1;
            }
        }
    }
    // Guards the guard: if the query parameter were ever renamed, the loop
    // above would silently check nothing.
    assert!(checked >= 10, "only {checked} page sizes were inspected");
}
