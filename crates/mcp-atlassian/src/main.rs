use anyhow::Context;
use atlassian_client::Config;
use mcp_atlassian::server::AtlassianServer;
use rmcp::{transport::stdio, ServiceExt};
use tracing_subscriber::EnvFilter;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    // stdout carries the MCP protocol in stdio mode — all logging goes to stderr.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();

    let config = Config::from_env().context("failed to load configuration")?;
    let server = AtlassianServer::new(&config).context("failed to initialize clients")?;

    let transport = std::env::var("TRANSPORT").unwrap_or_else(|_| "stdio".into());
    let tools = server.tool_names();
    // One startup summary, in whichever form suits the reader: the banner for a
    // human watching a terminal or `docker logs`, the structured line for a log
    // collector. Both go to stderr — stdout is the protocol (D29).
    if banner_wanted() {
        mcp_atlassian::banner::print(&config, &transport, tools.len());
    } else {
        tracing::info!(
            transport = %transport,
            jira = config.jira.is_some(),
            confluence = config.confluence.is_some(),
            read_only = config.read_only,
            dry_run = config.dry_run,
            tools = tools.len(),
            "starting mcp-atlassian"
        );
    }
    log_registered_tools(&tools);

    match transport.as_str() {
        "stdio" => {
            let service = server.serve(stdio()).await?;
            service.waiting().await?;
        }
        "streamable-http" | "http" => {
            #[cfg(feature = "http")]
            http::serve(server).await?;
            #[cfg(not(feature = "http"))]
            anyhow::bail!(
                "this binary was built without the `http` feature; \
                 rebuild with `cargo build --features http` or use TRANSPORT=stdio"
            );
        }
        other => anyhow::bail!("unknown TRANSPORT `{other}`: use `stdio` or `streamable-http`"),
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

/// `NO_BANNER=true` swaps the banner for the structured startup line.
fn banner_wanted() -> bool {
    !std::env::var("NO_BANNER").is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes"
        )
    })
}

#[cfg(feature = "http")]
mod http {
    use anyhow::Context;
    use mcp_atlassian::server::AtlassianServer;
    use rmcp::transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    };
    use std::sync::Arc;

    /// Serves the MCP server over streamable HTTP at `http://{HOST}:{PORT}/mcp`.
    ///
    /// Env: `HOST` (default 127.0.0.1), `PORT` (default 8000), plus
    /// `ALLOWED_HOSTS` — extra comma-separated Host-header values for
    /// non-loopback deployments (DNS-rebinding protection allows only
    /// loopback by default).
    pub async fn serve(server: AtlassianServer) -> anyhow::Result<()> {
        let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let port: u16 = std::env::var("PORT")
            .unwrap_or_else(|_| "8000".into())
            .parse()
            .context("PORT must be a number")?;

        let mut config = StreamableHttpServerConfig::default();
        for extra in std::env::var("ALLOWED_HOSTS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            config.allowed_hosts.push(extra.to_string());
        }
        if host != "127.0.0.1" && host != "localhost" && host != "::1" {
            // Make the bind address itself pass Host validation.
            config.allowed_hosts.push(host.clone());
            config.allowed_hosts.push(format!("{host}:{port}"));
            tracing::warn!(
                %host,
                "binding to a non-loopback address; set ALLOWED_HOSTS to the public hostname(s) clients will use"
            );
        }

        let service = StreamableHttpService::new(
            move || Ok(server.clone()),
            Arc::new(LocalSessionManager::default()),
            config,
        );
        let router = axum::Router::new().nest_service("/mcp", service);
        let listener = tokio::net::TcpListener::bind((host.as_str(), port))
            .await
            .with_context(|| format!("failed to bind {host}:{port}"))?;
        tracing::info!("listening on http://{host}:{port}/mcp");
        axum::serve(listener, router).await?;
        Ok(())
    }
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
