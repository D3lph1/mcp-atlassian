//! Startup warnings are collected by the constructor and printed by `main`
//! after the banner, never logged from `AtlassianServer::new` (D29).
//!
//! What is asserted here is that the warning is *returned* — the ordering
//! itself belongs to `main` and is not covered. Note the gap: a
//! `tracing::warn!` put back into `new` would print above the banner again
//! and every test below would still pass. Capturing the subscriber to close
//! that would cost more than the bug is worth; the field's comment in
//! `server.rs` is what carries the rule.

use std::path::PathBuf;

use atlassian_client::{Auth, Config, ServiceConfig, ToolFilter};
use mcp_atlassian::server::AtlassianServer;

fn service() -> ServiceConfig {
    ServiceConfig {
        base_url: "https://example.atlassian.net".into(),
        auth: Auth::Basic {
            username: "u@example.com".into(),
            token: "t".into(),
        },
        deployment: None,
    }
}

fn config() -> Config {
    Config {
        jira: Some(service()),
        confluence: Some(service()),
        ..Config::default()
    }
}

#[test]
fn unset_attachment_dir_is_reported_once() {
    let server = AtlassianServer::new(&config()).unwrap();
    let warnings = server.startup_warnings();

    let matching: Vec<_> = warnings
        .iter()
        .filter(|w| w.contains("ATTACHMENT_DIR"))
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one ATTACHMENT_DIR warning, got {warnings:?}"
    );
    assert!(
        matching[0].contains("any path this process can reach"),
        "the warning should say what is at stake: {}",
        matching[0]
    );
}

#[test]
fn a_restricted_attachment_dir_warns_about_nothing() {
    let config = Config {
        attachment_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR"))),
        ..config()
    };
    let server = AtlassianServer::new(&config).unwrap();

    assert!(
        server.startup_warnings().is_empty(),
        "a sandboxed configuration has nothing to warn about: {:?}",
        server.startup_warnings()
    );
}

/// Without attachment tools there is no path for the model to name, so the
/// warning would be noise — the sandbox it asks for guards tools that are
/// not registered.
#[test]
fn no_attachment_tools_means_no_warning() {
    let config = Config {
        disabled_tools: ToolFilter::parse("*_attachment*"),
        ..config()
    };
    let server = AtlassianServer::new(&config).unwrap();

    assert!(
        server.startup_warnings().is_empty(),
        "no attachment tool survived filtering: {:?}",
        server.startup_warnings()
    );
}
