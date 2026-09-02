use atlassian_client::{Auth, Config, ServiceConfig, ToolFilter};
use mcp_atlassian::server::AtlassianServer;

/// Tools not annotated `readOnlyHint`. Includes the two attachment downloads:
/// they write to the local filesystem, which READ_ONLY must also prevent.
const WRITE_TOOL_COUNT: usize = 32;

fn jira_config() -> ServiceConfig {
    ServiceConfig {
        base_url: "https://example.atlassian.net".into(),
        auth: Auth::Basic {
            username: "u@example.com".into(),
            token: "t".into(),
        },
        deployment: None,
    }
}

fn confluence_config() -> ServiceConfig {
    ServiceConfig {
        base_url: "https://example.atlassian.net/wiki".into(),
        auth: Auth::Basic {
            username: "u@example.com".into(),
            token: "t".into(),
        },
        deployment: None,
    }
}

fn full_config() -> Config {
    Config {
        jira: Some(jira_config()),
        confluence: Some(confluence_config()),
        ..Config::default()
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

fn enabled(patterns: &str) -> Vec<String> {
    selected(patterns, "")
}

/// The tool set an `ENABLED_TOOLS` / `DISABLED_TOOLS` pair produces. An empty
/// string means the variable is unset.
fn selected(enabled: &str, disabled: &str) -> Vec<String> {
    let config = Config {
        enabled_tools: ToolFilter::parse(enabled),
        disabled_tools: ToolFilter::parse(disabled),
        ..full_config()
    };
    AtlassianServer::new(&config).unwrap().tool_names()
}

#[test]
fn enabled_tools_acts_as_allowlist() {
    assert_eq!(
        enabled("jira_search,confluence_search"),
        ["confluence_search", "jira_search"]
    );
}

#[test]
fn a_trailing_wildcard_selects_a_whole_product() {
    let names = enabled("confluence_*");
    assert_eq!(names.len(), 30);
    assert!(
        names.iter().all(|n| n.starts_with("confluence_")),
        "{names:?}"
    );
}

#[test]
fn a_wildcard_may_stand_anywhere_in_the_pattern() {
    // Attachment tools across both products, whatever the verb between them.
    let names = enabled("*_attachment*");
    assert!(names.len() >= 6, "{names:?}");
    assert!(names.iter().all(|n| n.contains("attachment")), "{names:?}");
    assert!(names.iter().any(|n| n.starts_with("jira_")), "{names:?}");
    assert!(
        names.iter().any(|n| n.starts_with("confluence_")),
        "{names:?}"
    );

    // A verb shared by both products, with the wildcard on both sides.
    let names = enabled("*_search");
    assert_eq!(names, ["confluence_search", "jira_search"]);
}

#[test]
fn patterns_and_exact_names_mix_in_one_list() {
    let names = enabled("jira_get_*, confluence_search");
    assert!(
        names.contains(&"confluence_search".to_string()),
        "{names:?}"
    );
    assert!(names.contains(&"jira_get_issue".to_string()), "{names:?}");
    assert!(
        !names.contains(&"jira_create_issue".to_string()),
        "{names:?}"
    );
}

#[test]
fn a_lone_wildcard_is_the_same_as_no_filter() {
    assert_eq!(enabled("*").len(), 70);
}

#[test]
fn read_only_wins_over_allowlist() {
    let config = Config {
        enabled_tools: ToolFilter::parse("jira_create_issue"),
        disabled_tools: None,
        read_only: true,
        dry_run: false,
        ..full_config()
    };
    let server = AtlassianServer::new(&config).unwrap();
    assert!(server.tool_names().is_empty());
}

#[test]
fn disabled_tools_subtracts_from_the_allowlist() {
    let names = selected("jira_*", "*_delete_*,*_attachment*");
    assert!(names.iter().all(|n| n.starts_with("jira_")), "{names:?}");
    assert!(!names.iter().any(|n| n.contains("delete")), "{names:?}");
    assert!(!names.iter().any(|n| n.contains("attachment")), "{names:?}");
    assert!(names.contains(&"jira_search".to_string()), "{names:?}");
}

#[test]
fn disabled_tools_works_without_an_allowlist() {
    // The common shape: everything except one product's writes.
    let names = selected("", "confluence_*");
    assert_eq!(names.len(), 40);
    assert!(names.iter().all(|n| n.starts_with("jira_")), "{names:?}");
}

#[test]
fn the_denylist_wins_over_the_allowlist() {
    // Named by both, so it goes. Otherwise the pair would depend on which
    // variable the reader looked at first.
    assert!(selected("jira_delete_issue", "jira_delete_issue").is_empty());
    assert!(selected("jira_*", "*").is_empty());
}

#[test]
fn an_exact_name_in_the_denylist_carves_one_tool_out_of_a_wildcard() {
    let names = selected("jira_*", "jira_delete_issue");
    assert_eq!(names.len(), 39);
    assert!(
        !names.contains(&"jira_delete_issue".to_string()),
        "{names:?}"
    );
    assert!(
        names.contains(&"jira_create_issue".to_string()),
        "{names:?}"
    );
}

#[test]
fn read_only_and_the_denylist_compose() {
    let config = Config {
        disabled_tools: ToolFilter::parse("confluence_*"),
        read_only: true,
        ..full_config()
    };
    let names = AtlassianServer::new(&config).unwrap().tool_names();
    assert!(names.iter().all(|n| n.starts_with("jira_")), "{names:?}");
    assert!(
        !names.contains(&"jira_create_issue".to_string()),
        "{names:?}"
    );
    assert!(names.contains(&"jira_search".to_string()), "{names:?}");
}
