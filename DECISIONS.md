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

- `crates/atlassian-client` — shared HTTP: auth (token/PAT/OAuth), retries,
  error mapping, env configuration, plus the shared MCP result types behind an
  `mcp` feature. Depends on reqwest, serde.
- `crates/atlassian-jira` — the Jira client, its models and (behind `mcp`) its
  tools.
- `crates/atlassian-confluence` — the same for Confluence.
- `crates/storage-markdown` — storage XHTML ↔ Markdown (htmd/comrak). No
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
they do not pull in rmcp (verified: `cargo tree -p atlassian-jira` shows no
rmcp until `--features mcp`).

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
  `atlassian-jira`/`atlassian-confluence`, not a feature. It is not an MCP
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
same split as D15/D21, and it stays true for a third product.

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
wanted, and the matcher is 15 lines (`atlassian-client/src/tool_filter.rs`).
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
