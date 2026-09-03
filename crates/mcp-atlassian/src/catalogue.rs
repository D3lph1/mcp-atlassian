//! `--list-tools`: the tool surface, printed without configuring anything.
//!
//! The point is that it works on an unconfigured machine — someone deciding
//! whether to install this should not have to produce an API token first. So
//! the catalogue is read from the routers' own metadata, which exists before
//! any client does, and it lists every tool the build has rather than the
//! ones a particular configuration would register. `READ_ONLY`,
//! `ENABLED_TOOLS` and the rest narrow that set at startup (D22, D27); the
//! startup log says what survived, and this says what there was to begin
//! with.

/// What a tool does to the world, from the annotations that also drive
/// `READ_ONLY`, the audit log and `DRY_RUN` (D22). An unannotated tool counts
/// as a write here for the same reason it does there.
fn kind(tool: &rmcp::model::Tool) -> &'static str {
    let annotations = tool.annotations.as_ref();
    if annotations.and_then(|a| a.read_only_hint) == Some(true) {
        "read-only"
    } else if annotations.and_then(|a| a.destructive_hint) == Some(true) {
        "destructive"
    } else {
        "write"
    }
}

/// The tool catalogue as printable text: one line per tool, grouped by
/// product, with what it changes and its title.
pub fn render() -> String {
    let products = [
        ("Jira", mcp_atlassian_jira::tools::router().list_all()),
        (
            "Confluence",
            mcp_atlassian_confluence::tools::router().list_all(),
        ),
    ];

    let total: usize = products.iter().map(|(_, tools)| tools.len()).sum();
    // One column width for the whole listing, so names line up across
    // products rather than shifting at the group boundary.
    let width = products
        .iter()
        .flat_map(|(_, tools)| tools.iter())
        .map(|tool| tool.name.len())
        .max()
        .unwrap_or(0);

    let mut out = format!(
        "mcp-atlassian {} — {total} tools\n",
        env!("CARGO_PKG_VERSION")
    );
    for (product, mut tools) in products {
        if tools.is_empty() {
            continue;
        }
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        out.push_str(&format!("\n{product} ({})\n", tools.len()));
        for tool in tools {
            let title = tool.title.as_deref().unwrap_or("");
            out.push_str(&format!(
                "  {:width$}  {:<11}  {title}\n",
                tool.name,
                kind(&tool),
                width = width
            ));
        }
    }
    out
}

/// The catalogue as JSON: an array of objects, one per tool, for scripts and
/// other programs — the text form is for people.
pub fn render_json() -> String {
    let tools: Vec<serde_json::Value> = ["jira", "confluence"]
        .into_iter()
        .zip([
            mcp_atlassian_jira::tools::router().list_all(),
            mcp_atlassian_confluence::tools::router().list_all(),
        ])
        .flat_map(|(product, mut tools)| {
            tools.sort_by(|a, b| a.name.cmp(&b.name));
            tools.into_iter().map(move |tool| {
                serde_json::json!({
                    "name": tool.name,
                    "product": product,
                    "kind": kind(&tool),
                    "title": tool.title,
                    "description": tool.description,
                })
            })
        })
        .collect();
    serde_json::to_string_pretty(&tools).expect("plain strings serialize")
}
