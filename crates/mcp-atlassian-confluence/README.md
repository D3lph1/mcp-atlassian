# mcp-atlassian-confluence

Confluence REST client and MCP tools behind the
[`mcp-atlassian`](https://crates.io/crates/mcp-atlassian) server. Works against
Cloud and Server/Data Center.

Part of the `mcp-atlassian` workspace. It is published because the server
depends on it, not as a general-purpose library — the API moves whenever the
server needs it to.

## What it does

Pages, search (CQL), comments including inline ones, spaces and labels,
attachments, version history and diffs, templates and page restrictions — 30
MCP tools, plus prompts and `confluence://123456` resources.

Content crosses the boundary as Markdown: storage XHTML is converted on read
and back on write, via
[`mcp-atlassian-storage-markdown`](https://crates.io/crates/mcp-atlassian-storage-markdown).
Section edits happen on the storage document itself, so changing one heading
never round-trips the whole page.

## Features

The `mcp` feature adds the tool, prompt and resource layer and pulls in
[`rmcp`](https://crates.io/crates/rmcp). Without it this is a plain REST client
with no MCP dependency.

## Example

```rust
use mcp_atlassian_client::{Auth, ServiceConfig};
use mcp_atlassian_confluence::ConfluenceClient;

let client = ConfluenceClient::new(&ServiceConfig {
    base_url: "https://company.atlassian.net".into(),
    auth: Auth::Basic {
        username: "you@example.com".into(),
        token: std::env::var("CONFLUENCE_API_TOKEN")?,
    },
    deployment: None,
})?;

let page = client.get_page("123456").await?;
```

## License

MIT.
