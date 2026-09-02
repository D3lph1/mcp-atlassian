# mcp-atlassian-jira

Jira REST client and MCP tools behind the
[`mcp-atlassian`](https://crates.io/crates/mcp-atlassian) server. Works against
Cloud and Server/Data Center through the same code path.

Part of the `mcp-atlassian` workspace. It is published because the server
depends on it, not as a general-purpose library — the API moves whenever the
server needs it to.

## What it does

REST API **v2** everywhere, so one code path serves Cloud and Server/DC and
bodies stay plain text rather than ADF. Response structs deserialize only the
fields actually used.

Covers issues, search (JQL), transitions, comments and worklogs, links, users
and watchers, fields, agile boards and sprints, and attachments — 40 MCP tools,
plus prompts and `jira://PROJ-123` resources.

## Features

The `mcp` feature adds the tool, prompt and resource layer and pulls in
[`rmcp`](https://crates.io/crates/rmcp). Without it this is a plain REST client
with no MCP dependency.

## Example

```rust
use mcp_atlassian_client::{Auth, ServiceConfig};
use mcp_atlassian_jira::JiraClient;

let client = JiraClient::new(&ServiceConfig {
    base_url: "https://company.atlassian.net".into(),
    auth: Auth::Basic {
        username: "you@example.com".into(),
        token: std::env::var("JIRA_API_TOKEN")?,
    },
    deployment: None,
})?;

let issue = client.get_issue("PROJ-1", None).await?;
```

## License

MIT.
