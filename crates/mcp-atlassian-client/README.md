# mcp-atlassian-client

Shared HTTP layer behind the [`mcp-atlassian`](https://crates.io/crates/mcp-atlassian)
server: authentication, configuration from the environment, retries, error
mapping and an optional TTL cache for reference data.

Part of the `mcp-atlassian` workspace. It is published because the crates that
depend on it are, not as a general-purpose library — the API moves whenever the
server needs it to.

## What it does

- **Auth** — Basic (Cloud API token), Bearer PAT (Server/Data Center) and
  OAuth 2.0 with a rotating refresh token. `Debug` never prints a credential.
- **Config** — every environment variable the server reads is parsed in one
  place, including the `*_FILE` spelling that reads a secret from disk.
- **Requests** — one client over `reqwest`/rustls, bounded retries only for what
  is safe to replay, and Atlassian errors mapped to messages an LLM can act on.
- **Extras** — `TtlCache` (opt-in, reference data only), `ToolFilter` wildcards
  for `ENABLED_TOOLS`/`DISABLED_TOOLS`, and MCP result types behind the `mcp`
  feature.

## Example

```rust
use mcp_atlassian_client::{Auth, ServiceConfig};

let service = ServiceConfig {
    base_url: "https://company.atlassian.net".into(),
    auth: Auth::Basic {
        username: "you@example.com".into(),
        token: std::env::var("JIRA_API_TOKEN")?,
    },
    deployment: None, // inferred from the auth mode
};
```

## License

MIT.
