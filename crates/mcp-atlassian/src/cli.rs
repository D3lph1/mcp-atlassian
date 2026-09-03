//! The command line, declared with clap.
//!
//! Configuration stays in the environment (D8): MCP clients launch the server
//! from a JSON config that carries settings as `env`, so flags would be a
//! second way to say the same thing. The command line therefore only names
//! *what to do* — serve, list the tools, print a completion script — and all
//! of it is parsed before `Config::from_env` is called, so every command
//! except `serve` works on a machine with no token.
//!
//! Actions are subcommands, not flags: a flag modifies a run, a subcommand
//! replaces it, and `tools --format json` scopes the option to the one
//! command it belongs to instead of guarding it with `requires`.

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

/// The footer of `--help`. A leading `\` keeps the literal from starting with
/// a newline; the line breaks after that are real, so what is written here is
/// what is printed.
const AFTER_HELP: &str = "\
Every setting has a flag on `serve` and an environment variable; run
`mcp-atlassian serve --help` for the list. Secrets are environment-only.

Full reference: https://github.com/d3lph1/mcp-atlassian";

#[derive(Parser, Debug)]
#[command(
    name = "mcp-atlassian",
    version,
    about = "MCP server for Jira and Confluence — Cloud and Server/Data Center",
    long_about = "MCP server for Jira and Confluence — Cloud and Server/Data Center.\n\n\
        With no command this starts the server, speaking MCP on stdin/stdout \
        (TRANSPORT=streamable-http serves HTTP instead). That is what an MCP \
        client's configuration runs.",
    after_help = AFTER_HELP
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start the MCP server (the default when no command is given).
    Serve(Box<ServeArgs>),

    /// Print every tool this build offers.
    ///
    /// Needs no configuration and ignores READ_ONLY, ENABLED_TOOLS and
    /// DISABLED_TOOLS: this is what the build has, not what a configuration
    /// would register — the startup log reports that.
    Tools {
        /// Output format.
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },

    /// Print a shell completion script.
    ///
    /// Load it with, for example, `eval "$(mcp-atlassian completions zsh)"`,
    /// or write it where your shell looks for completions. Homebrew installs
    /// it for you.
    Completions {
        /// The shell to generate for.
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Grouped by product, one line per tool.
    Text,
    /// One object per tool: name, product, kind, title, description.
    Json,
}

impl Cli {
    /// The completion script for `shell`, written by clap from the same
    /// declarations that produce `--help` — so the two cannot disagree.
    pub fn completion_script(shell: Shell) -> String {
        let mut command = Self::command();
        let mut out = Vec::new();
        clap_complete::generate(shell, &mut command, "mcp-atlassian", &mut out);
        String::from_utf8(out).expect("clap writes UTF-8")
    }
}

/// Settings for `serve`, each mirroring one environment variable.
///
/// Flags win over the environment, and a flag left out changes nothing — the
/// variable still applies. Nothing here parses or validates: the values are
/// layered over the environment and read by `Config::read`, which stays the
/// one place that understands them (D8). That is why every field is a plain
/// `Option<String>` rather than a typed value.
///
/// **No secret is a flag.** Arguments are visible to every process on the
/// machine through `ps` and land in shell history, so tokens come from the
/// environment or from a file a `*-token-file` flag points at (D28).
#[derive(clap::Args, Debug, Default)]
#[command(next_help_heading = "Jira")]
pub struct ServeArgs {
    /// Jira base URL [env: JIRA_URL]
    #[arg(long, value_name = "URL")]
    pub jira_url: Option<String>,

    /// Jira account email, for API-token auth [env: JIRA_USERNAME]
    #[arg(long, value_name = "EMAIL")]
    pub jira_username: Option<String>,

    /// File holding the Jira API token [env: JIRA_API_TOKEN_FILE]
    #[arg(long, value_name = "PATH")]
    pub jira_api_token_file: Option<String>,

    /// File holding the Jira personal access token [env: JIRA_PERSONAL_TOKEN_FILE]
    #[arg(long, value_name = "PATH")]
    pub jira_personal_token_file: Option<String>,

    /// Override the deployment inferred from the auth mode: cloud or server
    /// [env: JIRA_DEPLOYMENT]
    #[arg(long, value_name = "KIND")]
    pub jira_deployment: Option<String>,

    /// Confluence base URL [env: CONFLUENCE_URL]
    #[arg(long, value_name = "URL", help_heading = "Confluence")]
    pub confluence_url: Option<String>,

    /// Confluence account email [env: CONFLUENCE_USERNAME]
    #[arg(long, value_name = "EMAIL", help_heading = "Confluence")]
    pub confluence_username: Option<String>,

    /// File holding the Confluence API token [env: CONFLUENCE_API_TOKEN_FILE]
    #[arg(long, value_name = "PATH", help_heading = "Confluence")]
    pub confluence_api_token_file: Option<String>,

    /// File holding the Confluence personal access token
    /// [env: CONFLUENCE_PERSONAL_TOKEN_FILE]
    #[arg(long, value_name = "PATH", help_heading = "Confluence")]
    pub confluence_personal_token_file: Option<String>,

    /// Override the Confluence deployment: cloud or server
    /// [env: CONFLUENCE_DEPLOYMENT]
    #[arg(long, value_name = "KIND", help_heading = "Confluence")]
    pub confluence_deployment: Option<String>,

    /// OAuth 2.0 client id; configures both services [env: ATLASSIAN_OAUTH_CLIENT_ID]
    #[arg(long, value_name = "ID", help_heading = "OAuth 2.0 (Cloud)")]
    pub oauth_client_id: Option<String>,

    /// OAuth 2.0 cloud id [env: ATLASSIAN_OAUTH_CLOUD_ID]
    #[arg(long, value_name = "ID", help_heading = "OAuth 2.0 (Cloud)")]
    pub oauth_cloud_id: Option<String>,

    /// File holding the OAuth client secret
    /// [env: ATLASSIAN_OAUTH_CLIENT_SECRET_FILE]
    #[arg(long, value_name = "PATH", help_heading = "OAuth 2.0 (Cloud)")]
    pub oauth_client_secret_file: Option<String>,

    /// File holding the OAuth refresh token
    /// [env: ATLASSIAN_OAUTH_REFRESH_TOKEN_FILE]
    #[arg(long, value_name = "PATH", help_heading = "OAuth 2.0 (Cloud)")]
    pub oauth_refresh_token_file: Option<String>,

    /// Register only tools annotated read-only [env: READ_ONLY]
    #[arg(long, help_heading = "Safety")]
    pub read_only: bool,

    /// Describe writes instead of performing them [env: DRY_RUN]
    #[arg(long, help_heading = "Safety")]
    pub dry_run: bool,

    /// Ask through MCP elicitation before a destructive tool runs
    /// [env: CONFIRM_DESTRUCTIVE]
    #[arg(long, help_heading = "Safety")]
    pub confirm_destructive: bool,

    /// Allowlist of tool-name patterns, comma-separated (jira_*, *_get_*)
    /// [env: ENABLED_TOOLS]
    #[arg(long, value_name = "PATTERNS", help_heading = "Safety")]
    pub enabled_tools: Option<String>,

    /// Denylist subtracted from the allowlist; deny wins [env: DISABLED_TOOLS]
    #[arg(long, value_name = "PATTERNS", help_heading = "Safety")]
    pub disabled_tools: Option<String>,

    /// Append one JSONL record per write call to this file [env: AUDIT_LOG_FILE]
    #[arg(long, value_name = "PATH", help_heading = "Safety")]
    pub audit_log: Option<String>,

    /// The only directory attachment tools may read from and write to
    /// [env: ATTACHMENT_DIR]
    #[arg(long, value_name = "DIR", help_heading = "Safety")]
    pub attachment_dir: Option<String>,

    /// Cap on one attachment either direction; 0 removes the limit
    /// [env: MAX_ATTACHMENT_BYTES]
    #[arg(long, value_name = "BYTES", help_heading = "Safety")]
    pub max_attachment_bytes: Option<String>,

    /// Seconds to cache reference data; 0 or unset disables caching
    /// [env: CACHE_TTL]
    #[arg(long, value_name = "SECONDS", help_heading = "Behaviour")]
    pub cache_ttl: Option<String>,

    /// Seconds per Atlassian request [env: REQUEST_TIMEOUT]
    #[arg(long, value_name = "SECONDS", help_heading = "Behaviour")]
    pub request_timeout: Option<String>,

    /// tracing directives, e.g. debug or mcp_atlassian_client=debug,info
    /// [env: LOG_FILTER]
    #[arg(long, value_name = "DIRECTIVES", help_heading = "Behaviour")]
    pub log_filter: Option<String>,

    /// Print the structured startup line instead of the banner [env: NO_BANNER]
    #[arg(long, help_heading = "Behaviour")]
    pub no_banner: bool,

    /// stdio (default) or streamable-http [env: TRANSPORT]
    #[arg(long, value_name = "KIND", help_heading = "HTTP transport")]
    pub transport: Option<String>,

    /// Bind address for the HTTP transport [env: HOST]
    #[arg(long, value_name = "HOST", help_heading = "HTTP transport")]
    pub host: Option<String>,

    /// Port for the HTTP transport [env: PORT]
    #[arg(long, value_name = "PORT", help_heading = "HTTP transport")]
    pub port: Option<String>,

    /// Extra Host-header values to accept, comma-separated [env: ALLOWED_HOSTS]
    #[arg(long, value_name = "HOSTS", help_heading = "HTTP transport")]
    pub allowed_hosts: Option<String>,

    /// File holding the bearer token every /mcp request must carry
    /// [env: MCP_BEARER_TOKEN_FILE]
    #[arg(long, value_name = "PATH", help_heading = "HTTP transport")]
    pub mcp_bearer_token_file: Option<String>,
}

impl ServeArgs {
    /// The variables these flags stand in for, in the spelling `Config::read`
    /// expects. Only what was actually given appears, so an absent flag leaves
    /// the environment alone.
    pub fn overrides(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        let mut set = |name: &'static str, value: &Option<String>| {
            if let Some(value) = value {
                out.push((name, value.clone()));
            }
        };
        set("JIRA_URL", &self.jira_url);
        set("JIRA_USERNAME", &self.jira_username);
        set("JIRA_API_TOKEN_FILE", &self.jira_api_token_file);
        set("JIRA_PERSONAL_TOKEN_FILE", &self.jira_personal_token_file);
        set("JIRA_DEPLOYMENT", &self.jira_deployment);
        set("CONFLUENCE_URL", &self.confluence_url);
        set("CONFLUENCE_USERNAME", &self.confluence_username);
        set("CONFLUENCE_API_TOKEN_FILE", &self.confluence_api_token_file);
        set(
            "CONFLUENCE_PERSONAL_TOKEN_FILE",
            &self.confluence_personal_token_file,
        );
        set("CONFLUENCE_DEPLOYMENT", &self.confluence_deployment);
        set("ATLASSIAN_OAUTH_CLIENT_ID", &self.oauth_client_id);
        set("ATLASSIAN_OAUTH_CLOUD_ID", &self.oauth_cloud_id);
        set(
            "ATLASSIAN_OAUTH_CLIENT_SECRET_FILE",
            &self.oauth_client_secret_file,
        );
        set(
            "ATLASSIAN_OAUTH_REFRESH_TOKEN_FILE",
            &self.oauth_refresh_token_file,
        );
        set("ENABLED_TOOLS", &self.enabled_tools);
        set("DISABLED_TOOLS", &self.disabled_tools);
        set("AUDIT_LOG_FILE", &self.audit_log);
        set("ATTACHMENT_DIR", &self.attachment_dir);
        set("MAX_ATTACHMENT_BYTES", &self.max_attachment_bytes);
        set("CACHE_TTL", &self.cache_ttl);
        set("REQUEST_TIMEOUT", &self.request_timeout);
        set("LOG_FILTER", &self.log_filter);
        set("TRANSPORT", &self.transport);
        set("HOST", &self.host);
        set("PORT", &self.port);
        set("ALLOWED_HOSTS", &self.allowed_hosts);
        set("MCP_BEARER_TOKEN_FILE", &self.mcp_bearer_token_file);
        // A switch left off says nothing: the variable still decides, and it
        // understands spellings beyond `true` that clap would not.
        for (name, on) in [
            ("READ_ONLY", self.read_only),
            ("DRY_RUN", self.dry_run),
            ("CONFIRM_DESTRUCTIVE", self.confirm_destructive),
            ("NO_BANNER", self.no_banner),
        ] {
            if on {
                out.push((name, "true".to_string()));
            }
        }
        out
    }

    /// The process environment with these flags laid over it.
    pub fn environment(&self) -> Overrides {
        Overrides {
            values: self.overrides().into_iter().collect(),
        }
    }
}

/// `Env` that answers from the flags first and the process environment
/// second — which is what "a flag wins over a variable" means.
pub struct Overrides {
    values: std::collections::HashMap<&'static str, String>,
}

impl mcp_atlassian_client::Env for Overrides {
    fn get(&self, name: &str) -> Option<String> {
        self.values
            .get(name)
            .cloned()
            .or_else(|| std::env::var(name).ok())
    }
}
