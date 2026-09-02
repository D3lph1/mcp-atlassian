use mcp_atlassian_client::{Auth, Error, ServiceConfig};
use mcp_atlassian_jira::JiraClient;
use wiremock::matchers::{header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config(server: &MockServer, auth: Auth) -> ServiceConfig {
    ServiceConfig {
        base_url: server.uri(),
        auth,
        deployment: None,
    }
}

#[tokio::test]
async fn get_myself_cloud_parses_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/myself"))
        .and(header_exists("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "accountId": "5b10a2844c20165700ede21g",
            "displayName": "Mia Krystof",
            "emailAddress": "mia@example.com",
            "timeZone": "Australia/Sydney",
            "active": true,
            "avatarUrls": {"48x48": "https://ignored.example/avatar.png"}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let jira = JiraClient::new(&config(
        &server,
        Auth::Basic {
            username: "mia@example.com".into(),
            token: "secret".into(),
        },
    ))
    .unwrap();

    let myself = jira.get_myself().await.unwrap();
    assert_eq!(
        myself.account_id.as_deref(),
        Some("5b10a2844c20165700ede21g")
    );
    assert_eq!(myself.display_name, "Mia Krystof");
    assert!(myself.active);
    // Server/DC-only field absent on Cloud:
    assert_eq!(myself.name, None);
}

#[tokio::test]
async fn get_myself_maps_401_to_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/myself"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let jira = JiraClient::new(&config(
        &server,
        Auth::Pat {
            token: "bad".into(),
        },
    ))
    .unwrap();
    let err = jira.get_myself().await.unwrap_err();
    assert!(matches!(err, Error::Unauthorized), "got: {err:?}");
}

#[tokio::test]
async fn retries_once_on_429_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/myself"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/myself"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "admin",
            "displayName": "Admin",
            "active": true
        })))
        .expect(1)
        .mount(&server)
        .await;

    let jira = JiraClient::new(&config(
        &server,
        Auth::Pat {
            token: "pat".into(),
        },
    ))
    .unwrap();
    let myself = jira.get_myself().await.unwrap();
    // Server/DC identifies users by `name`:
    assert_eq!(myself.name.as_deref(), Some("admin"));
}
