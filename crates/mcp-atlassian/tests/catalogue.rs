//! `--list-tools` prints the whole surface, and prints it truthfully.
//!
//! Enumerated over the routers rather than written out tool by tool (D32):
//! adding a tool must not require editing this file, but must not be able to
//! slip out of the catalogue either.

use mcp_atlassian::catalogue;

/// Every tool the build has appears, and nothing else does.
#[test]
fn the_catalogue_lists_every_tool() {
    let rendered = catalogue::render();

    let names: Vec<String> = mcp_atlassian_jira::tools::router()
        .list_all()
        .into_iter()
        .chain(mcp_atlassian_confluence::tools::router().list_all())
        .map(|tool| tool.name.to_string())
        .collect();

    for name in &names {
        assert!(
            rendered.contains(name.as_str()),
            "{name} is missing from the catalogue"
        );
    }
    assert!(
        rendered.contains(&format!("{} tools", names.len())),
        "the count should match the {} tools that exist:\n{rendered}",
        names.len()
    );
}

/// The listed kind comes from the annotation that `READ_ONLY`, the audit log
/// and `DRY_RUN` also read, so a tool cannot look read-only here and behave
/// as a write there.
#[test]
fn a_tool_is_listed_as_what_its_annotation_says() {
    let rendered = catalogue::render();

    for tool in mcp_atlassian_jira::tools::router().list_all() {
        let annotations = tool.annotations.as_ref().expect("annotated");
        let expected = if annotations.read_only_hint == Some(true) {
            "read-only"
        } else if annotations.destructive_hint == Some(true) {
            "destructive"
        } else {
            "write"
        };
        let line = rendered
            .lines()
            .find(|line| line.split_whitespace().next() == Some(&tool.name))
            .unwrap_or_else(|| panic!("{} is not listed", tool.name));
        assert!(
            line.contains(expected),
            "{} should be listed as {expected}: {line}",
            tool.name
        );
    }
}

/// Both products are named, with their counts — the grouping is the reason
/// the listing is readable at 70 entries.
#[test]
fn products_are_grouped_and_counted() {
    let rendered = catalogue::render();
    let jira = mcp_atlassian_jira::tools::router().list_all().len();
    let confluence = mcp_atlassian_confluence::tools::router().list_all().len();

    assert!(rendered.contains(&format!("Jira ({jira})")), "{rendered}");
    assert!(
        rendered.contains(&format!("Confluence ({confluence})")),
        "{rendered}"
    );
}
