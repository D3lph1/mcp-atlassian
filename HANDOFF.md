# Handoff

Project status and the idea backlog. Update when finishing a phase.

## Current state (2026-09-01)

Phases 1–5 are done. A functionally complete MCP server for Jira + Confluence:

- **70 tools** (40 Jira + 30 Confluence) — full coverage of the surface
  comparable servers expose: issues, search, transitions, comments, worklog,
  links/epics, fields, agile, watchers, batch create, changelog; Confluence
  pages incl. section edits and moves, inline comments, attachments, version
  history and diff, templates, restrictions
- Auth: API token (Cloud), PAT (Server/DC), OAuth 2.0 refresh flow (D17)
- Transports: stdio (default), streamable HTTP behind the `http` feature (D18)
- Filtering: `ENABLED_TOOLS`; `READ_ONLY_MODE` driven by tool annotations
  (40 read-only / 30 write, 14 of them destructive) — D22
- Audit log: `AUDIT_LOG_FILE` appends one JSONL record per write call —
  timestamp, tool, arguments, outcome, duration, `destructive` flag (D23)
- Markdown ↔ Confluence storage (htmd/comrak, D10)
- Structured output: every tool advertises an `outputSchema` and returns
  `structuredContent` (D20)
- Tests: 74 (wiremock + end-to-end over an in-memory transport), clippy clean; CI (fmt/clippy/test, musl artifact,
  docker); scratch Dockerfile
- Binary: 3.6 MB stdio / 3.9 MB with http; idle RSS ~2 MB (target < 30 MB —
  comfortably under). The audit log added ~16 KB (chrono was already in the
  tree via rmcp)

Deliberately not done: SSE transport (deprecated in MCP), multi-user proxy.

Known loose ends:
- [ ] The Docker image has not been built locally (daemon / credential helper
      on this machine); the CI `docker` job should verify it
- [ ] No README.md for publication yet
- [ ] git is not initialized, no commits

## Feature backlog (prioritized)

### Top (value / effort)
- [ ] **TTL cache** (in-memory, ~5 min) for reference data: projects, issue
      types, spaces, boards. Less latency and rate-limit pressure.
      Was started once and reverted — see the note below.
- [ ] **`DRY_RUN=true`** — write tools return a description of the action
      without performing it. A safety net for demos and prompt debugging.

### MCP protocol features (rmcp supports them)
- [ ] **Resources** — `jira://PROJ-123`, `confluence://12345` as MCP resources
      (resource templates)
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

**TTL cache (reverted).** An implementation existed briefly:
`atlassian-client/src/cache.rs` with a type-erased `TtlCache` (JSON values,
`Mutex<HashMap>`), plus `JiraClient::with_cache` / `ConfluenceClient::with_cache`
wrapping only reference-data methods (`get_myself`, `get_projects`,
`get_issue_types`, `get_boards`, `get_spaces`). It was rolled back before the
config wiring landed. The design worth keeping if it comes back: disabled by
default (a cache changes observable behavior — a project created out of band
stays invisible for up to one TTL), never cache issues/searches/comments/
sprints, and include filter arguments in the cache key.

## Key files

- `DECISIONS.md` — 23 architecture decisions (D1–D23); read before structural
  changes
- `CLAUDE.md` — commands, layout, conventions, env vars, roadmap
- `crates/atlassian-{jira,confluence}/src/tools/` — tools, next to the
  client they call (D15/D21)
- `crates/atlassian-client/` — HTTP/auth/OAuth core
