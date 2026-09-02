# Handoff

Project status and the idea backlog. Update when finishing a phase.

## Current state (2026-09-02)

Phases 1–5 are done. A functionally complete MCP server for Jira + Confluence:

- **70 tools** (40 Jira + 30 Confluence) — full coverage of the surface
  comparable servers expose: issues, search, transitions, comments, worklog,
  links/epics, fields, agile, watchers, batch create, changelog; Confluence
  pages incl. section edits and moves, inline comments, attachments, version
  history and diff, templates, restrictions
- Auth: API token (Cloud), PAT (Server/DC), OAuth 2.0 refresh flow (D17); every
  token also readable from a file via `*_FILE` (D28)
- Transports: stdio (default), streamable HTTP behind the `http` feature (D18)
- Filtering: `ENABLED_TOOLS` / `DISABLED_TOOLS`, wildcards anywhere in a
  pattern (`jira_*`, `*_get_*`), deny wins (D27); `READ_ONLY` driven by tool
  annotations (40 read-only / 30 write, 14 of them destructive) — D22
- `DRY_RUN=true`: write tools stay listed, are validated against their own
  input schema and described instead of performed; reads still execute (D26)
- Audit log: `AUDIT_LOG_FILE` appends one JSONL record per write call —
  timestamp, tool, arguments, outcome, duration, `destructive` and `dry_run`
  flags (D23)
- Startup banner on stderr (never stdout — that is the protocol), colour only
  for a TTY; `NO_BANNER` falls back to the structured line; the registered
  tools are then logged per product (D29)
- Markdown ↔ Confluence storage (htmd/comrak, D10)
- Structured output: every tool advertises an `outputSchema` and returns
  `structuredContent` (D20)
- Resources: `jira://PROJ-123` (JSON), `confluence://123456` (Markdown) and
  their `/comments` sub-resources; `resources/list` intentionally empty
  (D24, D44); `issue_key` completion for prompts and templates
- Prompts: `/jira_issue`, `/jira_triage`, `/jira_standup`, `/confluence_page`
  — each fetches its own data, then asks (D30)
- Tool annotations: `title`, `readOnlyHint`, `destructiveHint`,
  `idempotentHint`, `openWorldHint` on all 70; `CONFIRM_DESTRUCTIVE` asks
  through elicitation (D42)
- TTL cache: `CACHE_TTL` seconds, reference data only (projects, issue types,
  boards, link types, fields, spaces), off by default (D25)
- Tests: 245 (wiremock + end-to-end over an in-memory transport, plus the
  HTTP transport over a real socket), coverage 90.4% by line gated at 85,
  clippy clean; CI (fmt/clippy/test, cargo-deny, musl artifact, docker,
  coverage gate); scratch Dockerfile
- Binary (aarch64-apple-darwin, release): 3.85 MB stdio / 4.22 MB with
  http after phase 8. Phases 6–7 had grown it to 4.07 / 4.45 MB (streaming
  attachments, the config matrix, the HTTP middleware); dropping
  `EnvFilter` for `Targets` took the regex crates out and 216 KB with them,
  so stdio is now below where phase 5 left it (3.97 MB). `cargo bloat
  --crates` (unstripped): std 419 KB, `mcp_atlassian` 292 KB (router
  projections, the one cut still open — HANDOFF-PLAN §3.5), rmcp 244 KB,
  rustls 175 KB. Idle RSS ~2 MB, target < 30 MB

Deliberately not done: SSE transport (deprecated in MCP), multi-user proxy.

Known loose ends:
- [ ] The Docker image has not been built locally (daemon / credential helper
      on this machine); the CI `docker` job should verify it
- [ ] The CI matrix (Linux aarch64 cross, macOS x86_64, Windows) and the
      Coveralls upload have not run yet — the first push will say. A
      private repository needs `COVERALLS_REPO_TOKEN`

## Feature backlog (prioritized)

### Top (value / effort)
- [x] **TTL cache** — `CACHE_TTL` seconds, opt-in, reference data only (D25).
- [x] **`DRY_RUN=true`** — write tools return a description of the action
      without performing it (D26). A safety net for demos and prompt debugging.

### MCP protocol features (rmcp supports them)
- [x] **Resources** — `jira://PROJ-123`, `confluence://123456` as resource
      templates (D24). Possible follow-ups: sub-resources
      (`jira://PROJ-1/comments`), and `resources/list` seeded from recently
      updated items if a client turns out to need a non-empty list.
- [x] **Prompts** — `jira_issue`, `jira_triage`, `jira_standup`,
      `confluence_page` (D30). Still possible: release notes from a
      fixVersion.
- [x] **Elicitation** — `CONFIRM_DESTRUCTIVE`, D42.

### Security / operations
- [x] Audit log of write operations (JSONL to a file) — `AUDIT_LOG_FILE`, D23
- [x] Secrets from files — `*_FILE` on every token variable, D28
- [ ] Multi-instance (two Jiras) — needs a TOML config instead of env vars,
      plus tool prefixes. This is the *only* open case for a config file; a
      file as a second spelling of the current settings was considered and
      rejected (D28)

### Wider coverage
- [ ] Jira: `move_issue` across projects (Cloud async task API — needs task
      polling, deliberately skipped for now)
- [ ] Confluence: page view analytics (Cloud-only analytics API)
- [ ] JSM (Service Management): requests, queues

Not doing: Bitbucket. The product scope is Jira + Confluence; no third
product is planned, so no generalisation of the server crate for one
(HANDOFF-PLAN §5.1 withdrawn).

### Distribution
- [x] Release binaries on GitHub: Linux musl x86_64/aarch64, macOS
      x86_64/aarch64, Windows x86_64, on a `v*` tag (D45)
- [ ] Publish to crates.io (`cargo install mcp-atlassian`) — the path deps
      carry versions already; publish client → markdown → jira → confluence
      → server
- [ ] Homebrew formula

## Notes

**Phase 10 (2026-09-02, D45).** README with CI and coverage badges, MIT
LICENSE, `--version`/`--help`, `USER` in the Dockerfile, and the CI
pipeline: check (fmt, clippy, test), coverage (llvm-cov → Coveralls, floor
85), cargo-deny, a five-target binary matrix, docker, and a release job on
`v*` tags. No CHANGELOG by decision of the owner; release notes are
generated from commits.

**Phase 9 (2026-09-02, D42–D44).** MCP surface: `title` / `idempotentHint`
/ `openWorldHint` on every tool with an invariant in `every_tool.rs`;
`CONFIRM_DESTRUCTIVE` elicitation wrapper, capability-checked; comments
sub-resources for both products; `issue_key` completion; prompts
`jira_triage`, `jira_standup`, `confluence_page` (Confluence gained its
`prompts` module); storage-markdown reads panels, links, images, task lists,
expand, status, jira and toc macros. Not done: the `logging` capability —
deprecated by SEP-2577, rmcp warns on every use.

**Phase 8 (2026-09-02, D41).** `JIRA_DEPLOYMENT` / `CONFLUENCE_DEPLOYMENT`
override the auth-mode inference (Data Center behind Basic auth);
`ConfluenceClient` knows its deployment and sends `accountId` or `username`
in restrictions accordingly — and that flag is where the Cloud v2 endpoints
will branch. `RUST_LOG` moved into `Config` and is parsed with `Targets`
(the `env-filter` feature and its regex crates are gone). Route filtering is
one pass. OAuth token responses without `expires_in` assume an hour. CI
coverage floor raised to 85. Withdrawn: the `Product` descriptor (no third
product), fake v1/v1 `match` arms.

**Phase 7 (2026-09-02, D37–D40).** Hardening and footprint: `ATTACHMENT_DIR`
sandbox with canonicalised paths and symlink refusal, `MAX_ATTACHMENT_BYTES`
with streamed downloads and uploads (D37); `Auth` redacts tokens in `Debug`
(D38); `MCP_BEARER_TOKEN` on the HTTP transport, `/healthz`, graceful
SIGTERM (D39); bounded retries with backoff for GET and 429, `REQUEST_TIMEOUT`,
one shared `reqwest::Client`, rotated OAuth refresh tokens written back to
their `*_FILE` (D40). `Config::read(&dyn Env)` owns every variable and is
tested as a matrix; `Config::default()` replaced 73 struct literals in tests.
The audit record carries `result` (created key/id) and the file is created
`0600`. `storage-markdown` and `tokio/signal` dropped from the server crate's
unconditional deps; rmcp built without default features. Deliberately not
done: cache invalidation on writes — no write tool changes anything the cache
holds (projects, types, boards, link types, fields, spaces).

**Phase 6 (2026-09-02, D33–D36).** Correctness on real instances, from the
review in `HANDOFF-PLAN.md`: Confluence attachment downloads (query string
in the link, D33), Jira downloads under OAuth (D33), field options via the
edit/create screens (D34), `jira_get_issue` returning parent/subtasks/links
and requested custom fields (D35), section edits on storage XHTML so the
other sections keep their macros (D36), fenced code ↔ `code` macro, raw
macros in Markdown surviving comrak, JQL/CQL quoting, changelog and worklog
caps on Server/DC, comments newest-first on both deployments, paging fields
(`start`/`has_more`, `start_at`/`is_last`) and offset arguments on every list
tool that pages, `jira_get_space_page_tree` saying `truncated`, typed
`RemoteLink`, MCP error codes (`invalid_params` for what the caller can
fix). Left for a live instance: D34's createmeta path on older DC, and
Confluence downloads under OAuth (the gateway's handling of `/wiki/download`
is unverified).

**Review of 2026-09-02 (D31).** Three defects, all the same shape — a rule
applied at some call sites and forgotten at others: attachment downloads
annotated read-only while writing to local disk, unescaped path segments
letting an interpolated issue key steer the request elsewhere, and ten list
tools missing the page-size cap. Each fix moved the rule to the boundary. Two
duplications came out with them: `cached()` and the attachment file helpers.
Nothing else in the sweep needed changing — cache discipline (D25), resource
URI validation, OAuth refresh serialization and error wording all held.

**Coverage (D32).** 84% by line (`cargo llvm-cov`), gated in CI at 80%. The
tool wrappers were 14% before `tests/every_tool.rs` — two schema-driven
invariants over `tools/list` rather than 70 per-tool tests. Add invariants
there, not cases; a per-tool test has to be remembered, an enumeration does
not.

**Config in a file: not doing (D28).** An MCP stdio server is launched by its
client from a config file that already exists and is not ours. A second one
splits settings across two places, adds a precedence matrix, and forces every
error message to know which source a value came from (D13). The two real pains
it would have solved are solved without it: wildcards for `ENABLED_TOOLS`
(D27) and `*_FILE` for secrets (D28). Multi-instance stays the one open case.

**Dry run (landed, D26).** One router wrapper in `mcp-atlassian/src/dry_run.rs`,
same shape as the audit wrapper and keyed off the same `readOnlyHint`
annotation; no product crate changed. Order in `AtlassianServer::new` is prune
→ intercept → audit, so an intercepted call is still logged with
`dry_run: true`. The route's `outputSchema` is swapped for the report's, so
D20 still holds. Argument checking is intentionally shallow (required present,
undeclared warned) — deeper validation would duplicate every tool's schema.
Deliberately absent: faking IDs so a create→comment chain keeps working. The
second call in such a chain sees a real-looking key that does not exist, which
is fine for rehearsing a prompt and wrong for demoing a scenario.

**TTL cache (landed, D25).** The reverted first attempt came back with the
design the note prescribed: `atlassian-client/src/cache.rs`, `with_cache` on
both clients, off by default, filter arguments in the key, nothing cached that
a user edits. It also covers link types and field definitions, and
`search_fields` filters a cached full list client-side. Values are held as
`Arc<dyn Any>` rather than JSON, so a hit costs a downcast and a clone.
Deliberately absent: single-flight for concurrent misses, and any invalidation
on writes — a `create_project` through this server still waits out the TTL.

## Key files

- `HANDOFF-PLAN.md` — the 2026-09-02 full-codebase review and the phase 6–10
  sequence. Phases 6 (correctness), 7 (hardening + footprint) and 8
  (deployment correctness + size), 9 (MCP surface) and 10 (release) are
  done; what is left is the backlog below
- `DECISIONS.md` — 45 architecture decisions (D1–D45); read before structural
  changes
- `CLAUDE.md` — commands, layout, conventions, env vars, roadmap
- `crates/atlassian-{jira,confluence}/src/tools/` — tools, next to the
  client they call (D15/D21)
- `crates/atlassian-client/` — HTTP/auth/OAuth core
