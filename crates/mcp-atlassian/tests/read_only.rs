//! `READ_ONLY_MODE`: only tools annotated `readOnlyHint: true` are registered.
//!
//! The annotation is the single source of truth, so these tests double as a
//! guard — a new tool that forgets it is treated as a write and shows up here.

use std::collections::HashSet;

use atlassian_client::{Auth, Config, ServiceConfig};
use mcp_atlassian::server::AtlassianServer;

fn service() -> ServiceConfig {
    ServiceConfig {
        base_url: "https://example.atlassian.net".into(),
        auth: Auth::Basic {
            username: "u@example.com".into(),
            token: "t".into(),
        },
    }
}

fn config(read_only: bool) -> Config {
    Config {
        jira: Some(service()),
        confluence: Some(service()),
        enabled_tools: None,
        read_only,
        audit_log: None,
    }
}

/// Names that clearly mutate state, whatever their annotation claims. Keeps an
/// independent check on the annotations themselves.
fn looks_like_a_write(name: &str) -> bool {
    const MUTATING: &[&str] = &[
        "_create_",
        "_update_",
        "_delete_",
        "_add_",
        "_remove_",
        "_edit_",
        "_upload_",
        "_assign_",
        "_move_",
        "_set_",
        "_transition_",
        "_reply_",
        "_link_to_",
    ];
    // `_get_transitions` and `_search_assignable_users` read; match on the
    // verb position rather than a bare substring.
    MUTATING.iter().any(|verb| name.contains(verb))
        && !name.contains("_get_")
        && !name.contains("_search_")
        && !name.contains("_list_")
}

#[test]
fn every_tool_declares_whether_it_is_read_only() {
    let server = AtlassianServer::new(&config(false)).unwrap();
    for tool in server.tools() {
        let hint = tool.annotations.as_ref().and_then(|a| a.read_only_hint);
        assert!(
            hint.is_some(),
            "{} has no readOnlyHint — READ_ONLY_MODE would treat it as a write",
            tool.name
        );
    }
}

#[test]
fn annotations_agree_with_what_the_tool_names_imply() {
    let server = AtlassianServer::new(&config(false)).unwrap();
    for tool in server.tools() {
        let read_only = tool
            .annotations
            .as_ref()
            .and_then(|a| a.read_only_hint)
            .unwrap_or(false);
        assert_eq!(
            read_only,
            !looks_like_a_write(&tool.name),
            "{} is annotated read_only={read_only}, which contradicts its name",
            tool.name
        );
    }
}

#[test]
fn read_only_mode_registers_only_read_tools() {
    let full = AtlassianServer::new(&config(false)).unwrap();
    let read_only = AtlassianServer::new(&config(true)).unwrap();

    let allowed: HashSet<String> = read_only.tool_names().into_iter().collect();
    assert!(!allowed.is_empty(), "read-only mode registered nothing");
    assert!(
        allowed.len() < full.tool_names().len(),
        "read-only mode registered everything"
    );

    // Nothing that mutates survives.
    for name in &allowed {
        assert!(
            !looks_like_a_write(name),
            "write tool {name} is available in READ_ONLY_MODE"
        );
    }

    // Every read tool does survive — the mode restricts, it does not prune.
    for tool in full.tools() {
        let read_only_tool = tool
            .annotations
            .as_ref()
            .and_then(|a| a.read_only_hint)
            .unwrap_or(false);
        if read_only_tool {
            assert!(
                allowed.contains(tool.name.as_ref()),
                "read tool {} went missing in READ_ONLY_MODE",
                tool.name
            );
        }
    }
}

#[test]
fn destructive_tools_are_flagged_for_clients() {
    // Clients use destructiveHint to decide when to ask for confirmation.
    let server = AtlassianServer::new(&config(false)).unwrap();
    let destructive: HashSet<String> = server
        .tools()
        .into_iter()
        .filter(|t| {
            t.annotations
                .as_ref()
                .and_then(|a| a.destructive_hint)
                .unwrap_or(false)
        })
        .map(|t| t.name.to_string())
        .collect();

    for name in [
        "jira_delete_issue",
        "confluence_delete_page",
        "confluence_delete_attachment",
        "confluence_set_page_restrictions",
    ] {
        assert!(
            destructive.contains(name),
            "{name} should be marked destructive"
        );
    }
    // Additive writes must not be marked destructive.
    assert!(!destructive.contains("jira_add_comment"));
    assert!(!destructive.contains("jira_create_issue"));
}
