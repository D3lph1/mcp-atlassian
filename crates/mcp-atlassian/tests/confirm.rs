//! `CONFIRM_DESTRUCTIVE` (D42): a destructive tool asks through elicitation
//! and runs only on a yes; a client without elicitation is not blocked.

use std::sync::{Arc, Mutex};

use mcp_atlassian::server::AtlassianServer;
use mcp_atlassian_client::{Auth, Config, ServiceConfig};
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientInfo, ElicitRequestParams, ElicitResult,
    ElicitationAction, Implementation,
};
use rmcp::service::{RequestContext, RunningService};
use rmcp::{ClientHandler, ErrorData as McpError, RoleClient, ServiceExt};
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
        confirm_destructive: true,
        ..Config::default()
    }
}

/// A client that supports elicitation and answers every question the same
/// way, remembering what it was asked.
#[derive(Clone)]
struct Answering {
    accept: bool,
    asked: Arc<Mutex<Vec<String>>>,
}

impl ClientHandler for Answering {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.capabilities = ClientCapabilities::builder().enable_elicitation().build();
        info.client_info = Implementation::new("test-client", "0");
        info
    }

    async fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, McpError> {
        let message = match request {
            ElicitRequestParams::FormElicitationParams { message, .. } => message,
            _ => String::from("<not a form>"),
        };
        self.asked.lock().unwrap().push(message);
        Ok(if self.accept {
            ElicitResult::new(ElicitationAction::Accept).with_content(json!({ "confirm": true }))
        } else {
            ElicitResult::new(ElicitationAction::Decline)
        })
    }
}

async fn connect<H: ClientHandler>(mock: &MockServer, handler: H) -> RunningService<RoleClient, H> {
    let server = AtlassianServer::new(&config(mock)).unwrap();
    let (client_io, server_io) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    handler.serve(client_io).await.unwrap()
}

fn args(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

async fn mock_delete() -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/rest/api/2/issue/PROJ-1"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&mock)
        .await;
    mock
}

fn delete_call() -> CallToolRequestParams {
    CallToolRequestParams::new("jira_delete_issue")
        .with_arguments(args(json!({ "issue_key": "PROJ-1" })))
}

#[tokio::test]
async fn a_confirmed_destructive_call_runs_and_the_question_names_it() {
    let mock = mock_delete().await;
    let asked = Arc::new(Mutex::new(Vec::new()));
    let client = connect(
        &mock,
        Answering {
            accept: true,
            asked: asked.clone(),
        },
    )
    .await;

    let result = client.call_tool(delete_call()).await.unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);
    let questions = asked.lock().unwrap().clone();
    assert_eq!(questions.len(), 1);
    assert!(questions[0].contains("Delete Jira issue"), "{questions:?}");
    assert!(questions[0].contains("PROJ-1"), "{questions:?}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_declined_destructive_call_does_not_reach_atlassian() {
    let mock = mock_delete().await;
    let client = connect(
        &mock,
        Answering {
            accept: false,
            asked: Arc::new(Mutex::new(Vec::new())),
        },
    )
    .await;

    let result = client.call_tool(delete_call()).await.unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");
    let text = result.content[0].as_text().unwrap().text.clone();
    assert!(text.contains("not performed"), "{text}");
    assert!(mock.received_requests().await.unwrap().is_empty());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_non_destructive_write_is_not_asked_about() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue/PROJ-1/comment"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "1", "body": "x" })))
        .mount(&mock)
        .await;
    let asked = Arc::new(Mutex::new(Vec::new()));
    let client = connect(
        &mock,
        Answering {
            accept: false,
            asked: asked.clone(),
        },
    )
    .await;
    client
        .call_tool(
            CallToolRequestParams::new("jira_add_comment")
                .with_arguments(args(json!({ "issue_key": "PROJ-1", "body": "x" }))),
        )
        .await
        .unwrap();
    assert!(asked.lock().unwrap().is_empty());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_client_without_elicitation_is_not_blocked() {
    // `()` declares no elicitation capability; the call goes through, and
    // the server has warned once on stderr.
    let mock = mock_delete().await;
    let client = connect(&mock, ()).await;
    let result = client.call_tool(delete_call()).await.unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);
    client.cancel().await.unwrap();
}
