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
| `*_FILE` on any token var | read the secret from that path instead of the environment — `JIRA_API_TOKEN_FILE=/run/secrets/jira`; Docker/Kubernetes secrets convention (D28). Setting both spellings is an error |
| `JIRA_PERSONAL_TOKEN` | Server/DC auth (Bearer PAT); presence switches mode |
| `CONFLUENCE_URL` / `CONFLUENCE_USERNAME` / `CONFLUENCE_API_TOKEN` / `CONFLUENCE_PERSONAL_TOKEN` | same scheme |
| `ATLASSIAN_OAUTH_CLIENT_ID` / `_CLIENT_SECRET` / `_REFRESH_TOKEN` / `_CLOUD_ID` | OAuth 2.0 (Cloud only); all four together, takes precedence over `*_URL` and configures both services (D17) |
| `ENABLED_TOOLS` | comma-separated allowlist of tool-name patterns; `*` matches any run of characters anywhere (`jira_*`, `*_get_*`, `*_attachment*`); no `*` = exact name; empty = all (D27) |
| `DISABLED_TOOLS` | same syntax, subtracted from what `ENABLED_TOOLS` allows; deny wins (`ENABLED_TOOLS=jira_*` + `DISABLED_TOOLS=*_delete_*`) (D27) |
| `READ_ONLY` | `true` → only tools annotated `readOnlyHint` are registered; writes are absent from `tools/list` (D22). Named `READ_ONLY`, not `READ_ONLY_MODE` as other servers spell it (D8) |
| `DRY_RUN` | `true` → write tools stay listed but are validated and described instead of performed (D26). For demos and prompt rehearsal; reads still execute for real |
| `AUDIT_LOG_FILE` | path to a JSONL audit log; every write call appends one record (D23). Unset = no auditing |
| `CACHE_TTL` | seconds to cache reference data (projects, issue types, boards, spaces, fields); unset or `0` = no caching (D25) |
| `NO_BANNER` | `true` → print the structured startup line instead of the banner (D29) |
| `NO_COLOR` | any value → no ANSI colour in the banner (colour is also off when stderr is not a terminal) |
| `TRANSPORT` | `stdio` (default) or `streamable-http` (needs `--features http`) |
| `HOST` / `PORT` / `ALLOWED_HOSTS` | HTTP transport bind address (127.0.0.1:8000) and extra Host-header allowlist (D18) |

## Layout (cargo workspace)

```
Cargo.toml                       # workspace root: [workspace.dependencies], [workspace.lints]
crates/
  atlassian-client/              # shared HTTP: env config, auth, retries, error
                                 #   mapping, opt-in TTL cache (D25),
                                 #   ENABLED_TOOLS wildcards (D27); `mcp`
                                 #   feature adds ListResult / StatusResult
                                 #   used by both products
  storage-markdown/              # storage-XHTML <-> Markdown (htmd / comrak),
                                 #   zero Atlassian deps
  atlassian-jira/                # everything Jira
    src/lib.rs                   #   REST v2 client + JiraTools (tool state)
    src/models.rs
    src/prompts.rs               #   `/jira_issue PROJ-123` as an MCP prompt (D30)
    src/resources.rs             #   `jira://PROJ-123` as an MCP resource (D24)
    src/tools/                   #   meta, users, search, issues, transitions,
                                 #     comments, links, fields, agile, attachments
  atlassian-confluence/          # everything Confluence
    src/lib.rs                   #   REST client + ConfluenceTools
    src/models.rs
    src/resources.rs             #   `confluence://123456` as an MCP resource (D24)
    src/tools/                   #   search, pages, comments, spaces, attachments,
                                 #     versions, admin, storage (projections)
  mcp-atlassian/                 # the server; contains no product knowledge
    src/main.rs                  #   entry: config, transport (stdio / http)
    src/server.rs                #   composition, route filtering, ServerHandler
    src/audit.rs                 #   JSONL audit log of write calls (D23)
    src/banner.rs                #   startup banner; stderr only, never stdout (D29)
    src/dry_run.rs               #   DRY_RUN: describe writes, do not run them (D26)
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

Resources work the same way: a product exposes `resources::{URI_PREFIX,
templates}` plus `read_resource(uri)` on its tool state, and `server.rs`
dispatches on the URI prefix (D24). `resources/list` is empty by design —
issues and pages are unbounded; the templates carry the URI shapes.

Prompts too: a product exposes `prompts::router()` over its own state, and
`project_prompt_router` re-targets it (D30). Prompts fetch their own data — a
prompt that only tells the model which tool to call is not worth invoking.
Both surfaces are gated on the product still having tools after filtering, so
neither is a way around `ENABLED_TOOLS`.

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
  `destructive_hint`. "Read-only" means *changes nothing*, not "changes
  nothing in Atlassian" — a tool that writes a local file is a write (D31). That annotation is what `READ_ONLY`, the audit log
  and `DRY_RUN` all key off, and an unannotated tool is treated as a write
  (D22). Tests fail if it is missing.
- No new heavyweight deps without a DECISIONS.md entry; check binary size
  impact (`cargo bloat`) for anything non-trivial.
- List tools take their page size from `mcp::page_size(args.limit, default)`,
  never a bare `unwrap_or` — an uncapped limit floods the context (D31).
  Pagination offsets are not page sizes and are not capped.
- Values interpolated into an endpoint path are checked once, in
  `AtlassianClient::request`; do not re-implement that per call site (D31).
- Tests never hit real Atlassian instances; use wiremock + fixtures (D14).
- Caching is opt-in and for reference data only (D25). A new client method
  that returns issues, page bodies or anything a user edits must not go
  through `cached(...)`; if it takes narrowing arguments, they belong in the
  cache key.

## Roadmap phases

1. ✅ Skeleton: config, auth, shared client, rmcp stdio server.
2. ✅ Jira core tools (D9 list, 13 tools).
3. ✅ Confluence core tools (11 tools) + markdown conversion.
4. ✅ `ENABLED_TOOLS` / `READ_ONLY` (route filtering at startup),
   Dockerfile (musl + scratch), GitHub Actions CI.
5. ✅ Deferred items (D11): streamable HTTP (`http` feature, D18), OAuth 2.0
   refresh-flow (D17), Jira agile API (4 tools), attachments (3 tools).
   Not doing: SSE transport (deprecated in MCP spec), multi-user auth proxy.

Filtering lives in `AtlassianServer::new` — routes are pruned from the
`ToolRouter` before serving (`#[tool_handler(router = self.tool_router)]`),
so disabled tools never appear in `tools/list`. Two wrappers then compose over
the survivors, in this order: `DRY_RUN` replaces write handlers with a
description of the call (`src/dry_run.rs`, D26), and `AUDIT_LOG_FILE` wraps
that (`src/audit.rs`, D23) so an intercepted call is still logged, marked
`dry_run: true`. All three read the same `readOnlyHint` source of truth, so a
tool cannot be read-only for one and a write for another, and an unannotated
tool is always treated as a write.
