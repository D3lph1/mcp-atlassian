# mcp-atlassian (Rust)

Lightweight MCP server for Jira and Confluence (Cloud + Server/Data Center).
Built for a small footprint: target < 30 MB RSS, static musl binary, `FROM
scratch` Docker image — where interpreted implementations of the same surface
typically need an order of magnitude more.

Architecture decisions live in `DECISIONS.md` — read it before changing
anything structural. Key ones: rmcp SDK (D1), Jira API v2 only (D5),
conventional env-var names (D8), curated tool set (D9), deployment detection
by auth mode with an explicit override (D16, D41). Status, what is
deliberately not done, and the backlog are D46 — update it when picking up
or finishing an item.

## Attribution

No AI attribution anywhere in this repository. Nothing that ships, is
committed, or is published may name Claude, Anthropic, Claude Code or any
assistant, in any form:

- commit messages: no `Co-Authored-By:` trailer, no `Generated with
  [Claude Code]` line, no emoji marker;
- commit author and committer: `d3lph1 <d3lph1.contact@gmail.com>`, never an
  assistant identity and never another address;
- PR titles, PR bodies, issue comments, release notes, changelogs;
- source comments, doc comments, `README.md`, `DECISIONS.md`, and every other
  tracked file.

Write plain Conventional Commits describing the change and its reason. The
history was rewritten once to strip this attribution — do not reintroduce it.

## Commands

```bash
cargo build                    # debug build
cargo test                     # unit + wiremock integration tests
cargo clippy -- -D warnings    # lint, warnings are errors
cargo fmt                      # format
cargo run -p mcp-atlassian     # run stdio server (needs env vars, see below)
cargo test -p mcp-atlassian-jira   # test a single crate
mcp-atlassian --list-tools     # the flags: --version, --help, --list-tools
```

CI (`.github/workflows/ci.yml`): fmt + clippy + test, coverage to Coveralls
with an 85% floor, cargo-deny, binaries for Linux musl x86_64/aarch64, macOS
x86_64/aarch64 and Windows x86_64, docker; a `v*` tag publishes a release
(D45). A push to master or a `v*` tag also publishes
`ghcr.io/d3lph1/mcp-atlassian` for amd64 and arm64, assembled from those musl
binaries by `Dockerfile.ci` — no emulated build; the tag is `X.Y.Z`/`X.Y`/
`latest` for a release and `edge`/`sha-<commit>` for master (D47).
`Dockerfile` stays the self-contained local build; keep its runtime stage
identical to the CI one.

All five crates are published on crates.io under the `mcp-atlassian-`
prefix (D15). Each keeps its own `README.md` and `LICENSE` inside its
directory — cargo packages nothing from outside one, so the root copies do
not travel.

Releasing is: bump `version` in the root `Cargo.toml` (all five share it),
commit, tag `vX.Y.Z`, push. CI does the rest — binaries, the GHCR image,
the GitHub release, then `cargo publish --workspace` (D45). A `version` job
fails the tag if the manifest and the tag disagree, so the two never drift.
Publishing needs `CARGO_REGISTRY_TOKEN` in the repository secrets.

The Homebrew formula updates itself too: the `homebrew` job renders
`.github/homebrew/mcp-atlassian.rb` and pushes it to `d3lph1/homebrew-tap`
using `HOMEBREW_TAP_TOKEN`, a PAT scoped to that repository (D45). Change
the formula here, never in the tap — the next release overwrites it.

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
| `JIRA_DEPLOYMENT` / `CONFLUENCE_DEPLOYMENT` | `cloud` or `server`: overrides the auth-mode inference, e.g. Data Center behind Basic auth (D41) |
| `CONFLUENCE_URL` / `CONFLUENCE_USERNAME` / `CONFLUENCE_API_TOKEN` / `CONFLUENCE_PERSONAL_TOKEN` | same scheme |
| `ATLASSIAN_OAUTH_CLIENT_ID` / `_CLIENT_SECRET` / `_REFRESH_TOKEN` / `_CLOUD_ID` | OAuth 2.0 (Cloud only); all four together, takes precedence over `*_URL` and configures both services (D17) |
| `ENABLED_TOOLS` | comma-separated allowlist of tool-name patterns; `*` matches any run of characters anywhere (`jira_*`, `*_get_*`, `*_attachment*`); no `*` = exact name; empty = all (D27) |
| `DISABLED_TOOLS` | same syntax, subtracted from what `ENABLED_TOOLS` allows; deny wins (`ENABLED_TOOLS=jira_*` + `DISABLED_TOOLS=*_delete_*`) (D27) |
| `READ_ONLY` | `true` → only tools annotated `readOnlyHint` are registered; writes are absent from `tools/list` (D22). Named `READ_ONLY`, not `READ_ONLY_MODE` as other servers spell it (D8) |
| `DRY_RUN` | `true` → write tools stay listed but are validated and described instead of performed (D26). For demos and prompt rehearsal; reads still execute for real |
| `CONFIRM_DESTRUCTIVE` | `true` → tools annotated `destructiveHint` ask the user through MCP elicitation before running; clients without elicitation are not blocked (D42) |
| `AUDIT_LOG_FILE` | path to a JSONL audit log; every write call appends one record (D23). Unset = no auditing |
| `ATTACHMENT_DIR` | the only directory attachment tools may read from and write to; unset = any path, with a startup warning (D37) |
| `MAX_ATTACHMENT_BYTES` | cap on one attachment either direction; default 50 MB, `0` = no limit (D37) |
| `REQUEST_TIMEOUT` | seconds per Atlassian request (default 30); downloads and uploads get ten times it (D40) |
| `CACHE_TTL` | seconds to cache reference data (projects, issue types, boards, spaces, fields); unset or `0` = no caching (D25) |
| `NO_BANNER` | `true` → print the structured startup line instead of the banner (D29) |
| `NO_COLOR` | any value → no ANSI colour in the banner (colour is also off when stderr is not a terminal) |
| `RUST_LOG` | `tracing` directives, `info` by default (`debug`, `mcp_atlassian_client=debug,info`); no regex forms |
| `TRANSPORT` | `stdio` (default) or `streamable-http` (needs `--features http`) |
| `HOST` / `PORT` / `ALLOWED_HOSTS` | HTTP transport bind address (127.0.0.1:8000) and extra Host-header allowlist (D18) |
| `MCP_BEARER_TOKEN` (or `_FILE`) | HTTP transport: every `/mcp` request must carry this bearer token; `/healthz` is exempt (D39) |

## Layout (cargo workspace)

```
Cargo.toml                        # workspace root: [workspace.dependencies], [workspace.lints]
crates/
  mcp-atlassian-client/           # shared HTTP: env config, auth, retries, error
                                  #   mapping, opt-in TTL cache (D25),
                                  #   ENABLED_TOOLS wildcards (D27); `mcp`
                                  #   feature adds ListResult / StatusResult
                                  #   used by both products
  mcp-atlassian-storage-markdown/ # storage-XHTML <-> Markdown (htmd / comrak),
                                  #   zero Atlassian deps
  mcp-atlassian-jira/             # everything Jira
    src/lib.rs                    #   REST v2 client + JiraTools (tool state)
    src/models.rs
    src/prompts.rs                #   `/jira_issue`, `/jira_triage`, `/jira_standup` (D30)
    src/resources.rs              #   `jira://PROJ-123`, `jira://PROJ-123/comments`,
                                  #     `issue_key` completion (D24, D44)
    src/tools/                    #   meta, users, search, issues, transitions,
                                  #     comments, links, fields, agile, attachments
  mcp-atlassian-confluence/       # everything Confluence
    src/lib.rs                    #   REST client + ConfluenceTools
    src/models.rs
    src/prompts.rs                #   `/confluence_page 123456` (D30)
    src/resources.rs              #   `confluence://123456`, `confluence://123456/comments` (D24, D44)
    src/tools/                    #   search, pages, comments, spaces, attachments,
                                  #     versions, admin, storage (projections)
  mcp-atlassian/                  # the server; contains no product knowledge
    src/main.rs                   #   entry: config, transport dispatch
    src/http.rs                   #   streamable HTTP: bearer token, /healthz,
                                  #     graceful stop (`http` feature, D39)
    src/server.rs                 #   composition, route filtering, ServerHandler
    src/audit.rs                  #   JSONL audit log of write calls (D23)
    src/banner.rs                 #   startup banner; stderr only, never stdout (D29)
    src/dry_run.rs                #   DRY_RUN: describe writes, do not run them (D26)
    src/confirm.rs                #   CONFIRM_DESTRUCTIVE: ask via elicitation first (D42)
    src/router_ext.rs             #   projects product routers onto the server (D21)
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
- Every tool must carry `title`, `annotations(read_only_hint = ...,
  open_world_hint = false)`; write tools add `destructive_hint` and
  `idempotent_hint` (D42). `tests/every_tool.rs` fails otherwise. "Read-only" means *changes nothing*, not "changes
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
  A link the API returned (attachment `content`, `_links.download`) goes
  through `get_bytes`, which checks origin instead — it legitimately carries
  a query string (D33).
- Values interpolated into JQL/CQL go through `mcp_atlassian_client::query::quote`.
- Every environment variable is read in `Config::read` (`config.rs`), which
  takes an `Env`; nothing else calls `std::env::var`. Tests build a `Config`
  with `..Config::default()`.
- A tool that touches the local filesystem resolves the path through the
  product state's `files()` (`FileAccess`, D37); never `tokio::fs` on a path
  the model supplied.
- `Auth` has a hand-written `Debug`; keep tokens out of any new `Debug`
  (D38).
- List responses say how to page: Confluence `ResultsPage` carries
  `start`/`limit`/`has_more`, Jira `AgilePage`/`SearchPage` carry `start_at`,
  and every tool over them takes the matching offset argument.
- Issue fields not modelled in `IssueFields` land in `extra` only when the
  caller named them in `fields` (D35); `get_issue(key, None)` requests
  `DEFAULT_ISSUE_FIELDS`, never everything.
- Confluence section edits operate on storage XHTML via
  `mcp_atlassian_storage_markdown::replace_section` (D36); never round-trip a whole page
  through Markdown to change part of it.
- `to_mcp_error` picks the JSON-RPC code: what the caller can fix is
  `invalid_params`, the rest `internal_error`, HTTP status in `data`.
- Tests never hit real Atlassian instances; use wiremock + fixtures (D14).
- A rule that must hold for every tool goes in `tests/every_tool.rs` as an
  enumeration over `tools/list`, not as one test per tool (D32). CI fails
  under 80% line coverage.
- Caching is opt-in and for reference data only (D25). A new client method
  that returns issues, page bodies or anything a user edits must not go
  through `cached(...)`; if it takes narrowing arguments, they belong in the
  cache key.

## Route filtering and wrappers

Filtering lives in `AtlassianServer::new` — routes are pruned from the
`ToolRouter` before serving (`#[tool_handler(router = self.tool_router)]`),
so disabled tools never appear in `tools/list`. Three wrappers then compose
over the survivors, in this order: `DRY_RUN` replaces write handlers with a
description of the call (`src/dry_run.rs`, D26), `CONFIRM_DESTRUCTIVE` makes
destructive handlers ask first (`src/confirm.rs`, D42), and `AUDIT_LOG_FILE`
wraps that (`src/audit.rs`, D23) so an intercepted or declined call is still
logged. All of them read the same annotations, so a tool cannot be read-only
for one and a write for another, and an unannotated tool is always treated
as a write.
