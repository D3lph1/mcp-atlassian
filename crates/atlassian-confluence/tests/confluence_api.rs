use atlassian_client::{Auth, ServiceConfig};
use atlassian_confluence::ConfluenceClient;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Cloud-style client: base URL carries the `/wiki` prefix.
fn client(server: &MockServer) -> ConfluenceClient {
    ConfluenceClient::new(&ServiceConfig {
        base_url: format!("{}/wiki", server.uri()),
        auth: Auth::Basic {
            username: "u@example.com".into(),
            token: "secret".into(),
        },
        deployment: None,
    })
    .unwrap()
}

fn page_json(id: &str, title: &str, version: u64) -> serde_json::Value {
    json!({
        "id": id,
        "type": "page",
        "title": title,
        "space": { "key": "DEV", "name": "Development" },
        "version": { "number": version },
        "body": { "storage": { "value": "<p>hello</p>", "representation": "storage" } }
    })
}

#[tokio::test]
async fn search_sends_cql_under_wiki_prefix() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/content/search"))
        .and(query_param("cql", "space = DEV"))
        .and(query_param("limit", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [page_json("123", "Runbook", 3)],
            "size": 1
        })))
        .expect(1)
        .mount(&server)
        .await;

    let page = client(&server).search("space = DEV", 10, 0).await.unwrap();
    assert_eq!(page.results.len(), 1);
    assert_eq!(page.results[0].title, "Runbook");
    assert_eq!(page.results[0].space.as_ref().unwrap().key, "DEV");
}

#[tokio::test]
async fn get_page_expands_storage_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/content/123"))
        .and(query_param("expand", "body.storage,version,space"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page_json("123", "Runbook", 3)))
        .expect(1)
        .mount(&server)
        .await;

    let page = client(&server).get_page("123").await.unwrap();
    assert_eq!(page.body.unwrap().storage.unwrap().value, "<p>hello</p>");
    assert_eq!(page.version.unwrap().number, 3);
}

#[tokio::test]
async fn create_page_posts_storage_body_with_parent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/wiki/rest/api/content"))
        .and(body_partial_json(json!({
            "type": "page",
            "title": "New page",
            "space": { "key": "DEV" },
            "ancestors": [{ "id": "42" }],
            "body": { "storage": { "value": "<p>body</p>", "representation": "storage" } }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(page_json("124", "New page", 1)))
        .expect(1)
        .mount(&server)
        .await;

    let page = client(&server)
        .create_page("DEV", "New page", "<p>body</p>", Some("42"))
        .await
        .unwrap();
    assert_eq!(page.id, "124");
}

#[tokio::test]
async fn update_page_fetches_version_and_increments() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/content/123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page_json("123", "Runbook", 3)))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/wiki/rest/api/content/123"))
        .and(body_partial_json(json!({
            "title": "Runbook v2",
            "version": { "number": 4 },
            "body": { "storage": { "value": "<p>updated</p>" } }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(page_json("123", "Runbook v2", 4)))
        .expect(1)
        .mount(&server)
        .await;

    let page = client(&server)
        .update_page("123", Some("Runbook v2"), Some("<p>updated</p>"))
        .await
        .unwrap();
    assert_eq!(page.version.unwrap().number, 4);
}

#[tokio::test]
async fn update_page_keeps_existing_title_and_body_when_omitted() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/content/123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page_json("123", "Runbook", 3)))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/wiki/rest/api/content/123"))
        .and(body_partial_json(json!({
            "title": "Runbook",
            "version": { "number": 4 },
            "body": { "storage": { "value": "<p>hello</p>" } }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(page_json("123", "Runbook", 4)))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .update_page("123", None, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn add_comment_posts_container_reference() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/wiki/rest/api/content"))
        .and(body_partial_json(json!({
            "type": "comment",
            "container": { "id": "123", "type": "page" }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "999",
            "type": "comment",
            "title": ""
        })))
        .expect(1)
        .mount(&server)
        .await;

    let comment = client(&server)
        .add_comment("123", "<p>looks good</p>")
        .await
        .unwrap();
    assert_eq!(comment.id, "999");
}

#[tokio::test]
async fn labels_roundtrip() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/wiki/rest/api/content/123/label"))
        .and(body_partial_json(
            json!([{ "prefix": "global", "name": "ops" }]),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{ "name": "ops", "prefix": "global" }],
            "size": 1
        })))
        .expect(1)
        .mount(&server)
        .await;

    let labels = client(&server).add_label("123", "ops").await.unwrap();
    assert_eq!(labels.results[0].name, "ops");
}

#[tokio::test]
async fn pages_report_where_they_are_and_whether_more_exist() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wiki/rest/api/content/123/child/page"))
        .and(query_param("limit", "2"))
        .and(query_param("start", "4"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                { "id": "1", "title": "A", "type": "page" },
                { "id": "2", "title": "B", "type": "page" }
            ],
            "start": 4, "limit": 2, "size": 2,
            "_links": { "next": "/rest/api/content/123/child/page?start=6&limit=2" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let page = client(&server)
        .get_page_children("123", 2, 4)
        .await
        .unwrap();
    assert_eq!((page.start, page.limit, page.size), (4, 2, 2));
    assert!(page.has_more);

    // The last page: no next link, fewer results than the limit.
    let page: atlassian_confluence::ResultsPage<atlassian_confluence::Label> =
        serde_json::from_value(json!({
            "results": [{ "name": "ops" }], "start": 0, "limit": 25, "size": 1
        }))
        .unwrap();
    assert!(!page.has_more);
    // An envelope without `size` (some endpoints omit it) counts its results.
    let page: atlassian_confluence::ResultsPage<atlassian_confluence::Label> =
        serde_json::from_value(json!({ "results": [{ "name": "a" }, { "name": "b" }] })).unwrap();
    assert_eq!(page.size, 2);
    assert!(!page.has_more);
    // No next link but a full page: assume more, because Confluence's
    // link map is not always present.
    let page: atlassian_confluence::ResultsPage<atlassian_confluence::Label> =
        serde_json::from_value(json!({
            "results": [{ "name": "a" }], "start": 0, "limit": 1, "size": 1
        }))
        .unwrap();
    assert!(page.has_more);
}
