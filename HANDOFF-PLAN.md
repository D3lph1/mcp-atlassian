# Handoff: improvement plan (2026-09-02)

A full read of the workspace (7 400 lines of source, 3 800 of tests, 32
decisions) with one question: what should the next phases do. `cargo test`
(172 green), `cargo clippy --all-targets --all-features -D warnings` and
`cargo fmt --check` are all clean at `109c3eb`, so nothing here is "fix the
build". Everything is either a defect the tests do not see, a gap between what
a tool description promises and what its model delivers, or structure that
will hurt the next change. Scope is Jira + Confluence only — no Bitbucket,
no third product (owner's decision, 2026-09-02).

Companion to `HANDOFF.md` (status + feature backlog) and `DECISIONS.md`
(D1–D32). Items reference both; new decisions the work would need are marked
**→ Dnn**.

Priorities: **P0** a user hits it on a real instance today; **P1** security or
the project's own stated goal (footprint, D13 errors); **P2** refactor that
pays off on the next feature; **P3** polish.

---

## 1. Defects (P0) — **done in phase 6 (D33–D36)**

### 1.1 `confluence_download_attachment` is broken on real instances
`atlassian-client/src/http.rs:101-103` runs `check_path` on a relative
download link. Confluence's `_links.download` always carries a query string:
`/download/attachments/123/x.png?version=1&modificationDate=…&api=v2`. The
D31 guard rejects `?`, so every real download fails with "request path
contains a query string". The wiremock fixture
(`atlassian-confluence/tests/attachments_versions_admin.rs:28`) uses a link
*without* a query, which is why the suite is green.

Fix: `get_bytes` takes a URL the *API* returned, not one the model composed,
so it is not the D31 threat. Split the two concerns: `check_path` stays on
`request()` (model-composed identifiers); `get_bytes` parses the relative link
with `base_url.join`, and enforces only same-origin. Add the query string to
the fixture so the test fails first.

### 1.2 `jira_get_issue` promises "full fields" and cannot deliver them
`IssueFields` (`atlassian-jira/src/models.rs:61-83`) keeps ten fields. The
tool description says "with full fields including description"; the `fields`
argument lets the model ask for `customfield_10011`, `issuelinks`, `parent`,
`subtasks`, `components`, `fixVersions`, `resolution`, `duedate` — and serde
silently drops all of them. Concretely: `jira_remove_issue_link` tells the
model to take the link id "from the `issuelinks` field of jira_get_issue"
(`tools/links.rs:29`), a field no tool ever returns. Custom fields cannot be
read at all, only written.

Fix (D4 still holds — we still do not model the whole payload):
- add the structural fields to `IssueFields`: `issuelinks` (id, type name,
  inward/outward key), `parent` (key), `subtasks` (key, summary, status),
  `components`, `fix_versions`, `resolution`, `duedate`, `attachment` count.
- add `#[serde(flatten)] pub extra: Map<String, Value>` **filtered to the
  fields the caller asked for**: when `fields` is given, keep the requested
  unknown keys; when it is not, keep `customfield_*` keys only. Without the
  filter `jira_get_issue` with no `fields` returns every custom field the
  instance has, which is the flood D9 exists to prevent.
- `get_issue(key, None)` should request a default list (what `RESOURCE_FIELDS`
  in `resources.rs:648` already is) instead of everything; move that constant
  to the client.
- Test: an issue fixture with `issuelinks` and a custom field reaches the
  tool's `structuredContent`.

### 1.3 `confluence_update_page_section` destroys macros on the whole page
`tools/pages.rs:751-777`: the *entire* page goes storage → Markdown →
storage. `htmd` degrades every `ac:structured-macro` to its text (D10 says so
for reading), so editing one section rewrites the other sections without
their code blocks, panels, page links, images, tables of contents, Jira
macros. The tool's description says "leaving the rest of the page untouched".

Fix: operate on storage XHTML directly. Find `<hN>…heading…</hN>`, take
everything up to the next `<hM>` with M ≤ N, replace only that slice with the
converted replacement, leave the rest byte-for-byte. A tolerant scan (the
storage format is well-formed XHTML, headings are never nested) is ~40 lines;
no XML parser needed. Keep the Markdown path as the fallback for a page whose
headings are not `<hN>` elements, and say so in the result. Test: a page with
a macro outside the edited section round-trips the macro unchanged.

### 1.4 `jira_get_changelog` ignores `max_results` on Server/DC
`atlassian-jira/src/lib.rs:583-590`: DC fetches the issue with
`expand=changelog` and returns every history entry. Truncate client-side to
`max_results` (newest first, which is the order Cloud returns).

### 1.5 `jira_get_field_options` on Cloud is likely the wrong endpoint
`lib.rs:447-451` calls `/rest/api/2/field/{id}/option`. On Cloud that endpoint
serves options of *Connect/Forge-provided* fields only; ordinary select
custom fields answer through
`/rest/api/2/field/{id}/context` → `/context/{contextId}/option`. Not
verifiable with wiremock; verify against a Cloud instance, and if confirmed,
route through the context API (first context, then options). The DC path
`/rest/api/2/customField/{id}/option` needs the same check.

### 1.6 Attachment downloads under OAuth cannot be same-origin
`http.rs:110`: under OAuth the base is `api.atlassian.com/ex/jira/{cloud_id}`
while Jira's `content` URL points at `{site}.atlassian.net`, so
`jira_download_attachment` always refuses with "foreign origin". Fix: under
OAuth, rewrite the attachment URL's path onto the gateway base (the
`/secure/attachment/…` path is served there too), or allow the site origin
that the `accessible-resources` endpoint reports for the cloud id. Same
question for Confluence `_links.download`.

### 1.7 `check_path`'s error text has 18 spaces in it
`http.rs:274` — a string literal broken across lines without `\`. Cosmetic,
but it reaches the model. One-line fix; the existing test only checks
substrings.

### 1.8 JQL/CQL string interpolation without escaping
`atlassian-jira/src/lib.rs:602` `project = "{project_key}"` and
`atlassian-confluence/src/lib.rs:253, 362` build queries with `format!`. A
key or query containing `"` breaks the query, and `"` OR … is an injection —
read-only and same-user, so not a privilege problem, but the error message it
produces is Atlassian's, not ours (D13). Add `jql_string(&str)` /
`cql_string(&str)` helpers that escape `\` and `"`, and validate project and
space keys against `^[A-Z][A-Z0-9_]*$` before use, with an error that names
the argument.

---

## 2. Security hardening (P1) — **done in phase 7 (D37–D39)**; 2.5 see CI

### 2.1 Local filesystem access is unrestricted
`mcp::save_attachment` writes to any path the model names, and
`read_for_upload` reads any file — `~/.ssh/id_ed25519` uploaded to a public
Jira is one prompt injection away. D31 fixed the *annotation*; the capability
is still unbounded.

Add `ATTACHMENT_DIR` (**→ D33**): when set, `save_path` and `file_path` must
resolve (after canonicalisation, so symlinks and `..` do not escape) inside
it; when unset, keep today's behaviour but log a startup WARN that attachment
tools have unrestricted filesystem access. Documented in CLAUDE.md's table
next to `AUDIT_LOG_FILE`. Reject a `save_path` that is a directory, and do not
follow an existing symlink at `save_path` (`O_NOFOLLOW`, or check before
write).

### 2.2 `Debug` prints credentials
`config.rs:11-20`: `Auth` derives `Debug`, so `Config`'s derived `Debug`
prints the API token and PAT. Nothing formats it today, which is exactly the
kind of guarantee that lasts until someone adds `tracing::debug!(?config)`.
Hand-write `Debug` for `Auth` that prints the variant and `username` only
(`OAuthSession` already does this). Add a test that `format!("{config:?}")`
does not contain the token.

### 2.3 HTTP transport has no authentication
D18 delegates to a reverse proxy. That is fine for the design, but the cost of
an optional `MCP_BEARER_TOKEN` (**→ D34**) is a 20-line axum middleware, and
without it a `HOST=0.0.0.0` deployment is an open Jira write proxy. Keep the
reverse-proxy advice for TLS; add the token check, constant-time compare, and
a startup WARN when binding non-loopback without it.

### 2.4 Audit log content
Arguments are logged verbatim (D23, deliberate). Two small things: record the
*result* id for creates (`CreatedIssue.key`, page id) so an audit line is
enough to undo; and document the file's expected permissions (create with
`0o600` on Unix via `OpenOptions::mode`).

### 2.5 Dependency auditing in CI
No `cargo audit` / `cargo deny` job. Add `EmbarkStudios/cargo-deny-action`
with `advisories` and `licenses` checks; the tree is small enough that this
stays green cheaply.

---

## 3. Footprint (P1 — the project's stated goal) — **done in phase 7**

### 3.1 Attachments are buffered whole in memory
`get_bytes` (`http.rs:129`) does `bytes().await?.to_vec()` — two copies of
the file; `post_multipart` and `read_for_upload` hold the whole upload. A
200 MB attachment puts the server at seven times its "< 30 MB" target.
Stream: `resp.bytes_stream()` → `tokio::fs::File` for downloads;
`reqwest::Body::wrap_stream(ReaderStream::new(file))` for uploads (needs
`reqwest/stream`, `tokio-util/io`; measure with `cargo bloat`). Add
`MAX_ATTACHMENT_BYTES` (default 50 MB) as a hard cap in both directions, with
an error naming the limit.

### 3.2 Three `reqwest::Client`s, three TLS root stores
`http.rs:39` per service and `oauth.rs:67` — each `Client::builder().build()`
parses the compiled-in webpki root set and holds its own connection pool.
Build one `reqwest::Client` in `Config::from_env` (or lazily in a
`OnceLock`) and share it. Measure idle RSS before/after; expect a visible
drop on the ~2 MB baseline.

### 3.3 `rmcp` default features
`rmcp = { version = "3", features = ["server", "transport-io"] }` keeps
defaults (`base64`, `macros`, `server`). `macros` is needed; check whether
`base64` is, with `default-features = false` and `cargo bloat --release
--crates`. Also record the duplicate `base64` 0.22/0.23 and `getrandom`
0.2/0.4 pairs in D12's notes — they are upstream (reqwest vs rmcp), not ours,
but they are ~100 KB the next size review will otherwise rediscover.

### 3.5 Size follow-up after phase 7 — `EnvFilter` → `Targets` **done**; `mcp_atlassian` router projections still open
The http build grew from 3.93 to 4.45 MB across phases 6–7 (stdio 3.97 →
4.07). `cargo bloat --release --features http --crates` on the unstripped
binary: std 419 KB, `mcp_atlassian` 292 KB, `atlassian_jira` 244 KB, rmcp
244 KB, tracing-subscriber 176 KB + `regex_automata`/`regex_syntax` 131 KB,
rustls 175 KB. Two cuts worth taking, cheapest first:
- `EnvFilter` is the only user of `regex` in the tree. `tracing_subscriber::
  filter::Targets` parses the same `crate=level` directives without it;
  switching (and dropping the `env-filter` feature) should remove ~130 KB.
  Directive-level regex matching (`[span{field=value}]`) is lost; nobody
  uses it here.
- `mcp_atlassian` at 292 KB is mostly the projected routers (D21): one
  boxed closure per tool per wrapper (project, dry-run, audit). Measure
  before touching; a single generic wrapper that dispatches on a small enum
  might halve it, or might not.
Also raise the CI coverage floor to 85 (currently 89.9% measured, 80 gated).

### 3.4 Unused dependencies — **done**
`mcp-atlassian/Cargo.toml`: `storage-markdown` is not referenced by the server
crate; `tokio/signal` is enabled but unused. Remove both. Consider
`cargo-machete` in CI.

---

## 4. Model gaps the LLM runs into (P1) — **done in phase 6**, except `url` fields

Beyond 1.2, the places where a tool returns less than its description implies:

- **Pagination is one-way.** `ListResult` has `count` only; `ResultsPage` has
  `size` but not `start`/`limit`/`_links.next`; `AgilePage` has `is_last` but
  no `start_at`, and no agile tool accepts one. A model that gets 25 of 300
  board issues has no way to ask for the next 25. Add `start`/`limit`/
  `has_more` to `ResultsPage`, `start_at`+`is_last` to `AgilePage`, and a
  `start_at` argument to `jira_get_board_issues`, `jira_get_sprint_issues`,
  `jira_get_comments`, `confluence_get_page_children`,
  `confluence_get_attachments`. `every_tool.rs` already asserts offsets are
  not capped, so the invariant is in place.
- **`confluence_get_space_page_tree` silently truncates at 50 pages** and
  presents the result as "the hierarchy". Either page internally up to a
  bound (500) or return `truncated: true` when `size == limit`.
- **`jira_get_worklog` and `confluence_get_labels` are unbounded** — no
  `maxResults`/`limit`. Add one through `page_size`.
- **`jira_get_sprints` has no page size** and no `start_at`.
- **`jira_create_remote_link` returns `serde_json::Value`** — the one tool
  whose `outputSchema` is `true`. Model `RemoteLink { id, self_url }`.
- **`Content` lacks `_links.webui`** so no tool can give the user a URL to a
  page; `Issue` lacks `self`/browse URL likewise. Add `url` fields (Jira:
  `{base}/browse/{key}`, computed; Confluence: `_links.base + _links.webui`).
- **`Comment.body` on Cloud v2** is plain text/wiki markup (D5), fine — but
  `jira_get_comments` returns `CommentPage` with `total` while the tool
  description says "newest first" and the client sends `orderBy=-created`;
  Server/DC ignores `orderBy` on that endpoint (returns oldest first). Sort
  client-side by `created` descending so both deployments match the
  description.
- **Silent parameter drops by deployment:** `start_at` is ignored on Cloud,
  `next_page_token` on DC (`lib.rs:118-127`). Return an `invalid_params`
  error naming the parameter and the deployment instead of ignoring it.

---

## 5. Structure and refactoring (P2)

### 5.1 The server crate hard-codes two products — **withdrawn**
`server.rs` names Jira and Confluence in about seven places (fields,
`*_available` flags, projections, prefix checks, prompt pruning) and
`main.rs::group_by_product` in one more. A `Product` descriptor would fold
those into a list — and its only real payoff was a third product. There is
none planned (no Bitbucket, no JSM crate for now), and two products spelled
out are easier to read than one abstraction with two instances. Left as is;
revisit only if a third product is ever decided.

### 5.2 `Config` owns only half of the environment — **done (D39)**
`TRANSPORT`, `HOST`, `PORT`, `ALLOWED_HOSTS`, `NO_BANNER`, `NO_COLOR` are read
in `main.rs`, the rest in `config.rs`. Move them into `Config` (a `Transport`
enum with `Http { host, port, allowed_hosts }`), so one place validates,
the banner can print the bind address, and a typo in `TRANSPORT` fails before
the clients are built rather than after.

### 5.3 `Config::from_env` reads the process environment directly — **done**
Untestable end-to-end: `oauth_from_env`, the `_FILE` branches for OAuth, the
"neither service configured" error, and every combination of `READ_ONLY` /
`DRY_RUN` / filters have no test through `from_env`. Take the environment as
`impl Fn(&str) -> Option<String>` (`Config::from_env()` passes
`|k| env::var(k).ok()`), then test with a `HashMap`. This also removes the
`MCP_TEST_SECRET_*` process-env juggling in `config.rs` tests.

### 5.4 73 `Config { … }` struct literals in tests — **done**
Every new `Config` field breaks 73 sites. Add `Config::single(base_url, auth)`
(one service, everything else default) and `#[derive(Default)]` where the
defaults are meaningful; migrate tests to it in the same change as 5.2, which
would otherwise break them all.

### 5.5 Confluence has no deployment knowledge — **done (D41)**
`ConfluenceClient` has no `cloud` flag, so `restriction_entry`
(`lib.rs:456-472`) sends both `accountId` and `username` for every user and
hopes. Derive `cloud` from the auth mode exactly as Jira does (D16), and send
the right one. This is also the hook for 5.6.

### 5.6 Confluence Cloud v1 REST is on a deprecation path — **hook in place (D41)**; the per-endpoint `match` waits for v2 to be needed
Atlassian is moving Cloud to `/wiki/api/v2/…` (pages, spaces, attachments,
versions, labels) and has announced sunset of parts of `/rest/api/content`.
D16 records what it cost when Jira did this to `/search`. Do not migrate now;
do put the switch in place: `ConfluenceClient` gains `cloud: bool` (5.5) and
every method that hits an endpoint v2 replaces routes through one `match`,
with v1 on both arms today. Record the plan as **→ D36** so the migration is
a set of arm edits, not a rewrite.

### 5.7 `to_mcp_error` flattens every error to `internal_error` — **done**
`mcp.rs:161`: `NotFound`, `InvalidUrl`, `Config` (e.g. "no Epic Link field")
are the caller's problem and should be `invalid_params` (-32602); 401/403 are
neither and deserve `internal_error` with `data: {"status": 401}`; `Api {
status: 400, .. }` is `invalid_params`. Clients that branch on the code
(retry on internal, do not retry on invalid params) currently retry
everything. One `match` in `to_mcp_error`, one test per arm.

### 5.8 Deployment override — **done (D41)**
D16 infers Cloud/DC from the auth mode and states the limitation: DC behind
Basic auth is unsupported. That is a real deployment (PATs disabled by
policy, or Jira < 8.14). Add `JIRA_DEPLOYMENT=cloud|server` /
`CONFLUENCE_DEPLOYMENT` as an explicit override, inference as the default.

### 5.9 Smaller — **done** except the audit `spawn_blocking` (noted, not needed on a local disk) and the `router_ext` clone (rmcp owns the context)
- `http.rs:102-105`: the `starts_with("http")` test is evaluated twice; hoist
  into one `if let Ok(url) = Url::parse(...)` with a scheme check.
- `AtlassianServer::new` calls `tool_router.list_all()` three times and
  allocates the name list twice; one pass.
- `router_ext.rs` clones `arguments` on every call; `ToolCallContext` fields
  are owned, so `context.arguments.take()` would do if rmcp allows it — check
  before changing.
- `audit.rs:1311`: `File::write_all` on the `current_thread` runtime blocks
  every in-flight call for the duration of the write. Fine on a local disk;
  wrap in `spawn_blocking` if the log ever goes to NFS, or note it in D23.
- `oauth.rs`: `expires_in` defaulting to `0` means a token response without
  it refreshes on every call. Default to 3600 and log once.
- `prompts.rs` and `resources.rs` both compute "the fields we model" — after
  1.2 there is one constant, in the client.

---

## 6. MCP surface (P2) — **done in phase 9 (D42–D44)**, except 6.5 (deprecated upstream)

### 6.1 Tool `title`, `idempotent_hint`, `open_world_hint`
rmcp 3.2 supports all three (`model/tool.rs:22, 82, 91`; the `#[tool]` macro
takes `title`). Clients render `title` in permission prompts, where
`jira_transition_issue` reads worse than "Transition Jira issue".
`idempotent_hint = true` on `assign_issue`, `add_label`, `set_restrictions`,
`update_issue` lets a client retry safely; `open_world_hint = false` on
everything (a closed Atlassian instance). Extend the `every_tool.rs`
annotation invariant: every tool has a title, every write declares
`idempotent_hint`.

### 6.2 Elicitation for destructive tools (backlog item, now concrete)
rmcp's `elicitation` feature is present (`service/server.rs:884`). A fourth
router wrapper in the D23/D26 shape: for routes with `destructive_hint`, ask
the client to confirm (`ElicitRequest` with a one-field boolean schema) before
calling through; on decline return a `StatusResult`-shaped "not performed".
Opt-in via `CONFIRM_DESTRUCTIVE=true`, because a client that does not
implement elicitation would see every delete hang. Detect the client
capability from `RequestContext::peer` and skip the prompt when absent, with
a WARN.

### 6.3 Completions
`completion/complete` is in rmcp (`handler/server.rs`). Prompt argument
`issue_key` and resource templates `{issue_key}` / `{page_id}` can complete
from a cached project list (`PROJ-` prefix) and from a bounded CQL title
search. Small, and it is the difference between typing `/jira_issue PROJ-1`
and picking it.

### 6.4 More prompts (backlog, D30 shape)
`jira_standup` (active sprint of a board: what moved since yesterday, from
the changelog), `jira_triage` (unassigned bugs by priority), `jira_release_notes`
(fixVersion → grouped summaries), `confluence_page` (brief on a page with its
newest comments). Each fetches its own data and ends with a bounded ask.

### 6.5 Logging capability — **withdrawn**: rmcp deprecates it (SEP-2577 removes logging from MCP)
Forward `tracing` WARN/ERROR to the client as `notifications/message`
(rmcp `enable_logging`). A model that sees "rate limited, retrying" behaves
differently from one that sees a 30 s silence.

### 6.6 Resource sub-paths
`jira://PROJ-1/comments`, `confluence://123/children` — the parsers already
reject these with a message that names the expected shape (D24). Cheap to add
once 1.2 lands, and they are what a client attaches after the issue itself.

---

## 7. Content conversion (P2) — **done** (D36, D43)

`storage-markdown` is the thinnest crate (116 lines) for the most visible
output. Three things a page usually has that currently vanish:

- **Reading:** pre-process the common macros before `htmd`: `ac:structured-
  macro[code]` → fenced block with the language parameter; `info`/`note`/
  `warning`/`tip` → blockquote with a bold label; `ac:link` + `ri:page` →
  `[title](confluence://id)` (the resource URI — the model can follow it);
  `ac:image` + `ri:attachment` → `![filename]`; `ac:task-list` → `- [ ]`;
  `expand` → the body; `toc`, `jira`, `status` → their text. A tolerant
  string scan per macro; no XML crate.
- **Writing:** `comrak` with `render.unsafe_ = false` strips raw HTML into
  `<!-- raw HTML omitted -->`, so a model that writes `<ac:structured-macro>`
  in Markdown loses it. Input is trusted (the model, on behalf of the user, to
  their own instance), so enable `unsafe_`. Also map fenced code blocks to the
  `code` macro so they render as Confluence code blocks rather than `<pre>`.
- **Round-trip test:** for each supported macro, `markdown_to_storage(
  storage_to_markdown(x))` preserves the semantic (fixture-based, a handful
  of cases). This is what makes 1.3's fallback path and `confluence_update_
  page` with Markdown content honest.

---

## 8. Reliability (P2) — **done in phase 7 (D40)** except cache invalidation (nothing to invalidate)

- **Retries:** `send()` retries 429 once. Add bounded retry with jittered
  backoff (3 attempts, 0.5–4 s) for connection errors, timeouts, 502/503/504
  — on idempotent requests only (GET, and PUT/DELETE where the body is
  cloneable). POST stays single-shot. `REQUEST_TIMEOUT` becomes
  `REQUEST_TIMEOUT` env (default 30).
- **OAuth refresh token persistence:** D17's limitation (a rotated token is
  lost on restart) has a cheap opt-in: when `ATLASSIAN_OAUTH_REFRESH_TOKEN_FILE`
  is set and writable, write the rotated token back to it (`0o600`, atomic
  rename). Log the write. The env-var form stays read-only.
- **Cache invalidation on writes** (D25 notes it as absent): a `create_page`
  / `create_project`-shaped write could `invalidate("spaces:*")`. Cheap;
  keeps `CACHE_TTL` from being a trap in an agent that creates then lists.
- **HTTP:** graceful shutdown on SIGTERM (the `signal` feature is already
  on), `/healthz` for Kubernetes probes, and a session idle timeout if
  `LocalSessionManager` does not expire sessions on its own (check).

---

## 9. Tests (P2)

The invariant approach (D32) is right; these are the invariants still
missing:

- **Fixtures with real query strings** (1.1). More generally, one test that
  every `_links.download` / `content` URL shape seen on Cloud and DC passes
  `get_bytes`.
- **`Config::from_env` matrix** (5.3): each env variable × unset/empty/set,
  and the error text names the variable.
- **No `Debug` output contains a token** (2.2).
- **Every tool with a `fields`/`expand` argument returns what it requested**
  (1.2) — fabricate `fields: "customfield_1"` and assert the key is in
  `structuredContent`.
- **Every list tool that can page exposes a way to page** (4): a tool whose
  response type has `is_last`/`has_more` must accept `start_at`/`start`.
- **HTTP transport** has no test at all (`main.rs::http` behind the feature).
  One test with `--features http`: bind on `127.0.0.1:0`, `initialize`,
  `tools/list`, Host-header rejection.
- **Coverage ratchet:** 84% today; raise `--fail-under-lines` to 83 now and
  again after each phase.

---

## 10. Distribution and docs (P3, but blocks a first release) — **done in phase 10 (D45)**, except crates.io / Homebrew and `--list-tools`; no CHANGELOG by decision

- `README.md` — install (binary, Docker, `cargo install`), the env table
  from CLAUDE.md, Claude Desktop / Cursor config snippets, a tool list
  generated from `tools/list` (a small `xtask` or `--list-tools` flag so it
  cannot drift).
- `LICENSE` — `license = "MIT"` is declared; the file is missing.
- `CHANGELOG.md`, `rust-version` in `[workspace.package]`, and
  `rust-toolchain.toml` pinned to stable.
- Release workflow: tag → musl x86_64 + aarch64, macOS arm64, Windows;
  `docker/build-push-action` to GHCR with `linux/amd64,linux/arm64`;
  `cargo publish` in dependency order (client → markdown → jira → confluence →
  server). Docker: `USER 65534:65534` in the scratch stage; `.dockerignore`
  should exclude `.github`, `*.md`, `target` already is.
- `--version` / `--help` / `--list-tools` flags (no clap; `std::env::args`
  with three matches) so a user can check a binary without configuring it.
- Docker image has still never been built on this machine (`HANDOFF.md`).

---

## 11. Suggested sequencing

| Phase | Scope | Items | Size |
|---|---|---|---|
| 6 ✅ | Correctness on real instances | 1.1–1.8, 4 (pagination, worklog/labels caps, changelog), 5.7, 7 (comrak `unsafe_`, code macro) | done 2026-09-02 |
| 7 ✅ | Hardening + footprint | 2.1–2.5, 3.1–3.4, 5.2–5.4 (config), 8 (retries, token persistence) | done 2026-09-02 |
| 8 ✅ | Deployment correctness + size | 5.5, 5.6, 5.8, 5.9, 3.5, coverage floor to 85 | done 2026-09-02 |
| 9 ✅ | MCP surface | 6.1–6.6, remaining 7 (macro reading), 9 | done 2026-09-02 |
| 10 ✅ | Release | 10 | done 2026-09-02 |

Phase 6 first because 1.1 and 1.3 are the two tools a Confluence user
reaches for most after `get_page`, and both currently fail or damage data in
a way the suite cannot see. Phase 7 before 8 because 5.2–5.4 change `Config`,
and every later phase touches it. Phase 8 is what is left of §5 once the
third-product generalisation is withdrawn: the Confluence client learning
which deployment it talks to (5.5) and where its Cloud endpoints will move
(5.6), an explicit deployment override for the auth-mode inference (5.8), the
small refactors (5.9), and the size cuts phase 7 measured (3.5).

Each item that adds an env var or changes a boundary rule gets a DECISIONS.md
entry first (D33–D36 above are placeholders), per CLAUDE.md.

---

## 12. Considered and not proposed

- **A config file.** Still no (D28); multi-instance remains the one trigger.
- **Jira API v3 / ADF.** No (D5); nothing here needs it. Revisit only if v2
  `/search/jql` diverges further from DC.
- **Replacing `htmd`/`comrak` with an XML parser for storage format.** The
  macro pre-processing in §7 is string scanning on a well-formed, closed
  vocabulary; `quick-xml` or `roxmltree` would cost ~200 KB for no
  correctness gain on this input.
- **Streaming `tools/list` pagination.** 70 tools fit in one page; rmcp
  handles the cursor if a client asks.
- **Multi-user proxy / per-request credentials.** Out of scope (D11).
- **Bitbucket, or any third product.** Not planned; the server stays Jira +
  Confluence, and nothing is generalised in anticipation.
- **Single-flight for cache misses.** D25's reasoning still holds.
