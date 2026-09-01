use std::collections::HashSet;

use atlassian_client::{Auth, Config, ServiceConfig};
use mcp_atlassian::server::AtlassianServer;

/// Kept in sync with WRITE_TOOLS in server.rs.
const WRITE_TOOL_COUNT: usize = 30;

fn jira_config() -> ServiceConfig {
    ServiceConfig {
        base_url: "https://example.atlassian.net".into(),
        auth: Auth::Basic {
            username: "u@example.com".into(),
            token: "t".into(),
        },
    }
}

fn confluence_config() -> ServiceConfig {
    ServiceConfig {
        base_url: "https://example.atlassian.net/wiki".into(),
        auth: Auth::Basic {
            username: "u@example.com".into(),
            token: "t".into(),
        },
    }
}

fn full_config() -> Config {
    Config {
        jira: Some(jira_config()),
        confluence: Some(confluence_config()),
        enabled_tools: None,
        read_only: false,
        dry_run: false,
        audit_log: None,
        cache_ttl: None,
    }
}

#[test]
fn all_tools_registered_with_both_services() {
    let server = AtlassianServer::new(&full_config()).unwrap();
    assert_eq!(server.tool_names().len(), 70);
}

#[test]
fn unconfigured_service_tools_are_absent() {
    let config = Config {
        confluence: None,
        ..full_config()
    };
    let server = AtlassianServer::new(&config).unwrap();
    let names = server.tool_names();
    assert_eq!(names.len(), 40);
    assert!(names.iter().all(|n| n.starts_with("jira_")), "{names:?}");
}

#[test]
fn read_only_mode_removes_all_write_tools() {
    let config = Config {
        read_only: true,
        dry_run: false,
        ..full_config()
    };
    let server = AtlassianServer::new(&config).unwrap();
    let names = server.tool_names();
    assert_eq!(names.len(), 70 - WRITE_TOOL_COUNT, "{names:?}");
    for forbidden in [
        "jira_create_issue",
        "jira_update_issue",
        "jira_delete_issue",
        "jira_transition_issue",
        "jira_add_comment",
        "jira_add_worklog",
        "jira_move_to_sprint",
        "jira_upload_attachment",
        "confluence_create_page",
        "confluence_update_page",
        "confluence_delete_page",
        "confluence_add_comment",
        "confluence_add_label",
    ] {
        assert!(
            !names.contains(&forbidden.to_string()),
            "{forbidden} leaked"
        );
    }
    assert!(names.contains(&"jira_search".to_string()));
    assert!(names.contains(&"confluence_get_page".to_string()));
}

#[test]
fn enabled_tools_acts_as_allowlist() {
    let config = Config {
        enabled_tools: Some(HashSet::from([
            "jira_search".to_string(),
            "confluence_search".to_string(),
        ])),
        ..full_config()
    };
    let server = AtlassianServer::new(&config).unwrap();
    assert_eq!(
        server.tool_names(),
        vec!["confluence_search".to_string(), "jira_search".to_string()]
    );
}

#[test]
fn read_only_wins_over_allowlist() {
    let config = Config {
        enabled_tools: Some(HashSet::from(["jira_create_issue".to_string()])),
        read_only: true,
        dry_run: false,
        ..full_config()
    };
    let server = AtlassianServer::new(&config).unwrap();
    assert!(server.tool_names().is_empty());
}
