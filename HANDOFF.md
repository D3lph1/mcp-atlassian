# Handoff

Project status and the idea backlog. Update when finishing a phase.

## Current state (2026-09-02)

Phases 1–5 are done. A functionally complete MCP server for Jira + Confluence:

- **70 tools** (40 Jira + 30 Confluence) — full coverage of the surface
  comparable servers expose: issues, search, transitions, comments, worklog,
  links/epics, fields, agile, watchers, batch create, changelog; Confluence
  pages incl. section edits and moves, inline comments, attachments, version
  history and diff, templates, restrictions
- Auth: API token (Cloud), PAT (Server/DC), OAuth 2.0 refresh flow (D17)
- Transports: stdio (default), streamable HTTP behind the `http` feature (D18)
- Filtering: `ENABLED_TOOLS`; `READ_ONLY` driven by tool annotations
  (40 read-only / 30 write, 14 of them destructive) — D22
- `DRY_RUN=true`: write tools stay listed, are validated against their own
  input schema and described instead of performed; reads still execute (D26)
- Audit log: `AUDIT_LOG_FILE` appends one JSONL record per write call —
  timestamp, tool, arguments, outcome, duration, `destructive` and `dry_run`
  flags (D23)
- Markdown ↔ Confluence storage (htmd/comrak, D10)
- Structured output: every tool advertises an `outputSchema` and returns
  `structuredContent` (D20)
- Resources: `jira://PROJ-123` (JSON) and `confluence://123456` (Markdown) as
  resource templates; `resources/list` intentionally empty (D24)
- TTL cache: `CACHE_TTL` seconds, reference data only (projects, issue types,
  boards, link types, fields, spaces), off by default (D25)
- Tests: 119 (wiremock + end-to-end over an in-memory transport), clippy clean; CI (fmt/clippy/test, musl artifact,
  docker); scratch Dockerfile
- Binary: 3.6 MB stdio / 3.9 MB with http; idle RSS ~2 MB (target < 30 MB —
  comfortably under). The audit log added ~16 KB (chrono was already in the
  tree via rmcp), resources ~15 KB, the TTL cache ~16 KB, dry run 112 bytes —
  no new dependencies

Deliberately not done: SSE transport (deprecated in MCP), multi-user proxy.

Known loose ends:
- [ ] The Docker image has not been built locally (daemon / credential helper
      on this machine); the CI `docker` job should verify it
- [ ] No README.md for publication yet

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
- [ ] **Prompts** — ready-made: sprint standup, bug triage, release notes from
      JQL. A differentiator against other implementations.
- [ ] **Elicitation** — confirmation for destructive tools through the client.
      `destructiveHint` is already set on all 14 of them (D22), so this is now
      just wiring the elicitation round-trip.

### Security / operations
- [x] Audit log of write operations (JSONL to a file) — `AUDIT_LOG_FILE`, D23
- [ ] Multi-instance (two Jiras) — needs a TOML config instead of env vars,
      plus tool prefixes

### Wider coverage
- [ ] Jira: `move_issue` across projects (Cloud async task API — needs task
      polling, deliberately skipped for now)
- [ ] Confluence: page view analytics (Cloud-only analytics API)
- [ ] JSM (Service Management): requests, queues
- [ ] Bitbucket: a new `atlassian-bitbucket` crate (PRs, diffs) — the
      workspace is ready for it

### Distribution
- [ ] Release binaries on GitHub: musl x86_64/aarch64 + macOS arm64
- [ ] Publish to crates.io (`cargo install mcp-atlassian`)
- [ ] Homebrew formula

## Notes

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

- `DECISIONS.md` — 26 architecture decisions (D1–D26); read before structural
  changes
- `CLAUDE.md` — commands, layout, conventions, env vars, roadmap
- `crates/atlassian-{jira,confluence}/src/tools/` — tools, next to the
  client they call (D15/D21)
- `crates/atlassian-client/` — HTTP/auth/OAuth core
