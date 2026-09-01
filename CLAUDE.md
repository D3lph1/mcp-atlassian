# mcp-atlassian (Rust)

Lightweight MCP server for Jira and Confluence (Cloud + Server/Data Center).
Built for a small footprint: target < 30 MB RSS, static musl binary, `FROM
scratch` Docker image — where interpreted implementations of the same surface
typically need an order of magnitude more.

Architecture decisions live in `DECISIONS.md` — read it before changing
anything structural. Key ones: rmcp SDK (D1), Jira API v2 only (D5),
conventional env-var names (D8), curated tool set (D9), deployment detection
by auth mode (D16).

Project status and the feature backlog live in `HANDOFF.md` — update it when
finishing a phase or picking up a backlog item.

## Commands

```bash
cargo build                    # debug build
cargo test                     # unit + wiremock integration tests
cargo clippy -- -D warnings    # lint, warnings are errors
cargo fmt                      # format
cargo run -p mcp-atlassian     # run stdio server (needs env vars, see below)
cargo test -p atlassian-jira   # test a single crate
```

Release build (size-optimized, see profile in Cargo.toml):

```bash
cargo build --release
cargo build --release --target x86_64-unknown-linux-musl   # static, for Docker
docker build -t mcp-atlassian .                            # scratch image, ~5 MB
```

## Configuration (env vars)

Variable names follow the conventions established by existing Atlassian MCP
servers, so an existing client config works unchanged. Either service is
optional — tools register only for configured services.

| Var | Purpose |
|---|---|
| `JIRA_URL` | e.g. `https://company.atlassian.net` (Cloud) or self-hosted URL |
| `JIRA_USERNAME` + `JIRA_API_TOKEN` | Cloud auth (Basic) |
| `JIRA_PERSONAL_TOKEN` | Server/DC auth (Bearer PAT); presence switches mode |
| `CONFLUENCE_URL` / `CONFLUENCE_USERNAME` / `CONFLUENCE_API_TOKEN` / `CONFLUENCE_PERSONAL_TOKEN` | same scheme |
| `ATLASSIAN_OAUTH_CLIENT_ID` / `_CLIENT_SECRET` / `_REFRESH_TOKEN` / `_CLOUD_ID` | OAuth 2.0 (Cloud only); all four together, takes precedence over `*_URL` and configures both services (D17) |
| `ENABLED_TOOLS` | comma-separated allowlist of tool names; empty = all |
| `READ_ONLY_MODE` | `true` → only tools annotated `readOnlyHint` are registered; writes are absent from `tools/list` (D22) |
| `AUDIT_LOG_FILE` | path to a JSONL audit log; every write call appends one record (D23). Unset = no auditing |
| `TRANSPORT` | `stdio` (default) or `streamable-http` (needs `--features http`) |
| `HOST` / `PORT` / `ALLOWED_HOSTS` | HTTP transport bind address (127.0.0.1:8000) and extra Host-header allowlist (D18) |

## Layout (cargo workspace)

```
Cargo.toml                       # workspace root: [workspace.dependencies], [workspace.lints]
crates/
  atlassian-client/              # shared HTTP: env config, auth, retries, error
                                 #   mapping; `mcp` feature adds ListResult /
                                 #   StatusResult used by both products
  storage-markdown/              # storage-XHTML <-> Markdown (htmd / comrak),
                                 #   zero Atlassian deps
  atlassian-jira/                # everything Jira
    src/lib.rs                   #   REST v2 client + JiraTools (tool state)
    src/models.rs
    src/tools/                   #   meta, users, search, issues, transitions,
                                 #     comments, links, fields, agile, attachments
  atlassian-confluence/          # everything Confluence
    src/lib.rs                   #   REST client + ConfluenceTools
    src/models.rs
    src/tools/                   #   search, pages, comments, spaces, attachments,
                                 #     versions, admin, storage (projections)
  mcp-atlassian/                 # the server; contains no product knowledge
    src/main.rs                  #   entry: config, transport (stdio / http)
    src/server.rs                #   composition, route filtering, ServerHandler
    src/audit.rs                 #   JSONL audit log of write calls (D23)
    src/router_ext.rs            #   projects product routers onto the server (D21)
```

A product is one crate: its client, models and tools live together, so adding a
tool touches one directory. Tools are inherent methods on the product's own
state type (`JiraTools`/`ConfluenceTools`) — `#[tool_router]` requires that —
and `mcp-atlassian` re-targets those routers onto `AtlassianServer` via
`project_router` (D21).

Tools are grouped by domain, one file per domain, each with its own argument
schemas and a named router. Router names carry a product prefix
(`jira_search_router`, `confluence_search_router`) because they are associated
functions and therefore share one namespace per type.

Adding a tool: put it in the matching domain file of the product crate.
Adding a domain: new file with `#[tool_router(router = <product>_<domain>_router,
vis = "pub(crate)")]` plus one `+=` line in that product's `tools/mod.rs`.
Adding a product: new crate exposing `tools::router()` over its own state, plus
one `project_router(...)` call in `AtlassianServer::new`.

The `mcp` feature is what pulls rmcp into a product crate; without it the crate
is a plain REST library (D15).

## Conventions

- serde response structs: only the fields we use, `#[serde(default)]` on
  optionals, never deserialize the full Atlassian payload (D4).
- Jira: REST API v2 everywhere, plain-text/wiki-markup bodies — no ADF (D5).
- Errors returned to the MCP client must be actionable for an LLM: name the
  entity and the likely fix ("issue PROJ-1 not found", "401: check
  JIRA_API_TOKEN") (D13).
- New tools: add to the list in DECISIONS.md D9 first; keep tool descriptions
  prescriptive about *when* to call the tool.
- Tools return `Json<T>`, never hand-built text — that is what derives the
  `outputSchema` (D20). Lists go through `ListResult<T>`, write operations
  through `StatusResult`; `structuredContent` must be an object, not an array.
- Every tool must carry `annotations(read_only_hint = ...)`; write tools add
  `destructive_hint`. That annotation is what `READ_ONLY_MODE` filters on, and
  an unannotated tool is treated as a write (D22). Tests fail if it is missing.
- No new heavyweight deps without a DECISIONS.md entry; check binary size
  impact (`cargo bloat`) for anything non-trivial.
- Tests never hit real Atlassian instances; use wiremock + fixtures (D14).

## Roadmap phases

1. ✅ Skeleton: config, auth, shared client, rmcp stdio server.
2. ✅ Jira core tools (D9 list, 13 tools).
3. ✅ Confluence core tools (11 tools) + markdown conversion.
4. ✅ `ENABLED_TOOLS` / `READ_ONLY_MODE` (route filtering at startup),
   Dockerfile (musl + scratch), GitHub Actions CI.
5. ✅ Deferred items (D11): streamable HTTP (`http` feature, D18), OAuth 2.0
   refresh-flow (D17), Jira agile API (4 tools), attachments (3 tools).
   Not doing: SSE transport (deprecated in MCP spec), multi-user auth proxy.

Filtering lives in `AtlassianServer::new` — routes are pruned from the
`ToolRouter` before serving (`#[tool_handler(router = self.tool_router)]`),
so disabled tools never appear in `tools/list`. `AUDIT_LOG_FILE` then wraps
the surviving write routes (`src/audit.rs`, D23): same `readOnlyHint` source
of truth, so read tools are never logged and an unannotated one always is.
