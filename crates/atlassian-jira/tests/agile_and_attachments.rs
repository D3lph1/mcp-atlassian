use atlassian_client::{Auth, Error, ServiceConfig};
use atlassian_jira::JiraClient;
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> JiraClient {
    JiraClient::new(&ServiceConfig {
        base_url: server.uri(),
        auth: Auth::Basic {
            username: "u@example.com".into(),
            token: "secret".into(),
        },
    })
    .unwrap()
}

#[tokio::test]
async fn boards_and_sprints_roundtrip() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board"))
        .and(query_param("projectKeyOrId", "PROJ"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "values": [{ "id": 7, "name": "PROJ board", "type": "scrum" }],
            "isLast": true
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board/7/sprint"))
        .and(query_param("state", "active"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "values": [{ "id": 42, "name": "Sprint 5", "state": "active", "goal": "ship it" }],
            "isLast": true
        })))
        .expect(1)
        .mount(&server)
        .await;

    let jira = client(&server);
    let boards = jira.get_boards(Some("PROJ"), 25).await.unwrap();
    assert_eq!(boards.values[0].board_type, "scrum");
    let sprints = jira.get_sprints(7, Some("active")).await.unwrap();
    assert_eq!(sprints.values[0].name, "Sprint 5");
}

#[tokio::test]
async fn move_issues_posts_keys() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint/42/issue"))
        .and(body_partial_json(json!({ "issues": ["PROJ-1", "PROJ-2"] })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .move_issues_to_sprint(42, &["PROJ-1".into(), "PROJ-2".into()])
        .await
        .unwrap();
}

#[tokio::test]
async fn attachment_listing_and_download() {
    let server = MockServer::start().await;
    let content_url = format!("{}/secure/attachment/1001/report.pdf", server.uri());
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PROJ-1"))
        .and(query_param("fields", "attachment"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "1", "key": "PROJ-1",
            "fields": { "attachment": [{
                "id": "1001",
                "filename": "report.pdf",
                "size": 4,
                "mimeType": "application/pdf",
                "content": content_url
            }]}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/secure/attachment/1001/report.pdf"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"PDF!".to_vec()))
        .expect(1)
        .mount(&server)
        .await;

    let jira = client(&server);
    let attachments = jira.get_attachments("PROJ-1").await.unwrap();
    assert_eq!(attachments[0].filename, "report.pdf");
    let bytes = jira
        .download_attachment(&attachments[0].content)
        .await
        .unwrap();
    assert_eq!(bytes, b"PDF!");
}

#[tokio::test]
async fn download_refuses_foreign_origin() {
    let server = MockServer::start().await;
    let err = client(&server)
        .download_attachment("https://evil.example.com/steal")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Config(_)), "got: {err:?}");
    assert!(err.to_string().contains("foreign origin"), "{err}");
}

#[tokio::test]
async fn upload_sends_multipart_with_no_check_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue/PROJ-1/attachments"))
        .and(header("X-Atlassian-Token", "no-check"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "id": "1002",
            "filename": "notes.txt",
            "size": 5,
            "content": format!("{}/secure/attachment/1002/notes.txt", server.uri())
        }])))
        .expect(1)
        .mount(&server)
        .await;

    let uploaded = client(&server)
        .upload_attachment("PROJ-1", "notes.txt", b"hello".to_vec())
        .await
        .unwrap();
    assert_eq!(uploaded[0].id, "1002");
}
