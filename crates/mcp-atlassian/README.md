# mcp-atlassian

A small, fast [MCP](https://modelcontextprotocol.io) server for **Jira** and
**Confluence** — Cloud and Server/Data Center — written in Rust.

One static binary of about 4 MB, ~2 MB of RSS at idle, a `FROM scratch` Docker
image. 70 tools, prompts, resources, and the safety switches an operator wants:
read-only mode, dry run, confirmation of destructive calls, an audit log and a
sandbox for attachments.

## Install

```bash
cargo install mcp-atlassian --features http
docker pull ghcr.io/d3lph1/mcp-atlassian:latest
```

Prebuilt binaries for Linux, macOS and Windows are on the
[releases page](https://github.com/d3lph1/mcp-atlassian/releases).

## Run

Configured entirely through environment variables, using the names other
Atlassian MCP servers already use:

```bash
JIRA_URL=https://company.atlassian.net \
JIRA_USERNAME=you@example.com \
JIRA_API_TOKEN=... \
mcp-atlassian
```

Either service is optional — tools register only for what is configured.
`--version` and `--help` are the only flags; everything else is the
environment. Speaks MCP over stdin/stdout by default;
`TRANSPORT=streamable-http` serves HTTP instead (needs `--features http`).

The full list of variables, the tool catalogue and the design notes are in the
[repository](https://github.com/d3lph1/mcp-atlassian).

## License

MIT.
