# mcp-atlassian

[![CI](https://github.com/d3lph1/mcp-atlassian/actions/workflows/ci.yml/badge.svg)](https://github.com/d3lph1/mcp-atlassian/actions/workflows/ci.yml)
[![Coverage](https://coveralls.io/repos/github/d3lph1/mcp-atlassian/badge.svg?branch=master)](https://coveralls.io/github/d3lph1/mcp-atlassian?branch=master)

A small, fast [MCP](https://modelcontextprotocol.io) server for **Jira** and
**Confluence** — Cloud and Server/Data Center — written in Rust.

One static binary of about 4 MB, ~2 MB of RSS at idle, a `FROM scratch`
Docker image. 70 tools, prompts, resources, and the safety switches an
operator actually wants: read-only mode, dry run, confirmation of
destructive calls, an audit log, and a sandbox for attachments.

## Install

Download a binary from [Releases](https://github.com/d3lph1/mcp-atlassian/releases)
(Linux x86_64/arm64 static, macOS x86_64/arm64, Windows x86_64), or:

```bash
cargo install --git https://github.com/d3lph1/mcp-atlassian mcp-atlassian --features http
docker build -t mcp-atlassian .   # ~5 MB scratch image
```

## Configure

The server is configured with environment variables — the same names other
Atlassian MCP servers use, so an existing client config carries over.

```json
{
  "mcpServers": {
    "atlassian": {
      "command": "/usr/local/bin/mcp-atlassian",
      "env": {
        "JIRA_URL": "https://your-company.atlassian.net",
        "JIRA_USERNAME": "you@company.com",
        "JIRA_API_TOKEN": "…",
        "CONFLUENCE_URL": "https://your-company.atlassian.net/wiki",
        "CONFLUENCE_USERNAME": "you@company.com",
        "CONFLUENCE_API_TOKEN": "…"
      }
    }
  }
}
```

Server/Data Center uses a personal access token instead
(`JIRA_PERSONAL_TOKEN`, `CONFLUENCE_PERSONAL_TOKEN`). Either product is
optional. Any token can be read from a file with the `*_FILE` suffix
(`JIRA_API_TOKEN_FILE=/run/secrets/jira`), which is what Docker and
Kubernetes secrets expect. OAuth 2.0 (Cloud) works through
`ATLASSIAN_OAUTH_{CLIENT_ID,CLIENT_SECRET,REFRESH_TOKEN,CLOUD_ID}`.

The switches worth knowing:

| Variable | Effect |
|---|---|
| `READ_ONLY=true` | write tools are not registered at all |
| `DRY_RUN=true` | writes are validated and described, never sent |
| `CONFIRM_DESTRUCTIVE=true` | deletes, transitions and the like ask the user first (MCP elicitation) |
| `ENABLED_TOOLS` / `DISABLED_TOOLS` | wildcards: `jira_*`, `*_get_*`; deny wins |
| `AUDIT_LOG_FILE` | one JSONL line per write: tool, arguments, outcome, what it created |
| `ATTACHMENT_DIR` | the only directory attachment tools may read or write |
| `CACHE_TTL` | seconds to cache projects, fields, boards, spaces (off by default) |
| `JIRA_DEPLOYMENT` | `cloud` or `server` when the auth mode does not tell |
| `TRANSPORT=streamable-http` | HTTP instead of stdio; `HOST`, `PORT`, `MCP_BEARER_TOKEN` |

The full list is in [CLAUDE.md](CLAUDE.md).

## What the model gets

- **Tools** — Jira: search (JQL), issues, transitions, comments, worklog,
  links and epics, fields and their options, agile boards and sprints,
  watchers, attachments. Confluence: search (CQL), pages with section
  edits and moves, comments and inline comments, spaces and labels,
  attachments, version history and diffs, templates, restrictions. Every
  tool has a typed output schema and honest annotations (read-only,
  destructive, idempotent).
- **Content as Markdown** — Confluence storage format is converted both
  ways, including code blocks, panels, links, images and task lists; raw
  macros written in Markdown pass through.
- **Resources** — `jira://PROJ-123`, `jira://PROJ-123/comments`,
  `confluence://123456`, `confluence://123456/comments`.
- **Prompts** — `/jira_issue PROJ-123`, `/jira_triage PROJ`,
  `/jira_standup <board>`, `/confluence_page 123456`: each fetches its data
  and ends with a bounded ask.
- **Completion** of issue keys in prompts and resource templates.

## Design

Deployment is inferred from the auth mode (API token → Cloud, PAT →
Server/DC) and can be overridden. Jira uses REST v2 everywhere, so one code
path serves both. Endpoint paths are guarded against interpolated
identifiers, page sizes are capped, retries are bounded and only for what is
safe to replay, and every environment variable is read in one place.

The reasons behind these and forty other decisions — and what is
deliberately not done — are in [DECISIONS.md](DECISIONS.md).

## Develop

```bash
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release --features http
```

Tests never touch a real Atlassian instance; they run against wiremock and
an in-memory MCP transport.

## License and trademarks

MIT.

Jira, Confluence and Atlassian are trademarks of Atlassian Pty Ltd, and all
rights to them belong to Atlassian. This project is an independent,
community-built tool: it is not developed, endorsed or supported by
Atlassian.
