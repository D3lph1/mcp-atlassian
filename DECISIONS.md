# Architecture Decisions

Short entries. Format: decision → why.

## D1. MCP SDK: `rmcp` (official Rust SDK)
Official SDK from modelcontextprotocol; supports stdio and streamable HTTP,
with `#[tool]` macros for declaring tools. The alternative (hand-rolled
JSON-RPC) is extra work and risks drifting from the spec.

## D2. Async runtime: tokio, minimal features
`rmcp` requires tokio. Enable only `rt`, `macros`, `io-std`, `net`.
Single-threaded runtime (`current_thread`) for stdio mode — less memory.

## D3. HTTP client: reqwest + rustls
`reqwest` with `default-features = false`, `rustls-tls`, `json`, `gzip`.
No openssl → easier cross-compilation and static linking (musl).

## D4. No Atlassian SDK — a thin REST client
Our own wrapper over the Jira/Confluence REST APIs. serde structs carry only
the fields we use (`#[serde(default)]` plus ignoring the rest). This is the
main source of memory savings against implementations that materialize full
response models.

## D5. Jira API v2 for both Cloud and Server/DC
Cloud supports both v2 and v3. v3 requires ADF (Atlassian Document Format)
for description/comment — a heavyweight JSON document. v2 accepts plain text /
wiki markup and behaves the same on Cloud and Server/DC (v8.14+). One code
path instead of two. Move to ADF/v3 only if v2 is deprecated.

## D6. Auth in v1: API token (Cloud) + PAT (Server/DC)
- Cloud: Basic auth (email + API token).
- Server/DC: Bearer PAT.
Detected automatically: `*_PERSONAL_TOKEN` set → PAT, otherwise API token.
OAuth 2.0 and multi-user were deferred (see D11) — they pull in a browser
flow, token refresh and secret storage.

## D7. v1 transport: stdio
The primary scenario is a local launch from Claude Desktop / Claude Code /
Cursor. Streamable HTTP sits behind a cargo feature so it does not bloat the
binary for people who never use it.

## D8. Conventional environment variable names
`JIRA_URL`, `JIRA_USERNAME`, `JIRA_API_TOKEN`, `JIRA_PERSONAL_TOKEN`,
`CONFLUENCE_URL`, `CONFLUENCE_USERNAME`, `CONFLUENCE_API_TOKEN`,
`CONFLUENCE_PERSONAL_TOKEN`, `ENABLED_TOOLS`, `READ_ONLY`.
These names are what existing Atlassian MCP servers use, so switching to this
server is a change of the launch command, not of the client configuration.

One deliberate divergence: the switch other servers spell `READ_ONLY_MODE` is
`READ_ONLY` here, so the three behaviour switches read as a set — `READ_ONLY`,
`DRY_RUN` (D26), `CACHE_TTL` (D25) — instead of one of them carrying a `_MODE`
suffix the others do not. No alias: the rename landed before the first release,
so there is no configuration in the world to keep working. Anyone porting a
config from another server edits one variable name.

## D9. Full tool coverage (70), organized by domain
The set started deliberately small and grew to cover the surface users
actually expect from an Atlassian MCP server. Grouping keeps it navigable —
one file per domain (D19), so the count does not translate into sprawl:

Jira (40):
- meta: myself, search_users, get_projects, get_issue_types
- users: get_user_profile, search_assignable_users, assign_issue,
  get_watchers, add_watcher, remove_watcher
- search: search (JQL), get_project_issues
- issues: get/create/update/delete, batch_create_issues, get_changelog
- transitions: get_transitions, transition_issue
- comments: add/get/edit comment, add_worklog, get_worklog
- links: get_link_types, create_issue_link, remove_issue_link,
  create_remote_link, link_to_epic
- fields: search_fields, get_field_options
- agile: get_boards, get_board_issues, get_sprints, get_sprint_issues,
  move_to_sprint
- attachments: get, download, upload

Confluence (30):
- search: search (CQL), search_users
- pages: get_page, get_page_children, get_space_page_tree, create, update,
  update_page_section, move_page, delete
- comments: add, get, reply, get_inline, add_inline
- spaces: get_spaces, get_labels, add_label
- attachments: get, download, upload, delete
- versions: get_page_history, get_page_version, get_page_diff
- admin: list_templates, get_template, create_page_from_template,
  get/set_page_restrictions

Two guidelines survive the growth. First, a tool must earn its place by
unlocking a scenario (`jira_search_fields` resolves the custom-field ids that
updates need; `jira_search_assignable_users` prevents proposing an assignee
Jira would reject). Second, tools return data, not files: attachments are
written to disk and reported by path rather than inlined as base64, which
would flood the context window.

## D10. Content: Markdown in and out
- Confluence storage format (XHTML) → Markdown on read, via `htmd`
  (lightweight, no headless browser).
- Markdown → storage on write, via `comrak` (CommonMark → HTML).
- Version comparison → unified diff, via `similar` (Myers with heuristics).
An LLM client wants Markdown; raw storage XHTML burns tokens.

`diff_pages` started as a hand-rolled LCS to avoid a dependency. That was a
mistake worth recording: the LCS table costs O(n·m) memory, measured at
**249 MB and 231 ms** on a 5000-line page — against a server whose whole idle
footprint is ~2 MB — and it emitted the entire document as context rather than
just the changed regions (53,903 characters for a one-line edit). `similar`
does the same job at **2.6 MB, 1.7 ms and 133 characters**, and emits proper
`@@` hunks. It pulls in no transitive dependencies with
`default-features = false, features = ["text"]`.

The lesson generalizes: "avoid a dependency" is not a reason to reimplement a
non-trivial algorithm badly. Measure the naive version before keeping it.

## D11. Deferred → delivered in phase 5 (except SSE)
- ✅ OAuth 2.0 — refresh flow without a browser (see D17)
- ✅ Streamable HTTP — behind the `http` cargo feature (see D18)
- ✅ Jira Agile API — boards, sprints, sprint issues, move-to-sprint
- ✅ Attachments — list / download / upload (multipart)
- ❌ SSE transport — not happening: deprecated in the MCP spec
- ❌ Multi-user auth proxy — out of scope (single-tenant server)

## D12. Deployment: one static binary + a scratch image
Targets `x86_64-unknown-linux-musl` / `aarch64-unknown-linux-musl`.
Docker: `FROM scratch` plus the binary; TLS roots are compiled in via
rustls/webpki-roots, so no CA bundle is needed. Release profile: `lto = true`,
`codegen-units = 1`, `strip = true`, `panic = "abort"`, `opt-level = "z"`.

`lto = true` is *fat* LTO, and the choice is worth a number. Measured on
aarch64-apple-darwin, full rebuild after `cargo clean --release`:

| `lto` | binary | vs fat | build |
|---|---|---|---|
| `true` (fat) | 3 965 952 | — | 61 s |
| `"thin"` | 5 280 128 | +33.1% | 44 s |
| `false` | 5 461 344 | +37.7% | 37 s |

The usual advice — thin buys most of the win for a fraction of the time —
does not hold here: thin is only 3.3% smaller than no LTO at all, and the
whole 1.31 MB comes from thin → fat. The shape of the project explains it.
Five crates layered thinly over each other (tool → product client → shared
HTTP client) produce call chains that collapse only under cross-crate
inlining, which is exactly what thin does not do, and `opt-level = "z"`
suppresses inlining on its own — fat LTO restores the part of it that deletes
dead code.

So 24 seconds of build time buys 1.5 MB in a `FROM scratch` image whose whole
point is being an order of magnitude smaller than the interpreted
alternatives. Numbers are for this host, not the musl target the image uses;
the ratio should hold, the absolutes will not.

The other knobs trade diagnostics for size, and that is a trade rather than a
free win: `strip = true` costs symbol names in a backtrace, `panic = "abort"`
costs unwinding and `catch_unwind`. If a rare production panic ever needs
diagnosing, `strip = false` buys the names back for about 1 MB.

The released binaries differ across architectures by more than the profile
explains, and the reason is worth writing down once so the next person does
not go looking for a regression. On 0.1.0, `x86_64-unknown-linux-musl` is
6.41 MB against 4.83 MB for `aarch64-unknown-linux-musl`. Section sizes put
581 KB of the 1.6 MB into `.rela.dyn` and the rest, about 1030 KB, into
`.text`.

The relocations are a linking difference, not a code difference: the x86_64
musl target sets `static-position-independent-executables`, so its binary is
a static PIE (`ET_DYN`, ~24 800 relocations), while the aarch64 musl target
does not and links a plain `ET_EXEC` with none. That is upstream Rust target
configuration, not something this repository chose. It also means the aarch64
binary runs without ASLR — acceptable for a process a desktop client spawns
over stdio, and fixable with `-C link-arg=-static-pie` at the cost of those
same 581 KB, if the cross linker accepts it.

The `.text` difference is the architecture plus the cost of position
independence. AArch64 spends exactly four bytes per instruction; x86-64
averages more and needs more instructions in places, and its functions are
aligned to 16 bytes rather than four. This project has an unusual number of
small functions — 70 tools, each wrapped three times and boxed once per
projected router (D21) — so alignment padding accumulates. Neither number
indicates that LTO failed; without it the figures would be megabytes apart.

## D13. Errors: anyhow internally, typed mapping outward
HTTP 401/403/404/429 map to MCP errors with actionable text ("check
JIRA_API_TOKEN", "issue PROJ-1 not found"). 429 respects `Retry-After` and is
retried once. The LLM must be able to tell from the message what to fix.

## D14. Tests: wiremock
Unit and integration tests against `wiremock` (a mocked Atlassian API), with
fixtures taken from real Cloud/DC responses. No live instances in CI.

## D15. Structure: a crate per product, not a crate per layer
Each Atlassian product is one crate that owns everything about it — REST
client, models and MCP tools:

- `crates/mcp-atlassian-client` — shared HTTP: auth (token/PAT/OAuth), retries,
  error mapping, env configuration, plus the shared MCP result types behind an
  `mcp` feature. Depends on reqwest, serde.
- `crates/mcp-atlassian-jira` — the Jira client, its models and (behind `mcp`) its
  tools.
- `crates/mcp-atlassian-confluence` — the same for Confluence.
- `crates/mcp-atlassian-storage-markdown` — storage XHTML ↔ Markdown (htmd/comrak). No
  Atlassian dependencies at all.
- `crates/mcp-atlassian` (bin) — composes the product routers into one server:
  configuration, route filtering (`ENABLED_TOOLS`, `READ_ONLY`),
  transports.

The earlier layout put clients in product crates but tools in the server crate
(`mcp-atlassian/src/tools/jira/`). That split every product across two crates
at different levels of the tree: adding one Jira tool meant editing two
directories. Now a product is one directory, and the server crate contains no
product knowledge at all.

The `mcp` feature keeps the clients usable as plain REST libraries — without it
they do not pull in rmcp (verified: `cargo tree -p mcp-atlassian-jira` shows no
rmcp until `--features mcp`).

Every crate carries the `mcp-atlassian-` prefix, and that is a statement about
support rather than about tidiness. crates.io has one flat namespace, a name is
taken for good, and publishing is not optional here: a crate with a path
dependency on an unpublished one is rejected, so shipping the server means
shipping all five. `atlassian-jira` is exactly the name someone writing a real
Jira library for Rust would want. Taking it for what is, honestly, the
internals of one MCP server would close that door and quietly promise an API
this repository does not intend to keep stable — these crates version as one
line, `version.workspace = true`, and move whenever the server needs them to.
The prefix says whose internals they are. The property above stays true; it is
just no longer advertised as an offer. Directory names match the crate names,
so `cargo test -p <name>` and `ls crates/` agree.

Upside beyond tidiness: parallel compilation, tests and wiremock fixtures next
to their client. Dependency versions come from `[workspace.dependencies]`,
lints from `[workspace.lints]`.

## D16. Deployment detection by auth mode; search diverges Cloud/DC
Cloud removed `/rest/api/2/search` (2025) in favor of the token-paginated
`/rest/api/2/search/jql` (`nextPageToken`, no `total`). Server/DC still serves
the offset-paginated original (`startAt`/`total`).

Deployment is inferred from the auth mode: Basic (email + API token) or OAuth
→ Cloud, Bearer PAT → Server/DC. The same signal decides the user-reference
shape — Cloud `{"accountId": ...}` vs DC `{"name": ...}` (assignee and
friends) — the routing of `get_projects` (`/project/search` vs `/project`),
and the user-search parameter (`query` vs `username`).
Limitation: DC with Basic auth is unsupported (PAT exists since Jira 8.14).

## D17. OAuth 2.0: refresh flow without a browser
We do not implement a full 3LO wizard (browser, callback server). The user
obtains a refresh token once, out of band (scope `offline_access`), and sets
`ATLASSIAN_OAUTH_{CLIENT_ID,CLIENT_SECRET,REFRESH_TOKEN,CLOUD_ID}`; the server
refreshes the access token itself and honors refresh-token rotation, keeping
the new one in memory. OAuth configures both services through
`api.atlassian.com/ex/{jira,confluence}/{cloud_id}` and takes precedence over
`JIRA_URL`/`CONFLUENCE_URL`. One `OAuthSession` (Arc) is shared by both
clients, so they draw from the same token cache.
Limitation: after a process restart the refresh token from the environment is
used again; if Atlassian rotated it during the previous session, a fresh
authorization is required (keep the current token in the environment).

## D18. Streamable HTTP behind the `http` cargo feature
`TRANSPORT=streamable-http` (plus `HOST`, `PORT`, `ALLOWED_HOSTS`) serves the
`/mcp` endpoint through rmcp's StreamableHttpService + axum. The default
binary stays stdio-only (D7): the extra ~0.3 MB is paid only by those who
build with `--features http`. rmcp's DNS-rebinding protection is active —
non-loopback hosts must be listed in `ALLOWED_HOSTS`. There is no auth at the
HTTP layer; this is a single-tenant server, so put it behind a reverse proxy
for external access.

## D19. Tools: a file per domain, a router per file
`#[tool_router]` operates on an impl block, which makes "all tools in one
file" the path of least resistance — and a 900-line `server.rs` unreadable.
We split by domain instead: `tools/{jira,confluence}/<domain>.rs`, each with
its own argument schemas and a named router; the product's `mod.rs` merges
them with `+=`, and `AtlassianServer::new` merges the two product routers.

Why per domain and not per tool: the average tool is ~25 lines, so a separate
file would add ~10 lines of header (30% boilerplate) and 32 registration
points; related tools (`get_transitions` → `transition_issue`, `create` →
`update` → `delete`) are read together, and splitting them across files breaks
that.

Gotcha: routers are generated as associated functions on `AtlassianServer`,
so their names are global — `search_router` in jira and confluence would
collide. Hence the mandatory product prefix: `jira_search_router`,
`confluence_search_router`.

## D20. Structured output: `Json<T>` on every tool
MCP `2025-06-18` added `outputSchema` on a tool and `structuredContent` on a
result. Tools return `Json<T>` (rmcp's wrapper) instead of hand-built text, so
the schema is derived from `T` by the `#[tool]` macro and the typed value is
placed in `structuredContent`. Clients therefore know the shape of an answer
before calling, and can consume it without parsing prose.

Three consequences shaped the code:

- **Client models derive `JsonSchema`.** `schemars` is a plain dependency of
  `mcp-atlassian-jira`/`mcp-atlassian-confluence`, not a feature. It is not an MCP
  dependency (D15 still holds — the clients stay protocol-free), and it is
  already in the tree via rmcp.
- **Lists are wrapped.** `structuredContent` is a JSON *object* per spec, so
  collection results go through `ListResult<T>` (`{items, count}`) rather than
  a bare array, and write operations through `StatusResult` (`{ok, message}`).
- **Ad-hoc `serde_json::json!` projections became types.** Confluence page
  views, the space tree and version diffs are now `PageView` / `PageNode` /
  `PageDiff`, since an untyped `Value` cannot produce a schema.

Backwards compatibility is free: rmcp's `CallToolResult::structured` fills the
text `content` block with the same JSON, so clients that predate structured
output keep working.

## D21. Projecting tool routers across crates
`#[tool_router]` generates *inherent* methods, and Rust only allows those in
the crate that declares the type. Taken literally that forces every tool into
whichever crate declares the server — which is exactly the split D15 removes.

So each product crate declares its own small state type (`JiraTools`,
`ConfluenceTools`, each holding just its client) and builds a router over it.
`mcp-atlassian` re-targets those routes onto its own `AtlassianServer` with
`router_ext::project_router`, which wraps each route's handler and maps
`&AtlassianServer -> &JiraTools` on the way in.

The adapter uses only rmcp's public API (`ToolRoute::new_dyn`,
`ToolCallContext::new`, the `CallToolRequestParams` builders) and carries
arguments, MRTR input responses and request state across unchanged, so nothing
is lost in translation. The alternative — one giant crate, or tools split from
their clients — trades a ~50-line adapter for a structural problem in every
future product.

## D22. `READ_ONLY` is driven by tool annotations
`READ_ONLY=true` registers only tools whose MCP annotation says
`readOnlyHint: true`. Everything else never reaches `tools/list`, so a client
cannot call it — a blocked write is "tool not found", not a runtime refusal.

The check used to consult a `WRITE_TOOLS` list of names in `server.rs`. That
list sat far from the tools themselves, and a new write tool that nobody added
to it would have been silently callable in read-only mode. The annotation now
carries that fact next to the tool, and the default is fail-safe: a tool with
no `readOnlyHint` counts as a write.

The annotations pay for themselves twice — `destructiveHint` also tells
clients which operations (deletes, restriction changes, status transitions)
deserve a confirmation prompt, independent of this mode.

Guarding the guard: tests assert that every tool declares a hint, and
cross-check each annotation against what the tool's name implies, so a
mislabeled tool fails the suite rather than quietly widening read-only mode.

The variable was named `READ_ONLY_MODE` until it acquired siblings; see D8 for
why it diverges from the name other servers use.

## D23. Audit log: JSONL, driven by the same annotations as read-only mode
`AUDIT_LOG_FILE=/path/audit.jsonl` appends one JSON object per **write** call:

```json
{"ts":"2026-09-01T20:11:02.481Z","tool":"jira_delete_issue","args":{"issue_key":"PROJ-1"},"outcome":"ok","duration_ms":214,"destructive":true}
```

What counts as a write is `readOnlyHint`, the annotation `READ_ONLY`
already filters on (D22) — one source of truth, and the same fail-safe
default: a tool that forgets the annotation is audited rather than silently
skipped. `destructive` mirrors `destructiveHint` and is emitted only when
true, so the deletes and status changes are greppable without a JSON parser.
Reads are not logged: they are the bulk of the traffic and change nothing.

Three choices worth recording:

- **Wrapping routes, not the handler.** The audit wrapper re-targets each
  write route through `ToolRoute::new_dyn`, the same mechanism `router_ext`
  uses (D21), and runs after route filtering — so a tool removed by
  `ENABLED_TOOLS` or `READ_ONLY` never produces a record, because it
  cannot be called. The alternative, logging inside `ServerHandler::call_tool`,
  would need its own copy of the write/read decision.
- **Arguments are logged verbatim, results are not.** The arguments are what
  an operator needs to reconstruct or undo an action; a full response would
  multiply the log size with data already in Jira. Only the outcome (`ok` /
  `error` plus the message) and the duration are kept. Note that the arguments
  contain whatever text the client sent — comment bodies, page content — so
  the log deserves the same file permissions as a backup.
- **A failed append does not fail the call.** It is reported at ERROR level
  and dropped. Killing writes because a disk filled is worse than a visible
  gap in the log for a single-tenant server; a deployment that needs the
  stricter guarantee should ship the file off-host.

The record is written before the response is returned, so a client that saw a
result knows the record exists. Writes are one `write_all` per line into a
file opened `O_APPEND`, serialized by a mutex — concurrent calls cannot
interleave halves of a line, and nothing blocks on the log except the call
being logged.

The one new dependency, `chrono` (timestamps), was already in the tree via
rmcp with the same feature set (`default-features = false, features =
["now"]`), so it costs no extra crate: the release binary grew from 3.703 MB
to 3.719 MB.

Not covered: there is no user identity in the record, because the server
authenticates as exactly one Atlassian account (D6/D17) — the account is a
property of the process, not of the call.

## D24. Resources: two URI templates, no enumeration
`jira://PROJ-123` and `confluence://123456` are readable as MCP resources, so a
client can attach an issue or a page to the context without a tool call.

- Jira returns **JSON** — an issue is a record, and the field list is pinned to
  exactly what `IssueFields` models (D4), not "all fields", which would drag in
  every custom field the instance has.
- Confluence returns **Markdown** with the title as an H1 — the body is
  Markdown everywhere else in this server (D10), and a resource carries only
  contents, so the title has nowhere else to go.

`resources/list` is **empty on purpose**. The resources here are every issue
and every page; enumerating them is unbounded and would turn a listing into a
paginated crawl of the instance. The templates in `resources/templates/list`
carry the URI shapes, and discovery stays with the search tools, which is what
their descriptions already tell the model to use first.

Two smaller decisions worth keeping:

- **No `Url::parse`.** It treats the part before the first slash as a host and
  lowercases it, so `jira://PROJ-123` would arrive as `proj-123` — a key Jira
  does not have. The URIs are parsed by prefix instead, and anything beyond a
  bare key or id (`jira://PROJ-1/comments`, a query string) is rejected with
  the expected shape in the message (D13).
- **Resources follow the tool surface.** A product serves resources only when
  its service is configured *and* at least one of its tools survived filtering.
  Otherwise `ENABLED_TOOLS=confluence_search` would still leave `jira://` as a
  way to read Jira — an allowlist that only narrows half the surface is worse
  than none. `READ_ONLY` needs no such rule: resources are reads.

Both products keep their resource code next to their tools
(`src/resources.rs`), and the server crate only dispatches on the scheme — the
same split as D15/D21.

## D25. TTL cache: opt-in, reference data only
`CACHE_TTL=300` (seconds) caches the answers that describe the *instance*
rather than the work in it: `get_myself`, projects, issue types, boards, issue
link types, Jira field definitions, Confluence spaces. Unset, empty or `0`
means no caching — and that is the default.

Off by default because a cache changes what a read returns. A project created
out of band, a board added this morning, a custom field just introduced — each
stays invisible for up to one TTL. That is a fine trade when an agent resolves
the same field ids twenty times in a session, and a bad one when someone is
watching the tool output to confirm a change they just made. The operator
picks; the server does not decide for them.

Never cached, whatever the TTL: issues, JQL/CQL searches, comments, worklogs,
sprints, page bodies, attachments, versions, restrictions. Those are the things
people edit, and stale answers there are indistinguishable from bugs.

Mechanics:

- One `TtlCache` per client (`Mutex<HashMap<String, Arc<dyn Any + Send +
  Sync>>>`), values downcast on read. A key that holds another type is a miss
  rather than a panic, and an entry past its deadline is a miss too. Expired
  entries are dropped on the next insert, so the map cannot grow unbounded.
- Keys carry every argument that narrows the answer — `boards:PROJ:50` is not
  `boards:OPS:50`, `spaces:25` is not `spaces:50`. This is the failure the
  first attempt at this feature was reverted for.
- `search_fields` caches the *unfiltered* field list and filters client-side,
  so one entry serves every query.
- Failures are never cached: `get_or_fetch` stores only on `Ok`.
- Two concurrent misses on one key both fetch. Single-flight machinery would
  cost more than the extra request saves on a single-tenant server; the second
  write simply wins.

The cache is plain `std` — no `moka`, no `dashmap`, no new dependency. The
release binary grew by ~16 KB, from 3.734 MB to 3.750 MB.

## D26. `DRY_RUN`: writes are described, not performed
`DRY_RUN=true` keeps every write tool in `tools/list` but replaces its
handler: the call is validated against the tool's own input schema and
reported back, and nothing is sent to Atlassian. Reads still execute for
real — a rehearsal against an empty instance answers no useful question.

This is not a weaker `READ_ONLY` (D22); the two answer different
questions. Read-only removes the write tools, which is right for an untrusted
client and useless for rehearsing a prompt: a tool the model cannot see is a
tool it cannot be observed choosing. Dry run leaves the surface intact so the
model picks the tool, fills the arguments, and the operator sees what it
*would* have done. Setting both is not an error — read-only wins, because the
routes are already gone by the time the interception runs, and the server logs
a warning saying so.

The mode is disclosed, not hidden: the notice is appended to the description of
every intercepted tool. A model that believes its writes landed will report
them as done to the user, which is precisely the confusion the mode exists to
prevent.

Mechanics:

- One wrapper over the `ToolRouter` (`src/dry_run.rs`), same shape as the
  audit wrapper (D23) and driven by the same `readOnlyHint` source of truth
  (D22) — so a tool cannot be intercepted by one and executed by the other,
  and an unannotated tool is intercepted rather than silently performed. No
  product crate and no tool implementation changes.
- Order in `AtlassianServer::new`: prune → intercept → audit. Auditing sits
  outside, so an intercepted call is still recorded, with `dry_run: true` on
  the record. A log claiming writes that never left the process would be worse
  than no log.
- The result is a `DryRunReport` (`dry_run`, `tool`, `destructive`,
  `arguments`, `warnings`), and the route's `outputSchema` is swapped for that
  report's. Every tool still advertises one (D20), and it still describes what
  the client actually receives.
- Validation is deliberately shallow — required arguments present, and a
  warning for arguments the schema does not declare (serde would have dropped
  them silently, which is the mistake worth catching). A missing required
  argument is an error, because the real call would have failed to deserialize
  its parameters too. Types and values are the tool's business; re-checking
  them here would mean a second copy of every schema, free to drift.

No new dependency; the release binary grew by 112 bytes, from 3 932 512 to
3 932 624.

## D27. Tool selection: `ENABLED_TOOLS` / `DISABLED_TOOLS`, matched by wildcard
Entries of either list may contain `*`, standing for any run of characters
including none, anywhere in the pattern: `jira_*` is a whole product,
`*_get_*` a verb across both, `*_attachment*` one noun, `*` everything. A
pattern without `*` is still an exact name, so a list written before this
change means what it always did.

`DISABLED_TOOLS` is subtracted from whatever `ENABLED_TOOLS` let through, and
the subtraction wins: a tool named by both is removed. The alternative —
letting a more specific allow beat a broader deny — would make the pair depend
on which variable the reader looked at first. Deny-wins is the rule every
firewall, `.gitignore` and IAM policy already trained people on.

The two compose into the shape most operators actually want, which neither
expresses alone: `ENABLED_TOOLS=jira_*` plus `DISABLED_TOOLS=*_delete_*` is
"all of Jira except the deletes". Without a denylist that is 38 names pasted
by hand. `DISABLED_TOOLS` alone (no allowlist) is the other common shape —
everything except one product, or except one risky verb.

`READ_ONLY` is orthogonal and applies on top: it removes writes whatever the
two lists say. If the three between them leave nothing registered, the server
logs a warning — an empty `tools/list` is otherwise indistinguishable from a
broken build.

With 70 tools, the exact-names-only form was the one place the environment
genuinely hurt: narrowing the server to Jira reads meant pasting 20 names into
a JSON string in the client's config, and re-pasting them whenever the tool set
moved. Every such list was really a pattern written out longhand.

Not a glob library, and not `glob`/`globset` as a dependency: no `?`, no
`[a-z]`, no `{a,b}`, no escaping. Tool names are a flat set of lowercase
`product_verb_noun` identifiers; `*` alone covers every slice anyone has
wanted, and the matcher is 15 lines (`mcp-atlassian-client/src/tool_filter.rs`).
Segments between wildcards are matched greedily left to right, which is exact
for this grammar — with no backtracking constructs, the leftmost occurrence of
a segment never rules out a match a later one would have allowed. The suffix
is checked against what is left after the middles, so `jira*jira` does not
match `jira`.

A pattern that matches no registered tool is logged at WARN on startup. That
warning matters more with wildcards than it did with names: a typo in a
pattern enables nothing and looks exactly like a deliberately narrow filter.

## D28. Secrets may be read from files (`*_FILE`)
Every credential variable has a companion: `JIRA_API_TOKEN_FILE`,
`JIRA_PERSONAL_TOKEN_FILE`, `CONFLUENCE_API_TOKEN_FILE`,
`CONFLUENCE_PERSONAL_TOKEN_FILE`, `ATLASSIAN_OAUTH_CLIENT_SECRET_FILE`,
`ATLASSIAN_OAUTH_REFRESH_TOKEN_FILE`. Set one and the token is read from that
path instead of from the environment. The client id and cloud id have no
`_FILE` form — they are identifiers, not secrets.

This is the convention Docker and Kubernetes secrets already expect, so it
composes with `docker run --secret` and a mounted `Secret` without a wrapper
script. It is also the only way to keep a token out of the MCP client's config
JSON, which is a plaintext file in the user's home directory, and out of
`docker inspect` and `/proc/<pid>/environ`.

Rules, all of them because credentials are the wrong place to be clever:

- Both spellings set at once is an error naming both, not a precedence rule.
- The file's contents are trimmed. `echo secret > file` leaves a newline, and
  a token carrying one fails authentication with nothing in the message to
  suggest why.
- An empty or unreadable file is an error that names the variable and the
  path (D13), not a silently absent credential.

This closes the strongest argument for a TOML config file, which was
considered and rejected here. An MCP stdio server is launched by its client
from a config file that already exists and is not ours; a second one would
split the settings across two places, add a precedence matrix, force every
error message to know which source a value came from, and give up the D8
promise that switching servers is a change of the launch command. The one
thing the environment genuinely cannot express is several instances of the
same product (two Jiras), because the variable names are a flat namespace —
if that lands, the file arrives with it, as part of that feature rather than
as a second spelling of this one.

## D29. Startup banner: stderr only, colour only for a terminal
A framed banner with the version, transport, configured services, tool count
and active modes is printed at startup, followed by the usual logs.

**It goes to stderr, and nothing may ever move it to stdout.** In stdio mode
stdout carries the MCP protocol; one box-drawing character on it desynchronizes
the client's JSON reader. This is the same constraint that already sends the
tracing subscriber to stderr, and it is the one thing about this file worth
remembering. A test asserts stdout stays pure JSON-RPC.

Printed unconditionally rather than only on a TTY. The common deployment is a
container, where stderr is a pipe and `docker logs` is the only place anyone
looks — gating on a terminal would hide the banner exactly where it is most
useful. What does depend on the terminal is colour: ANSI escapes in a captured
log are noise, so they are emitted only when stderr is a TTY, and suppressed by
`NO_COLOR` (the no-color.org convention).

Under the title sit the tagline and the repository URL, the latter read from
`CARGO_PKG_REPOSITORY` rather than written out: a banner is where someone
looks when they want to know what this process is and where to report it, and
taking it from the manifest means a move of the repository cannot leave a
stale address on the screen. A test asserts the two agree.

`NO_BANNER=true` swaps it for the structured `tracing::info!` startup line. The
two are alternatives, not both: they carry the same facts, and printing them
together on every start is noise. The operator picks the format — a box for a
human, key-value fields for a log collector.

Layout is measured in characters, not bytes, and every row is padded from the
visible width, so colour codes cannot shift the frame; tests assert every line
of the box is exactly 80 columns with colour on and off, including an
over-long audit path (elided from the left, keeping the file name).

After the summary — in either form — the registered tools are logged, one
record per product: `jira tools registered count=17 tools=jira_get_issue, ...`.
This is log content rather than presentation, so it is emitted the same way
whether the banner or the structured line was chosen.

Per product, not per tool: 70 records would bury the rest of the startup
output, and the question being asked is almost always "is this one there",
which `grep` answers either way. A name whose prefix is not a known product is
grouped under `other` rather than dropped, so a future crate cannot vanish from
the log because this function has not heard of it. The point is that a narrowed
`ENABLED_TOOLS` / `DISABLED_TOOLS` / `READ_ONLY` can be verified against the
log instead of by calling `tools/list` through a client.

Nothing about the configuration is logged before the summary, and that costs
a small piece of structure. `AtlassianServer::new` is where the unset
`ATTACHMENT_DIR` (D37) and the opened audit log (D23) become known, but it
runs before the banner can be printed — the banner needs the tool count, which
only exists once filtering is done. Logging from the constructor put a `WARN`
above the frame, where it reads as a failure to start rather than as a note
about a running server. So the constructor collects its warnings into
`startup_warnings` and `main` emits them after the summary and the tool list,
with the audit line alongside them. Warnings last, deliberately: the final
line of a long startup is the one an operator actually reads.

No dependency. `std::io::IsTerminal` covers TTY detection, and the escape codes
are six string constants — `owo-colors`, `colored` and the rest would pull a
crate in for `format!`.

## D30. Prompts: they fetch their data
`prompts/list` and `prompts/get` are served, starting with `jira_issue`, which
takes an issue key and returns a briefing: the issue's fields, its newest
comments, then the ask ("state what is being asked, name what is blocking,
propose the next step"). In most clients a prompt is a slash command, so this
is typed `/jira_issue PROJ-123`.

The prompt performs the reads itself rather than telling the model to. A
prompt whose body is "call `jira_get_issue` on PROJ-123" saves the user
nothing they could not have typed, and costs a round trip before the model has
seen a single fact. Fetching makes the prompt worth invoking — which is the
whole difference between a template and a feature.

Two consequences, both handled:

- **Budgets.** A description or a comment thread can be arbitrarily long, so
  the briefing takes the 5 newest comments and truncates the description at
  4000 characters and each comment at 800, *saying* that it did. A silently
  clipped description reads as a complete one, and the model would plan
  against half a ticket. `jira_get_issue` remains available for the full text.
- **Failure.** A missing issue is an error naming the key (D13). A missing
  *comment* endpoint is not: the briefing that already succeeded is worth more
  than the comments it lacks, so that failure is logged at DEBUG and the
  briefing says "No comments."

The briefing ends with "do not create, update or transition anything unless
asked to". A prompt that pulls a ticket into context should not read as
licence to change it.

Prompts live in the product crate next to the client they call, and are
projected onto the server by `project_prompt_router` — `#[prompt_router]`
generates inherent methods for the same reason `#[tool_router]` does (D21), so
the same adapter shape applies.

They follow the tool surface: a product qualifies for prompts *and* resources
when it is configured and at least one of its tools survived filtering. So
`/jira_issue` cannot be a way around an `ENABLED_TOOLS` that removed Jira —
the same rule D24 already applies to `jira://`, which is why the flag is named
for the product rather than for either surface.

## D31. Guards belong at the boundary, not at the call site
Three defects found in one review shared a shape: a rule the project had
already decided, applied at some call sites and forgotten at others. The fixes
moved each rule to the one place it cannot be forgotten.

**A local write is a write.** `jira_download_attachment` and its Confluence
twin were annotated `readOnlyHint: true` — true of Atlassian, false of the
machine. The annotation is the single source of truth for three mechanisms
(D22, D23, D26), so all three were wrong at once: `READ_ONLY=true` kept the
tools registered, the audit log never recorded them, and `DRY_RUN` let them
run. A "read-only" server could overwrite `~/.ssh/authorized_keys` at a path
the model chose, silently. They are now `readOnlyHint: false` +
`destructiveHint: true`, and `_download_` joined the verb list the annotation
test cross-checks against.

**A path segment is data.** Endpoint paths are built with `format!`,
interpolating an issue key or page id that reaches us from the model. `Url::join`
normalizes `..` and honours `?`, so `PROJ-1/../../../myself` redirected the
request to another endpoint with the user's credentials and the original
method — for `DELETE`, worse than deleting the issue that was named. The check
now lives in `AtlassianClient::request`, not at the ~40 call sites. It rejects
rather than encodes: every value reaching a path segment is an identifier, so
one holding a path or query character is a mistake worth reporting.
`resources.rs` already applied this rule to URIs; the tool side had no
equivalent.

**A page size is capped.** Eight list tools clamped their `limit` to
`MAX_SEARCH_RESULTS`; ten had grown the `unwrap_or` without the `.min`, so
`limit: 100000` went to Atlassian unchanged and flooded the context — the
failure D9 exists to prevent. All eighteen now go through
`mcp::page_size(requested, default)`, which is also what a new tool reaches
for. `confluence_get_space_pages` defaulted to 100, above the cap; it is now
50 and says so. Pagination *offsets* are deliberately not clamped — capping an
offset caps paging itself.

The generalization: when the same decision is written out at N call sites, the
question is not whether one of them is wrong but which one, and adding the
N+1st is how it happens. Two shared helpers came out of the same review for
the same reason — `cached()` (duplicated verbatim in both product clients) and
`save_attachment` / `read_for_upload` (the halves of the attachment tools that
were not product-specific).

## D32. Test the tool layer with invariants, not with 70 tests
A coverage run put the tool wrappers at **13.9%** while the clients sat at 76%
and the server composition at 84%. The tests hit both ends and skipped the
middle: wiremock tests exercise the clients directly, end-to-end tests exercise
filtering, auditing and dry run through a handful of tools, and the 70 thin
wrappers in between — parse arguments, call the client, wrap the result — were
covered by neither.

That gap is where all three defects of D31 lived, and the reason is worth
naming: each wrapper is too trivial to look worth testing, yet the wrappers are
exactly where the decisions live that no compiler checks — which annotation,
which default page size, which endpoint path. Trivial code holding unverifiable
decisions is not low risk; it is low *visible* risk.

The answer is not 70 tests that must each be remembered when a tool is added.
It is two tests that enumerate `tools/list` and hold for whatever they find
(`tests/every_tool.rs`), joining the annotation and output-schema checks that
already worked this way:

- **every tool reaches the API with a well-formed path** — arguments are
  fabricated from each tool's own input schema, and the request it issues must
  exist and have no unsubstituted `format!` placeholder, no empty segment and
  no `..`. A tool that issues nothing is not wired to its client.
- **no tool passes an uncapped page size** — every tool that takes a `limit`
  is asked for 100000, and what reaches the wire must be ≤ 50. This is the
  D31 defect turned into something that cannot recur.

The mock answers `{}`, so most calls end in a deserialization error. That is
deliberate: the request is under test, not the response. Both tests found
something on their first run — an argument fabricator that built the wrong
array element type, and `confluence_download_attachment`'s internal
`limit=200`, which is a constant the tool chose rather than a caller's page
size, and is now excluded on that basis rather than by name.

Coverage went 62.6% → 83.0% by line, tool wrappers 13.9% → 72.4%, for two
tests. CI now fails under 80% (`cargo llvm-cov --fail-under-lines`). The floor
is a ratchet: raise it when the number rises, never lower it to make a red
build green.

## D33. A link the API returned is not a model-composed identifier
D31 checks every request path for `?`, `#` and `..` because the values in it
come from the model. That check was also applied to attachment download
links, which come from Atlassian: Confluence's `_links.download` always
carries `?version=…&api=v2`, so `confluence_download_attachment` refused
every real download while the fixture — written without a query string —
stayed green.

The two inputs get the two rules they need. `AtlassianClient::request` keeps
the identifier check. `get_bytes` takes a link the API returned, resolves a
relative one against the base (query intact) and enforces one thing: an
absolute link must share the base URL's origin, because the request carries
the user's credentials.

Origin is also why Jira downloads under OAuth could not work: the base is the
`api.atlassian.com/ex/jira/{cloud_id}` gateway and the `content` URL names the
site. `JiraClient::download_attachment` now takes the attachment, uses
`content` when it is same-origin, and otherwise (Cloud only) asks the gateway's
own `/rest/api/2/attachment/content/{id}`, which redirects to the binary. The
fixture now has the query string; the test failed first.

## D34. Field options come from the screen that offers them
`jira_get_field_options` called `/field/{id}/option`, which on Cloud serves
only options of Connect/Forge-provided fields, and a `/customField/{id}/option`
path that Server/DC does not have. Neither answered for an ordinary select
field.

The options a user can pick are whatever the edit or create screen offers,
and Jira exposes exactly that without administrator rights on every
deployment: `GET /issue/{key}/editmeta` (`fields[id].allowedValues`) and
`GET /issue/createmeta/{project}/issuetypes/{type}` (`values[].allowedValues`,
Cloud and DC ≥ 8.4). The tool takes `issue_key`, or `project_key` with an
optional `issue_type` (name; the project's first type otherwise), and reads
from that screen. Without either, Cloud falls back to the field-context API
(`/field/{id}/context` → `/context/{ctx}/option`), which needs Administer
Jira; Server/DC says which argument to pass. `allowedValues` entries spell
their label `value` (select options) or `name` (priorities, versions,
components); both land in `FieldOption.value`.

Verified against the documented response shapes with wiremock; not yet
against a live instance — the one endpoint worth confirming first is the
createmeta per-issue-type path on an older DC.

## D35. An issue carries its structure, and the custom fields asked for
`IssueFields` modelled ten fields and the tool promised "full fields". The
`fields` argument let the model ask for `customfield_10011` or `issuelinks`,
and serde dropped both without a word — `jira_remove_issue_link` even told
the model to take the link id "from the `issuelinks` field of
`jira_get_issue`", which nothing returned.

Two additions, D4 intact:

- **Structure is modelled.** `parent`, `subtasks`, `issuelinks` (with the
  link `id` and the far issue's key, summary and status), `components`,
  `fix_versions`, `resolution`, `duedate`. These are what a plan needs and
  what an LLM cannot guess.
- **The rest is `extra`, kept only when asked for.** `#[serde(flatten)]`
  collects the unmodelled fields; `prune_extra` then keeps those the caller
  named (or all of them under `*all` / `*navigable`) and drops nulls. Without
  the pruning the Agile endpoints' unasked `sprint`/`epic` and a `*all` of a
  hundred empty custom fields would flood the context; with it, a model that
  resolved `customfield_10011` through `jira_search_fields` can read it.

`get_issue(key, None)` requests `DEFAULT_ISSUE_FIELDS` — everything the
struct models — rather than everything the instance has. The `jira://`
resource used to carry its own copy of that list; there is now one.

## D36. Section edits happen on the storage document
`confluence_update_page_section` converted the whole page to Markdown,
spliced the new section in, and converted the whole page back. D10 says
what reading does to a macro — it degrades to its text — so editing one
section silently rewrote every other section without its code blocks, panels,
page links and images. The tool's description promised "leaving the rest of
the page untouched".

`mcp_atlassian_storage_markdown::replace_section` now finds `<hN>…</hN>` in the storage
XHTML, takes the run up to the next heading of level ≤ N, and replaces only
that slice. The bytes outside it are the bytes Confluence sent. The
replacement is converted from Markdown on its own, which is the only part
that ever needed converting. Heading text is compared after tags are
stripped, entities decoded and whitespace collapsed, so the heading the
model read in Markdown matches the one in storage. A heading inside a macro
body (expand, panel) is matched like any other, which is the one shape this
cannot edit safely; the tool says so.

Two smaller conversion fixes landed with it, because the section path made
them visible: fenced code blocks become `code` macros (and `code` macros
become fenced blocks with their language) instead of `<pre>`, and raw HTML
in Markdown passes through comrak — the input is the model writing to the
user's own instance, and stripping a hand-written macro to a comment helps
nobody. CommonMark forbids `:` in a tag name, so `<ac:…>`/`<ri:…>` are
spelled with a `-` while comrak looks at them and restored afterwards.

## D37. Attachment tools are sandboxed to `ATTACHMENT_DIR`, streamed and capped
D31 fixed the annotation of the download tools; the capability behind them
was still unbounded. `save_path` could be any path the process can write —
`~/.ssh/authorized_keys` — and `file_path` any file it can read, uploaded to
whatever Jira the credentials reach. Both take one prompt injection.

`ATTACHMENT_DIR` names the one directory the attachment tools may read from
and write to. Paths are canonicalised before the check, so `..` and symlinks
cannot lead out; a relative path is taken under the directory; an existing
symlink at `save_path` is refused rather than followed, and so is a
directory. Unset means the whole filesystem, as before — and the server says
so at startup, at WARN, whenever an attachment tool is registered. The
default stays open because the common deployment is a developer's own
machine, where "any path" is what they asked for; the warning is there so
that a container deployment does not stay open by accident.

`MAX_ATTACHMENT_BYTES` (default 50 MB, `0` for none) caps both directions.
Downloads stream from the socket to the file one chunk at a time and stop —
removing the partial file — at the limit; uploads are checked by size before
they are opened, then streamed with a `Content-Length`. Before this a 200 MB
attachment was held in memory twice by a server whose target is 30 MB.
The two helpers live in `mcp_atlassian_client::mcp::FileAccess` and are handed to
each product's tool state; a product cannot write a file without going
through them.

## D38. `Debug` never prints a credential
`Auth` derived `Debug`, so a `{:?}` of the configuration — in a log line, a
panic, a failing assertion — would print the API token or PAT. Nothing did
that, which is the kind of guarantee that lasts until someone writes
`tracing::debug!(?config)`. `Auth` now writes its own `Debug`: the variant,
the username, and `<redacted>` where the token is. A test formats a full
`Config` and asserts the tokens are absent. `OAuthSession` already did this.

## D39. The HTTP transport can require a bearer token
D18 delegated authentication to a reverse proxy, and still does for TLS. But
`HOST=0.0.0.0` without a proxy made the server an open write proxy to the
configured Atlassian account, and the cost of a token check is twenty
lines. `MCP_BEARER_TOKEN` (or `_FILE`) makes every `/mcp` request carry
`Authorization: Bearer …`; the comparison is constant-time; a mismatch is
`401` with `WWW-Authenticate: Bearer` and never reaches the protocol layer.
`/healthz` is exempt — it answers `ok` without touching Atlassian, so a
liveness probe needs no secret and cannot be used to probe one. Binding to a
non-loopback address without a token is logged at WARN. The transport also
stops cleanly on SIGTERM / Ctrl-C now, which is what `docker stop` sends.

Every variable the server reads moved into `Config` with this: `TRANSPORT`,
`HOST`, `PORT`, `ALLOWED_HOSTS` and `NO_BANNER` were read in `main.rs`, the
rest in `config.rs`, so a bad `TRANSPORT` failed after the clients were
built and the banner could not print the bind address. `Config::read` takes
an `Env` — the process environment in `main`, a map in tests — which is what
made the configuration matrix testable end to end for the first time.

## D40. Retries are bounded and only for what is safe to replay
One retry on 429 was the whole policy. A connection reset, a timeout, a 503
from a gateway restart each surfaced as a failed tool call, and the model
retried it — with a fresh reasoning step in between. `AtlassianClient::send`
now retries up to twice with a short backoff (500 ms, 1 s): a 429 for any
method after `Retry-After`, because the request was refused, not performed;
transport failures and 502/503/504 for GET only, because a POST that timed
out may have landed. Uploads are streamed and not cloneable, so they are
never retried. `REQUEST_TIMEOUT` (default 30 s) is per request; downloads
and uploads get ten times it.

All product clients now share one `reqwest::Client`. Each client parses the
compiled-in root store and keeps its own pool, and there were three — one
per service, one for OAuth. Timeouts are per request, so the shared client
carries no configuration.

A rotated OAuth refresh token is written back to the `*_FILE` it was read
from, owner-only, via a temporary file and rename, so a restart does not
begin with the token Atlassian just revoked (the D17 limitation). Inline
tokens stay in memory only; there is nowhere safe to write them.

## D41. Deployment is inferred from auth, and can be said outright
D16 infers Cloud or Server/Data Center from the auth mode, and records its
limitation: Data Center behind Basic auth is unsupported. That deployment is
real — PATs disabled by policy, or Jira older than 8.14 — and the failure
mode is quiet: Basic means Cloud, Cloud means `/rest/api/2/search/jql`, and
DC answers 404.

`JIRA_DEPLOYMENT` and `CONFLUENCE_DEPLOYMENT` (`cloud` | `server`, also
`datacenter`/`dc`) say which it is; unset keeps the inference exactly as it
was. Both clients read `ServiceConfig::deployment()`, so there is one place
that decides. OAuth is always Cloud.

Confluence had no notion of deployment at all. `restriction_entry` sent both
`accountId` and `username` for every user and let the server pick; it now
sends the one the deployment expects. The flag is also the hook for what is
coming: Atlassian is moving Confluence Cloud to `/wiki/api/v2/…` and has
announced the sunset of parts of `/rest/api/content`. When that lands, each
affected method grows a `match self.cloud` with the v2 path and shape on the
Cloud arm — an edit per endpoint rather than a rewrite, and Server/DC stays
on v1 untouched. Not done ahead of time: a `match` whose two arms are the
same is noise, and the v2 shapes are not settled.

Two smaller things landed in the same phase because the measurement in
phase 7 asked for them: `RUST_LOG` is read through `Config` like every other
variable and parsed with `tracing_subscriber::filter::Targets`, which takes
the same `crate=level` directives as `EnvFilter` without the regex crates
behind it; and the route filtering in `AtlassianServer::new` is one pass over
`tools/list` instead of three.

## D42. Every tool carries a title and says whether it is idempotent
MCP clients render a tool's `title` in the permission prompt they show
before a call; `jira_transition_issue` reads worse there than "Transition
Jira issue". `idempotentHint` tells a client whether a retry after a lost
response is safe; `openWorldHint` whether the tool may reach beyond the
system it names. All seventy tools now declare all three: a title that names
the product, `openWorldHint: false` (a closed Atlassian instance), and — for
writes — an `idempotentHint` that is true only where a repeat with the same
arguments leaves the same state (assign, watchers, labels, restrictions,
deletes, moves) and false for the rest (a second `add_comment` is a second
comment). `tests/every_tool.rs` enumerates `tools/list` and holds all three,
so a new tool without them fails the suite rather than the client's prompt.

`CONFIRM_DESTRUCTIVE=true` builds on the same annotations: a tool marked
destructive asks the user through MCP elicitation — one boolean form,
"perform this destructive operation?", naming the tool and its arguments —
and runs only on an accepted `confirm: true`. A decline is an error result
("was not performed"), which the audit log records as such and a model
reads as not done. The wrapper is the fourth of its kind (project, dry-run,
audit), re-targeting routes by annotation; it sits after dry run (a call
that will not happen needs no confirmation) and inside auditing. Opt-in and
capability-checked: a client that declared no elicitation support gets the
old behaviour and one warning, because blocking every delete behind a
question nobody can answer would be worse than asking none.

## D43. Reading storage: the common macros become what they mean
`htmd` degrades an `<ac:structured-macro>` to its text, which loses the one
thing a reader needs — what kind of thing it was. Before `htmd` sees the
document, the macros a page usually has are rewritten to plain HTML it can
read: `code` to a fenced block with its language (D36), `info`/`note`/
`warning`/`tip`/`panel` to a blockquote under a bold label, `expand` to its
body under its title, page and attachment links to links carrying the
title, images to `![alt](file)`, task lists to ☑/☐ items, `status` and
`jira` to their text in bold, `toc` to a marker. Nesting is handled by
rewriting the body first; a self-closing macro has no body. Anything else
still degrades to text. This is string scanning over a closed vocabulary,
not an XML parser: `quick-xml` or `roxmltree` would cost ~200 KB for no
correctness gain on this input.

## D44. Sub-resources and completions
`jira://KEY/comments` and `confluence://ID/comments` join the two resource
templates (D24): the comments are what a client attaches right after the
issue or page itself, and the URI parsers already knew how to refuse them.
Anything else after the key is still rejected with the expected shapes.

`completion/complete` answers for `issue_key`, wherever a prompt or a
resource template takes one: project keys (as `KEY-`) until the first dash,
then the project's most recently updated issues that match what was typed.
Other arguments answer with an empty list, which is what the spec says an
argument with nothing to offer does.

Not done, and not going to be: forwarding the server's log to the client
through the `logging` capability. rmcp marks the whole
capability deprecated — SEP-2577 removes it from the protocol — so a model
that needs to know about a retry will keep learning it from the tool's
error text (D13).

## D45. Release: five binaries from CI, coverage on Coveralls
The client that runs this server is a desktop app on someone's laptop, so
the binary has to exist for that laptop: CI builds `x86_64` and `aarch64`
for Linux (musl, static — one file for any distribution and the scratch
image) and macOS, and `x86_64` for Windows, all with the `http` feature. A
`v*` tag turns them into a GitHub release with checksums. Linux `aarch64`
cross-compiles with `taiki-e/setup-cross-toolchain-action`; macOS builds
both targets on the arm64 runner, which Apple's toolchain supports.

A tag also publishes the five crates to crates.io, and the job order is the
point: `release` first, `publish` after it and only if it succeeded. A GitHub
release can be deleted and a container tag can be overwritten, but a published
version can only be yanked — hidden from the resolver, still downloadable
forever. Everything reversible therefore happens before the one thing that is
not. `cargo publish --workspace` orders the crates itself and waits for each to
reach the index; the registry token lives in `CARGO_REGISTRY_TOKEN`.

Guarding both is a `version` job: on a tag it compares `Cargo.toml` against
`GITHUB_REF_NAME` and fails if they differ. Without it the two halves of a
release drift silently — the binary takes its version from the manifest, while
the image tags and the release name come from the git tag, so a tag pushed
without bumping the manifest would publish `:0.2.0` around a binary that
answers 0.1.1, with every job green. The job runs on every event rather than
only on tags, because a skipped job skips everything that needs it, and
`image` needs it on master too; the comparison itself is behind an `if`.

Coverage goes to Coveralls, which accepts the workflow's own `GITHUB_TOKEN`
for a public repository — no secret to manage, one badge in the README next
to the CI status. The 85% floor stays in the same step, so a drop fails the
build before it reaches the badge. Codecov was the alternative and now needs
a token for every upload.

`--version` and `--help` are the only flags; everything else is the
environment (D8). No clap: two string matches before the configuration is
read, so they work on an unconfigured machine.

## D46. Scope, status and what is left
The status and backlog used to live in two handoff files; they were folded
in here and removed, so there is one document to keep true.

**Where things stand (2026-09-03).** 0.1.1 is released and on crates.io; 0.1.0 was the first
tag, and 0.1.1 carries the startup-output ordering (D29) and the CI move off
the actions still running on Node 20. Both tags went green through the whole
matrix: five binaries on the GitHub release with checksums, and
`ghcr.io/d3lph1/mcp-atlassian` published as a two-platform image
(`linux/amd64`, `linux/arm64`, D47). `0.1.1`, `0.1` and `latest` share one
digest; `0.1.0` keeps its own. The image is 5.07 MB and `docker run --rm
ghcr.io/d3lph1/mcp-atlassian:latest --version` answers the version it is
tagged with.

70 tools (40 Jira, 30 Confluence), four prompts, four resource templates,
`issue_key` completion. 245 tests against wiremock and an in-memory MCP
transport, 90.4% line coverage gated at 85, clippy and cargo-deny clean.
Release binary on aarch64-apple-darwin: 3.85 MB stdio, 4.22 MB with `http`;
from CI, 4.83 MB for aarch64 musl and 6.41 MB for x86_64 musl (the spread is
explained in D12). Idle RSS ~2 MB against a 30 MB target.
`cargo bloat --crates` on the unstripped http build: std 419 KB,
`mcp_atlassian` 292 KB, rmcp 244 KB, rustls 175 KB. The 292 KB are mostly
the projected routers (D21) — one boxed closure per tool per wrapper — and
are the one size cut still open; measure before touching.

**Deliberately not done.** Bitbucket, or any third product: the server is
Jira + Confluence, and nothing is generalised in anticipation — a `Product`
descriptor that would fold the two products' spelled-out handling in
`server.rs` into a list was designed and withdrawn, because two products
written out are easier to read than one abstraction with two instances.
SSE transport (removed from the MCP spec). A multi-user auth proxy (D11).
A config file (D28) — multi-instance, two Jiras, is the one case that would
bring one, and it is not asked for. The `logging` capability (D44). Jira
API v3 / ADF (D5). Single-flight for cache misses (D25). Cache invalidation
on writes: no write tool changes anything the cache holds. A CHANGELOG:
release notes are generated from commits.

**Open, in rough order of value.**
- A Homebrew formula. crates.io itself is done: all five crates are
  published at 0.1.1, so `cargo install mcp-atlassian --features http`
  works. Each carries the `mcp-atlassian-` prefix (D15), its own README and
  a copy of the MIT text — cargo packages only what sits inside a crate
  directory, so the root copies of both never reached the archives.
- `--list-tools`, so a user can see the surface without configuring.
- A `jira_release_notes` prompt (fixVersion → grouped summaries).
- JSM (Service Management) requests and queues; Jira `move_issue` across
  projects (Cloud's async task API needs polling); Confluence page
  analytics (Cloud-only).
- `resources/list` seeded from recently updated items, if a client turns out
  to need a non-empty list.
- The audit append is a blocking write on the runtime thread (D23); wrap it
  in `spawn_blocking` if the log ever goes to a network filesystem.

**Verify on a live instance.** Two things wiremock cannot: the createmeta
per-issue-type path (D34) on a Data Center older than 8.4, and Confluence
attachment downloads under OAuth, where the gateway's handling of
`/wiki/download/…` is unverified (D33). The rest of this list is now
settled: the CI matrix's first run took Linux aarch64 cross-compilation,
macOS x86_64 and Windows on the first attempt, Coveralls accepted the
workflow's own token, and both the local and the published Docker images
have been built and run.

## D47. The published image is assembled, not compiled, in CI
`docker build .` compiles the server inside `rust:1-alpine`, which is right
for a local build and wrong for publishing: the release profile is
`lto = true`, `codegen-units = 1`, `opt-level = "z"` (D12), and running that
under QEMU to get `linux/arm64` on an x86_64 runner costs tens of minutes a
run. The matrix already builds both static musl binaries natively (D45), so
the image is a `COPY` of the right one — `Dockerfile.ci`, `ARG TARGETARCH`,
`FROM scratch`. Nothing executes during the build, so buildx needs no
emulation for a two-platform manifest.

The cost is two Dockerfiles. `Dockerfile` stays self-contained because the
README tells a user to run it and because it is the only thing that proves
the source-to-image path still works — CI keeps building it, unpushed. Their
runtime stages must stay identical; that is four lines each.

Tags: a `v*` tag publishes `X.Y.Z`, `X.Y` and `latest`; master publishes
`edge` and `sha-<commit>`. `latest` therefore always means the newest
release — a user who pulls it never lands on a half-finished master, and
someone who wants the tip asks for `edge` on purpose.

Auth is the workflow's own `GITHUB_TOKEN` with `packages: write`, so there
is no secret to manage. The first push creates the package private — it has
to be made public once by hand in the package settings, and the repository
linked from there.
