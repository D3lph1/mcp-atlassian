use anyhow::Context;
use clap::Parser;
use mcp_atlassian::cli::{Cli, Command, Format};
use mcp_atlassian::server::AtlassianServer;
use mcp_atlassian_client::{Config, Transport};
use rmcp::{transport::stdio, ServiceExt};
use tracing_subscriber::{filter::Targets, prelude::*};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    // Parsed before the configuration is read, so every command except
    // `serve` — and --help, --version — works on a machine with no token.
    match Cli::parse().command {
        Some(Command::Completions { shell }) => {
            print!("{}", Cli::completion_script(shell));
            return Ok(());
        }
        Some(Command::Tools { format }) => {
            match format {
                Format::Text => print!("{}", mcp_atlassian::catalogue::render()),
                Format::Json => println!("{}", mcp_atlassian::catalogue::render_json()),
            }
            return Ok(());
        }
        // No command is `serve`: that is what an MCP client's config runs.
        None | Some(Command::Serve) => {}
    }

    let config = Config::from_env().context("failed to load configuration")?;
    // `Targets` reads the same `crate=level` directives as `EnvFilter` and
    // needs no regex, which was ~130 KB of the binary (D41).
    let filter: Targets = config
        .log_filter
        .parse()
        .with_context(|| format!("RUST_LOG `{}` is not a valid filter", config.log_filter))?;
    // stdout carries the MCP protocol in stdio mode — all logging goes to stderr.
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(filter)
        .init();

    let server = AtlassianServer::new(&config).context("failed to initialize clients")?;

    let tools = server.tool_names();
    // One startup summary, in whichever form suits the reader: the banner for a
    // human watching a terminal or `docker logs`, the structured line for a log
    // collector. Both go to stderr — stdout is the protocol (D29).
    if config.banner {
        mcp_atlassian::banner::print(&config, config.transport.name(), tools.len());
    } else {
        tracing::info!(
            transport = config.transport.name(),
            jira = config.jira.is_some(),
            confluence = config.confluence.is_some(),
            read_only = config.read_only,
            dry_run = config.dry_run,
            confirm_destructive = config.confirm_destructive,
            tools = tools.len(),
            "starting mcp-atlassian"
        );
    }
    log_registered_tools(&tools);
    if let Some(path) = &config.audit_log {
        tracing::info!(path = %path.display(), "auditing write operations");
    }
    // Last, so a warning is the final thing on the screen rather than the
    // first — above the banner it reads as a failure to start (D29).
    for warning in server.startup_warnings() {
        tracing::warn!("{warning}");
    }

    match &config.transport {
        Transport::Stdio => {
            let service = server.serve(stdio()).await?;
            service.waiting().await?;
        }
        #[cfg(feature = "http")]
        Transport::StreamableHttp {
            host,
            port,
            allowed_hosts,
            bearer_token,
        } => {
            mcp_atlassian::http::serve(server, host, *port, allowed_hosts, bearer_token.clone())
                .await?;
        }
        #[cfg(not(feature = "http"))]
        Transport::StreamableHttp { .. } => anyhow::bail!(
            "this binary was built without the `http` feature; \
             rebuild with `cargo build --features http` or use TRANSPORT=stdio"
        ),
    }
    Ok(())
}

/// Names what survived filtering, so a narrowed `ENABLED_TOOLS` /
/// `DISABLED_TOOLS` / `READ_ONLY` can be checked against the log instead of by
/// calling `tools/list` through a client (D29).
///
/// One record per product rather than per tool: 70 lines would bury the rest of
/// the startup output, and the interesting question is almost always "is this
/// one there", which `grep` answers either way.
fn log_registered_tools(tools: &[String]) {
    for (product, names) in group_by_product(tools) {
        tracing::info!(
            count = names.len(),
            tools = %names.join(", "),
            "{product} tools registered"
        );
    }
}

/// Groups tool names by their product prefix, in a stable order. A name with no
/// known prefix lands in `other` rather than vanishing from the log.
fn group_by_product(tools: &[String]) -> Vec<(&'static str, Vec<&str>)> {
    let mut groups: Vec<(&'static str, Vec<&str>)> = vec![
        ("jira", Vec::new()),
        ("confluence", Vec::new()),
        ("other", Vec::new()),
    ];
    for name in tools {
        let group = match name.split_once('_').map(|(prefix, _)| prefix) {
            Some("jira") => 0,
            Some("confluence") => 1,
            _ => 2,
        };
        groups[group].1.push(name);
    }
    groups.retain(|(_, names)| !names.is_empty());
    groups
}

#[cfg(test)]
mod tests {
    use super::group_by_product;

    fn names(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn tools_are_grouped_by_product_in_a_stable_order() {
        let tools = names(&["confluence_search", "jira_search", "jira_get_issue"]);
        assert_eq!(
            group_by_product(&tools),
            vec![
                ("jira", vec!["jira_search", "jira_get_issue"]),
                ("confluence", vec!["confluence_search"]),
            ]
        );
    }

    #[test]
    fn an_empty_product_is_not_reported() {
        let tools = names(&["jira_search"]);
        assert_eq!(
            group_by_product(&tools),
            vec![("jira", vec!["jira_search"])]
        );
        assert!(group_by_product(&[]).is_empty());
    }

    #[test]
    fn a_tool_without_a_known_prefix_is_still_listed() {
        // A future product must not disappear from the log because this
        // function has not heard of it yet.
        let tools = names(&["bitbucket_get_pr", "jira_search"]);
        assert_eq!(
            group_by_product(&tools),
            vec![
                ("jira", vec!["jira_search"]),
                ("other", vec!["bitbucket_get_pr"]),
            ]
        );
    }
}
