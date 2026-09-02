use mcp_atlassian_client::{AtlassianClient, Auth, ServiceConfig};
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
        deployment: None,
    })
    .unwrap();

    let _: serde_json::Value = client.get("/rest/api/space", &[]).await.unwrap();
}

mod streaming_and_retries {
    use mcp_atlassian_client::{AtlassianClient, Auth, Error, ServiceConfig};
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> AtlassianClient {
        AtlassianClient::new(&ServiceConfig {
            base_url: server.uri(),
            auth: Auth::Pat { token: "t".into() },
            deployment: None,
        })
        .unwrap()
    }

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("mcp-stream-{name}-{}", std::process::id()))
    }

    #[tokio::test]
    async fn a_download_is_written_to_the_file_and_its_size_reported() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/download/a.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![7u8; 10_000]))
            .mount(&server)
            .await;
        let target = temp("ok");
        let size = client(&server)
            .download_to_file("/download/a.bin?version=1", &target, Some(1 << 20))
            .await
            .unwrap();
        assert_eq!(size, 10_000);
        assert_eq!(std::fs::metadata(&target).unwrap().len(), 10_000);
        let _ = std::fs::remove_file(&target);
    }

    #[tokio::test]
    async fn a_download_over_the_limit_is_refused_and_leaves_no_partial_file() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/download/big.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 5_000]))
            .mount(&server)
            .await;
        let target = temp("big");
        let error = client(&server)
            .download_to_file("/download/big.bin", &target, Some(1_000))
            .await
            .unwrap_err();
        assert!(matches!(error, Error::File(_)), "{error:?}");
        assert!(
            error.to_string().contains("MAX_ATTACHMENT_BYTES"),
            "{error}"
        );
        assert!(!target.exists(), "partial file left behind");
    }

    #[tokio::test]
    async fn a_get_is_retried_after_a_503_and_a_post_is_not() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/api/2/myself"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/rest/api/2/myself"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": true })))
            .mount(&server)
            .await;
        let value: serde_json::Value = client(&server)
            .get("/rest/api/2/myself", &[])
            .await
            .unwrap();
        assert_eq!(value["ok"], true);

        // A POST that failed at the transport may have landed; it is not
        // replayed.
        Mock::given(method("POST"))
            .and(path("/rest/api/2/issue"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        let error = client(&server)
            .post::<serde_json::Value, _>("/rest/api/2/issue", &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Api { status: 503, .. }), "{error:?}");
    }

    #[tokio::test]
    async fn a_get_gives_up_after_the_retry_budget() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/api/2/myself"))
            .respond_with(ResponseTemplate::new(502))
            .expect(3)
            .mount(&server)
            .await;
        let error = client(&server)
            .get::<serde_json::Value>("/rest/api/2/myself", &[])
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Api { status: 502, .. }), "{error:?}");
    }
}
