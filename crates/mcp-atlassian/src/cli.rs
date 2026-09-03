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

/// The environment variables, listed in `--help` so the one document a user
/// reaches for from the terminal is not a dead end.
const AFTER_HELP: &str = "\
Configuration is read from the environment, never from flags:

  JIRA_URL, JIRA_USERNAME, JIRA_API_TOKEN      Jira Cloud (Basic auth)
  JIRA_PERSONAL_TOKEN                          Jira Server / Data Center (PAT)
  CONFLUENCE_URL, CONFLUENCE_USERNAME, ...     the same scheme for Confluence
  ATLASSIAN_OAUTH_CLIENT_ID, _CLIENT_SECRET,   OAuth 2.0 (Cloud); configures
    _REFRESH_TOKEN, _CLOUD_ID                    both services at once
  <ANY_TOKEN>_FILE                             read the secret from a file

  READ_ONLY=true            register only read-only tools
  DRY_RUN=true              describe writes instead of performing them
  CONFIRM_DESTRUCTIVE=true  ask before destructive tools run
  ENABLED_TOOLS, DISABLED_TOOLS   wildcard allow/deny lists (jira_*, *_delete_*)
  AUDIT_LOG_FILE            JSONL log of every write call
  ATTACHMENT_DIR            the only directory attachment tools may touch
  CACHE_TTL                 seconds to cache reference data (off by default)
  TRANSPORT                 stdio (default) or streamable-http
  HOST, PORT, MCP_BEARER_TOKEN    HTTP transport bind address and auth
  RUST_LOG                  tracing directives (info by default)
  NO_BANNER, NO_COLOR       startup output

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
    Serve,

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
