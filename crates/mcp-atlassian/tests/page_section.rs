//! `confluence_update_page_section` edits the storage document in place
//! (D36): the sections it does not touch keep their macros byte for byte.
//! The earlier implementation round-tripped the whole page through Markdown,
//! which turned every macro outside the edited section into plain text.

use atlassian_client::{Auth, Config, ServiceConfig};
use mcp_atlassian::server::AtlassianServer;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config(mock: &MockServer) -> Config {
    Config {
        jira: None,
        confluence: Some(ServiceConfig {
            base_url: format!("{}/wiki", mock.uri()),
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
    let server = AtlassianServer::new(&config(mock)).unwrap();
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

const STORAGE: &str = "<h1>Runbook</h1>\
    <ac:structured-macro ac:name=\"toc\"><ac:parameter ac:name=\"maxLevel\">2</ac:parameter></ac:structured-macro>\
    <h2>Deploy</h2><p>old</p>\
    <h2>Rollback</h2><ac:structured-macro ac:name=\"code\"><ac:parameter ac:name=\"language\">bash</ac:parameter><ac:plain-text-body><![CDATA[git revert HEAD]]></ac:plain-text-body></ac:structured-macro>";

#[tokio::test]
async fn editing_one_section_leaves_the_macros_of_the_others_intact() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/content/123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "123", "type": "page", "title": "Runbook",
            "version": { "number": 3 },
            "body": { "storage": { "value": STORAGE } }
        })))
        .mount(&mock)
        .await;
    Mock::given(method("PUT"))
        .and(path("/wiki/rest/api/content/123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "123", "type": "page", "title": "Runbook", "version": { "number": 4 }
        })))
        .expect(1)
        .mount(&mock)
        .await;
    let client = connect(&mock).await;

    client
        .call_tool(
            CallToolRequestParams::new("confluence_update_page_section").with_arguments(args(
                json!({
                    "page_id": "123",
                    "heading_text": "Deploy",
                    "new_content": "Run `make deploy`.\n\n```bash\nmake deploy\n```"
                }),
            )),
        )
        .await
        .unwrap();

    let put = mock
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.method == "PUT")
        .expect("the page was updated");
    let body: Value = serde_json::from_slice(&put.body).unwrap();
    let sent = body["body"]["storage"]["value"].as_str().unwrap();

    // The untouched sections, macros and all, are byte for byte the same.
    assert!(
        sent.starts_with("<h1>Runbook</h1><ac:structured-macro ac:name=\"toc\">"),
        "{sent}"
    );
    assert!(sent.contains("<![CDATA[git revert HEAD]]>"), "{sent}");
    // The edited section carries the new content, converted from Markdown,
    // with its fenced block as a code macro.
    assert!(
        sent.contains("<h2>Deploy</h2><p>Run <code>make deploy</code>.</p>"),
        "{sent}"
    );
    assert!(sent.contains("<![CDATA[make deploy]]>"), "{sent}");
    assert!(!sent.contains("<p>old</p>"), "{sent}");
    assert_eq!(body["version"]["number"], 4);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_heading_that_is_not_on_the_page_is_reported_without_writing() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/content/123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "123", "type": "page", "title": "Runbook",
            "body": { "storage": { "value": STORAGE } }
        })))
        .mount(&mock)
        .await;
    let client = connect(&mock).await;

    let error = client
        .call_tool(
            CallToolRequestParams::new("confluence_update_page_section").with_arguments(args(
                json!({ "page_id": "123", "heading_text": "Nope", "new_content": "x" }),
            )),
        )
        .await
        .expect_err("an unknown heading must not update the page");
    assert!(error.to_string().contains("Nope"), "{error}");
    assert!(
        !mock
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.method == "PUT"),
        "nothing may be written"
    );
    client.cancel().await.unwrap();
}
