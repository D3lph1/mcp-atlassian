//! The command line, declared with clap.
//!
//! Configuration stays in the environment (D8): MCP clients launch the server
//! from a JSON config that carries settings as `env`, so flags would be a
//! second way to say the same thing. What the flags cover is everything a
//! person does *before* configuring — check the version, read the help, see
//! the tool catalogue, install completions — and all of it runs before
//! `Config::from_env` is called, so it works on a machine with no token.

use clap::{CommandFactory, Parser, ValueEnum};
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
        Speaks MCP on stdin/stdout by default; TRANSPORT=streamable-http serves \
        HTTP instead. Run with no arguments to start the server.",
    after_help = AFTER_HELP,
    // The env vars in after_help are the reference; clap's own `[env: ...]`
    // annotations would put a second, partial copy next to the flags.
    disable_help_subcommand = true,
    arg_required_else_help = false
)]
pub struct Cli {
    /// Print every tool this build offers and exit.
    ///
    /// Needs no configuration and ignores READ_ONLY, ENABLED_TOOLS and
    /// DISABLED_TOOLS: this is what the build has, not what a configuration
    /// would register.
    #[arg(long, short = 'l')]
    pub list_tools: bool,

    /// Output format for --list-tools.
    #[arg(long, value_enum, default_value_t = Format::Text, requires = "list_tools")]
    pub format: Format,

    /// Print a shell completion script and exit.
    ///
    /// Load it with, for example, `eval "$(mcp-atlassian --completions zsh)"`,
    /// or write it where your shell looks for completions.
    #[arg(long, value_name = "SHELL", value_enum, conflicts_with = "list_tools")]
    pub completions: Option<Shell>,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Grouped by product, one line per tool.
    Text,
    /// One object per tool: name, kind, title, description.
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
