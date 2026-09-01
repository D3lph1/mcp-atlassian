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
    tracing::info!(
        transport = %transport,
        jira = config.jira.is_some(),
        confluence = config.confluence.is_some(),
        read_only = config.read_only,
        dry_run = config.dry_run,
        tools = server.tool_names().len(),
        "starting mcp-atlassian"
    );

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
