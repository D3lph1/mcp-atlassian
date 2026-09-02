use atlassian_client::{Auth, ServiceConfig, Upload};
use atlassian_confluence::ConfluenceClient;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> ConfluenceClient {
    ConfluenceClient::new(&ServiceConfig {
        base_url: format!("{}/wiki", server.uri()),
        auth: Auth::Basic {
            username: "u@example.com".into(),
            token: "t".into(),
        },
        deployment: None,
    })
    .unwrap()
}

#[tokio::test]
async fn attachment_download_resolves_relative_link() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/content/123/child/attachment"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "id": "att1",
                "title": "diagram.png",
                "extensions": { "mediaType": "image/png", "fileSize": 3 },
                "_links": { "download": "/download/attachments/123/diagram.png?version=1&modificationDate=1700000000000&api=v2" }
            }],
            "size": 1
        })))
        .mount(&server)
        .await;
    // The download link is instance-relative, carries a query string (as real
    // ones always do) and must resolve under /wiki with the query intact.
    Mock::given(method("GET"))
        .and(path("/wiki/download/attachments/123/diagram.png"))
        .and(query_param("version", "1"))
        .and(query_param("api", "v2"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"PNG".to_vec()))
        .expect(1)
        .mount(&server)
        .await;

    let confluence = client(&server);
    let attachments = confluence.get_attachments("123", 25, 0).await.unwrap();
    let link = attachments.results[0]
        .links
        .as_ref()
        .unwrap()
        .download
        .clone()
        .unwrap();
    let bytes = confluence.download_attachment(&link).await.unwrap();
    assert_eq!(bytes, b"PNG");
}

#[tokio::test]
async fn upload_attachment_posts_multipart() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/wiki/rest/api/content/123/child/attachment"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{ "id": "att2", "title": "notes.txt" }],
            "size": 1
        })))
        .expect(1)
        .mount(&server)
        .await;

    let uploaded = client(&server)
        .upload_attachment("123", Upload::bytes("notes.txt", b"hi".to_vec()))
        .await
        .unwrap();
    assert_eq!(uploaded.results[0].id, "att2");
}

#[tokio::test]
async fn page_versions_and_historical_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/content/123/version"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                { "number": 3, "when": "2026-02-01T00:00:00Z", "message": "typo" },
                { "number": 2, "when": "2026-01-01T00:00:00Z" }
            ],
            "size": 2
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/content/123"))
        .and(query_param("status", "historical"))
        .and(query_param("version", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "123",
            "type": "page",
            "title": "Runbook",
            "version": { "number": 2 },
            "body": { "storage": { "value": "<p>old</p>" } }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let confluence = client(&server);
    let versions = confluence.get_page_versions("123", 25).await.unwrap();
    assert_eq!(versions.results[0].number, 3);
    assert_eq!(versions.results[0].message.as_deref(), Some("typo"));

    let old = confluence.get_page_version_body("123", 2).await.unwrap();
    assert_eq!(old.body.unwrap().storage.unwrap().value, "<p>old</p>");
}

#[tokio::test]
async fn move_page_preserves_title_and_bumps_version() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/content/123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "123",
            "type": "page",
            "title": "Runbook",
            "version": { "number": 4 },
            "body": { "storage": { "value": "<p>body</p>" } }
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/wiki/rest/api/content/123"))
        .and(body_partial_json(json!({
            "title": "Runbook",
            "version": { "number": 5 },
            "ancestors": [{ "id": "999" }],
            "space": { "key": "OPS" }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "123", "type": "page", "title": "Runbook", "version": { "number": 5 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .move_page("123", Some("999"), Some("OPS"))
        .await
        .unwrap();
}

#[tokio::test]
async fn space_pages_query_expands_ancestors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/content/search"))
        .and(query_param("expand", "ancestors,version"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                { "id": "1", "type": "page", "title": "Root" },
                { "id": "2", "type": "page", "title": "Child", "ancestors": [{ "id": "1", "title": "Root" }] }
            ],
            "size": 2
        })))
        .expect(1)
        .mount(&server)
        .await;

    let pages = client(&server).get_space_pages("DEV", 100).await.unwrap();
    assert_eq!(pages.results[1].ancestors[0].id, "1");
}

#[tokio::test]
async fn inline_comment_carries_text_anchor() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/wiki/rest/api/content"))
        .and(body_partial_json(json!({
            "type": "comment",
            "container": { "id": "123", "type": "page" },
            "extensions": {
                "location": "inline",
                "inlineProperties": { "originalSelection": "deploy step" }
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "c1", "type": "comment", "title": ""
        })))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .add_inline_comment("123", "<p>which env?</p>", "deploy step")
        .await
        .unwrap();
}

fn dc_client(server: &MockServer) -> ConfluenceClient {
    ConfluenceClient::new(&ServiceConfig {
        base_url: format!("{}/wiki", server.uri()),
        auth: Auth::Pat {
            token: "pat".into(),
        },
        deployment: None,
    })
    .unwrap()
}

#[tokio::test]
async fn restrictions_are_sent_per_operation_with_the_deployments_user_reference() {
    // Cloud knows users by account id …
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/wiki/rest/api/content/123/restriction"))
        .and(body_partial_json(json!([
            { "operation": "read", "restrictions": { "user": { "results": [{ "accountId": "acc-1" }] } } },
            { "operation": "update", "restrictions": { "group": { "results": [{ "name": "devs" }] } } }
        ])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": [] })))
        .expect(1)
        .mount(&server)
        .await;
    let confluence = client(&server);
    assert!(confluence.is_cloud());
    confluence
        .set_restrictions("123", &["acc-1".into()], &[], &[], &["devs".into()])
        .await
        .unwrap();
    let sent: serde_json::Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert!(sent[0]["restrictions"]["user"]["results"][0]
        .get("username")
        .is_none());

    // … Server/DC by username.
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/wiki/rest/api/content/123/restriction"))
        .and(body_partial_json(json!([
            { "operation": "read", "restrictions": { "user": { "results": [{ "username": "jdoe" }] } } },
            { "operation": "update" }
        ])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": [] })))
        .expect(1)
        .mount(&server)
        .await;
    let confluence = dc_client(&server);
    assert!(!confluence.is_cloud());
    confluence
        .set_restrictions("123", &["jdoe".into()], &[], &[], &[])
        .await
        .unwrap();
}

#[tokio::test]
async fn user_search_goes_through_cql() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/search/user"))
        .and(query_param("cql", "user.fullname ~ \"alice\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{ "user": { "accountId": "acc-1", "displayName": "Alice" } }],
            "size": 1
        })))
        .expect(1)
        .mount(&server)
        .await;

    let users = client(&server).search_users("alice", 10).await.unwrap();
    assert_eq!(users[0].display_name, "Alice");
}
