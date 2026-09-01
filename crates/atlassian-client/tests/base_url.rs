use atlassian_client::{AtlassianClient, Auth, ServiceConfig};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Confluence Cloud lives under a `/wiki` prefix — the client must preserve
/// it when joining request paths (Url::join drops the last segment of a base
/// without a trailing slash).
#[tokio::test]
async fn base_url_with_path_prefix_is_preserved() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/space"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": true })))
        .expect(1)
        .mount(&server)
        .await;

    let client = AtlassianClient::new(&ServiceConfig {
        base_url: format!("{}/wiki", server.uri()),
        auth: Auth::Pat { token: "t".into() },
    })
    .unwrap();

    let _: serde_json::Value = client.get("/rest/api/space", &[]).await.unwrap();
}
