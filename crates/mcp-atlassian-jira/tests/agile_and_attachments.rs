use mcp_atlassian_client::{Auth, Error, ServiceConfig, Upload};
use mcp_atlassian_jira::{Attachment, JiraClient};
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
        deployment: None,
    })
    .unwrap()
}

fn dc_client(server: &MockServer) -> JiraClient {
    JiraClient::new(&ServiceConfig {
        base_url: server.uri(),
        auth: Auth::Pat {
            token: "pat".into(),
        },
        deployment: None,
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
    let sprints = jira.get_sprints(7, Some("active"), 25, 0).await.unwrap();
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
    let bytes = jira.download_attachment(&attachments[0]).await.unwrap();
    assert_eq!(bytes, b"PDF!");
}

fn foreign_attachment() -> Attachment {
    Attachment {
        id: "1001".into(),
        filename: "report.pdf".into(),
        size: 4,
        mime_type: None,
        content: "https://site.atlassian.net/secure/attachment/1001/report.pdf".into(),
        created: None,
        author: None,
    }
}

#[tokio::test]
async fn download_on_server_refuses_a_foreign_content_url() {
    let server = MockServer::start().await;
    let err = dc_client(&server)
        .download_attachment(&foreign_attachment())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Config(_)), "got: {err:?}");
    assert!(err.to_string().contains("foreign origin"), "{err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn download_on_cloud_falls_back_to_the_content_endpoint() {
    // Under OAuth the base is the api.atlassian.com gateway and the `content`
    // URL names the site, so the same-origin rule would refuse every download.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/attachment/content/1001"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"PDF!".to_vec()))
        .expect(1)
        .mount(&server)
        .await;
    let bytes = client(&server)
        .download_attachment(&foreign_attachment())
        .await
        .unwrap();
    assert_eq!(bytes, b"PDF!");
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
        .upload_attachment("PROJ-1", Upload::bytes("notes.txt", b"hello".to_vec()))
        .await
        .unwrap();
    assert_eq!(uploaded[0].id, "1002");
}

#[tokio::test]
async fn sprints_and_sprint_issues_page_by_offset() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board/7/sprint"))
        .and(query_param("maxResults", "2"))
        .and(query_param("startAt", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "values": [{ "id": 3, "name": "Sprint 3", "state": "future" }],
            "startAt": 2, "isLast": true
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/sprint/3/issue"))
        .and(query_param("startAt", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issues": [], "startAt": 50, "total": 50
        })))
        .expect(1)
        .mount(&server)
        .await;

    let jira = client(&server);
    let sprints = jira.get_sprints(7, None, 2, 2).await.unwrap();
    assert_eq!(sprints.start_at, 2);
    assert!(sprints.is_last);
    let issues = jira.get_sprint_issues(3, 25, 50).await.unwrap();
    assert_eq!(issues.start_at, Some(50));
    assert_eq!(issues.total, Some(50));
}
