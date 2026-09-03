# mcp-atlassian

[![CI](https://github.com/d3lph1/mcp-atlassian/actions/workflows/ci.yml/badge.svg)](https://github.com/d3lph1/mcp-atlassian/actions/workflows/ci.yml)
[![Coverage](https://coveralls.io/repos/github/D3lph1/mcp-atlassian/badge.svg?branch=master)](https://coveralls.io/github/D3lph1/mcp-atlassian?branch=master)
[![crates.io](https://img.shields.io/crates/v/mcp-atlassian.svg)](https://crates.io/crates/mcp-atlassian)
[![License](https://img.shields.io/crates/l/mcp-atlassian.svg)](LICENSE)

An ultra-lightweight [MCP](https://modelcontextprotocol.io) server for
**Jira** and **Confluence**, covering both Atlassian Cloud and Server/Data
Center (self-hosted). Written in Rust.

One static binary. Nothing to install alongside it: no runtime, no shell, no
package manager in the image.

70 tools, prompts, resources, and the safety switches an operator actually
wants: read-only mode, dry run, confirmation of destructive calls, an audit
log, and a sandbox for attachments.

## Pick a way to run it

| | Good for | Section |
|---|---|---|
| **Homebrew** | macOS and Linux desktops; upgrades and shell completion come with it | [below](#homebrew) |
| **Docker** | servers, containers, and keeping the host clean | [below](#docker) |
| **Executable** | anything else, Windows included; no package manager needed | [below](#executable) |
| **From source** | a Rust toolchain and a reason to build it yourself | [below](#build-from-source) |

They all take the same configuration, described once in
[Configuration](#configuration).

## Homebrew

### Install

```bash
brew install d3lph1/tap/mcp-atlassian
```

Upgrades arrive with `brew upgrade`. Completion scripts for bash, zsh and fish
are installed along with the binary, so `mcp-atlassian <Tab>` works after your
next shell starts.

### Configure

Point your MCP client at the installed binary. On Apple Silicon that is
`/opt/homebrew/bin/mcp-atlassian`; on Intel macOS and on Linux, run
`which mcp-atlassian` to be sure.

```json
{
  "mcpServers": {
    "atlassian": {
      "command": "/opt/homebrew/bin/mcp-atlassian",
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

## Docker

### Install

```bash
docker pull ghcr.io/d3lph1/mcp-atlassian:latest
```

Tags are `X.Y.Z`, `X.Y` and `latest` for releases, plus `edge` for the tip of
master. The image is `linux/amd64` and `linux/arm64`, built `FROM scratch`
around the static binary: no shell, no package manager, nothing else to patch.

### Configure

The server speaks MCP over stdin and stdout, so `-i` is required and `-t` must
be left out.

```json
{
  "mcpServers": {
    "atlassian": {
      "command": "docker",
      "args": [
        "run", "--rm", "-i",
        "-e", "JIRA_URL", "-e", "JIRA_USERNAME", "-e", "JIRA_API_TOKEN",
        "-e", "CONFLUENCE_URL", "-e", "CONFLUENCE_USERNAME", "-e", "CONFLUENCE_API_TOKEN",
        "ghcr.io/d3lph1/mcp-atlassian:latest"
      ],
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

`-e NAME` without a value passes the variable through from the environment the
client sets, so the token never appears in the argument list.

Two container-specific notes:

- **Secrets.** Mount them and point the `*_FILE` variables at the mount, which
  is what Docker and Kubernetes secrets expect:
  `-v /run/secrets/jira:/secret:ro -e JIRA_API_TOKEN_FILE=/secret`.
- **Attachments.** The image has no filesystem of its own, so attachment tools
  need a volume: `-v "$PWD/attachments:/data" -e ATTACHMENT_DIR=/data`. Without
  `ATTACHMENT_DIR` the server warns at startup.

To build the image yourself, `docker build -t mcp-atlassian .` produces the
same thing from source.

## Executable

### Install

Download the binary for your platform from
[Releases](https://github.com/d3lph1/mcp-atlassian/releases). Put it wherever
you keep such things and make it executable.

### Configure

Give your MCP client the absolute path to the binary:

```json
{
  "mcpServers": {
    "atlassian": {
      "command": "/path/to/mcp-atlassian",
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

Shell completion is not installed for you here, so load it yourself:

```bash
eval "$(mcp-atlassian completions zsh)"   # or bash, fish, elvish, powershell
```

## Build from source

Needs a Rust toolchain, 1.88 or newer.

```bash
cargo install mcp-atlassian --features http
```

The `http` feature adds the streamable HTTP transport; leave it out for a
smaller binary if stdio is all you need.

## Configuration

Either product is optional: tools register only for what is configured.

**Authentication.** Cloud uses an email and an API token
(`JIRA_USERNAME` + `JIRA_API_TOKEN`). Server and Data Center use a personal
access token instead (`JIRA_PERSONAL_TOKEN`), and its presence is what selects
that mode. OAuth 2.0 for Cloud works through
`ATLASSIAN_OAUTH_{CLIENT_ID,CLIENT_SECRET,REFRESH_TOKEN,CLOUD_ID}` and
configures both products at once. Confluence takes the same variables with a
`CONFLUENCE_` prefix.

**Secrets from files.** Any token variable also accepts a `_FILE` spelling
that reads the value from a path: `JIRA_API_TOKEN_FILE=/run/secrets/jira`.
Setting both spellings of the same token is an error rather than a silent
preference.

The switches worth knowing:

| Variable | Effect |
|---|---|
| `READ_ONLY=true` | write tools are not registered at all |
| `DRY_RUN=true` | writes are validated and described, never sent |
| `CONFIRM_DESTRUCTIVE=true` | deletes, transitions and the like ask the user first, through MCP elicitation |
| `ENABLED_TOOLS` / `DISABLED_TOOLS` | wildcards: `jira_*`, `*_get_*`; deny wins |
| `AUDIT_LOG_FILE` | one JSONL line per write: tool, arguments, outcome, what it created |
| `ATTACHMENT_DIR` | the only directory attachment tools may read or write |
| `CACHE_TTL` | seconds to cache projects, fields, boards, spaces; off by default |
| `JIRA_DEPLOYMENT` | `cloud` or `server`, when the auth mode does not settle it |
| `LOG_FILTER` | tracing directives, `info` by default |
| `TRANSPORT=streamable-http` | HTTP instead of stdio; see `HOST`, `PORT`, `MCP_BEARER_TOKEN` |

Every one of them is also a flag on `serve`, and a flag wins over its variable:

```bash
mcp-atlassian serve --jira-url https://your-company.atlassian.net --read-only
```

`mcp-atlassian serve --help` lists all of them, grouped by area, each flag
naming the variable behind it. **Tokens are deliberately not flags**: arguments
are visible to every process through `ps` and are kept in shell history, so a
token comes from the environment or from a file that a `--*-token-file` flag
points at.

The complete list of variables is in [AGENTS.md](AGENTS.md).

## What the model gets

- **Tools.** Jira: search (JQL), issues, transitions, comments, worklog, links
  and epics, fields and their options, agile boards and sprints, watchers,
  attachments. Confluence: search (CQL), pages with section edits and moves,
  comments and inline comments, spaces and labels, attachments, version history
  and diffs, templates, restrictions. Every tool has a typed output schema and
  honest annotations: read-only, destructive, idempotent.
- **Content as Markdown.** Confluence storage format is converted both ways,
  including code blocks, panels, links, images and task lists; raw macros
  written in Markdown pass through.
- **Resources.** `jira://PROJ-123`, `jira://PROJ-123/comments`,
  `confluence://123456`, `confluence://123456/comments`.
- **Prompts.** `/jira_issue PROJ-123`, `/jira_triage PROJ`,
  `/jira_standup <board>`, `/confluence_page 123456`: each fetches its own data
  and ends with a bounded ask.
- **Completion** of issue keys in prompts and resource templates.

## Design

Deployment is inferred from the auth mode (API token means Cloud, PAT means
Server/DC) and can be overridden. Jira uses REST v2 everywhere, so one code
path serves both. Endpoint paths are guarded against interpolated identifiers,
page sizes are capped, retries are bounded and only for what is safe to replay,
and every setting is read and validated in one place.

The reasons behind these and forty other decisions, and what is deliberately
not done, are in [DECISIONS.md](DECISIONS.md).

## Develop

```bash
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release --features http
```

Tests never touch a real Atlassian instance; they run against wiremock and an
in-memory MCP transport.

## License and trademarks

MIT, see [LICENSE](LICENSE).

Jira, Confluence and Atlassian are trademarks of Atlassian Pty Ltd, and all
rights to them belong to Atlassian. This project is an independent,
community-built tool: it is not developed, endorsed or supported by Atlassian.
