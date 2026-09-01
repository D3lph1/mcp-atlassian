use std::sync::Arc;

use atlassian_client::oauth::{OAuthConfig, OAuthSession};
use atlassian_client::{AtlassianClient, Auth, Error, ServiceConfig};
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn oauth_client(server: &MockServer) -> AtlassianClient {
    let session = OAuthSession::new(OAuthConfig {
        client_id: "cid".into(),
        client_secret: "csecret".into(),
        refresh_token: "refresh-1".into(),
        token_url: format!("{}/oauth/token", server.uri()),
    })
    .unwrap();
    AtlassianClient::new(&ServiceConfig {
        base_url: format!("{}/ex/jira/cloud-1", server.uri()),
        auth: Auth::OAuth(Arc::new(session)),
    })
    .unwrap()
}

#[tokio::test]
async fn refreshes_once_and_caches_access_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_partial_json(json!({
            "grant_type": "refresh_token",
            "client_id": "cid",
            "refresh_token": "refresh-1"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "access-1",
            "expires_in": 3600,
            "refresh_token": "refresh-2"
        })))
        .expect(1) // cached for the second API call
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/ex/jira/cloud-1/rest/api/2/myself"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": true })))
        .expect(2)
        .mount(&server)
        .await;

    let client = oauth_client(&server);
    let _: serde_json::Value = client.get("/rest/api/2/myself", &[]).await.unwrap();
    let _: serde_json::Value = client.get("/rest/api/2/myself", &[]).await.unwrap();
}

#[tokio::test]
async fn expired_token_triggers_refresh_with_rotated_token() {
    let server = MockServer::start().await;
    // First refresh: token that expires immediately, rotates the refresh token.
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_partial_json(json!({ "refresh_token": "refresh-1" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "access-1",
            "expires_in": 0,
            "refresh_token": "refresh-2"
        })))
        .expect(1)
        .mount(&server)
        .await;
    // Second refresh must use the rotated token.
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_partial_json(json!({ "refresh_token": "refresh-2" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "access-2",
            "expires_in": 3600
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/ex/jira/cloud-1/rest/api/2/myself"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": true })))
        .mount(&server)
        .await;

    let client = oauth_client(&server);
    let _: serde_json::Value = client.get("/rest/api/2/myself", &[]).await.unwrap();
    let _: serde_json::Value = client.get("/rest/api/2/myself", &[]).await.unwrap();
}

#[tokio::test]
async fn failed_refresh_is_actionable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(403).set_body_string("invalid_grant"))
        .mount(&server)
        .await;

    let client = oauth_client(&server);
    let err = client
        .get::<serde_json::Value>("/rest/api/2/myself", &[])
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(matches!(err, Error::OAuth(_)), "got: {err:?}");
    assert!(msg.contains("ATLASSIAN_OAUTH"), "not actionable: {msg}");
}
