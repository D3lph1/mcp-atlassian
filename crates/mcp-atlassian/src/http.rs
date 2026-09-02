//! Streamable HTTP transport (D18): the MCP server at `/mcp`, behind an
//! optional bearer token (D39), with `/healthz` for probes and a graceful
//! stop on SIGTERM / Ctrl-C.
//!
//! Only built with the `http` cargo feature, so the default stdio binary
//! pays for none of it (D7).

use std::sync::Arc;

use anyhow::Context;
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};

use crate::server::AtlassianServer;

/// Serves the MCP server at `http://{host}:{port}/mcp` until the process is
/// asked to stop.
pub async fn serve(
    server: AtlassianServer,
    host: &str,
    port: u16,
    allowed_hosts: &[String],
    bearer_token: Option<String>,
) -> anyhow::Result<()> {
    let loopback = matches!(host, "127.0.0.1" | "localhost" | "::1");
    if !loopback && bearer_token.is_none() {
        tracing::warn!(
            %host,
            "binding to a non-loopback address without MCP_BEARER_TOKEN: anyone who can reach \
             this port can use the configured Atlassian credentials"
        );
    }
    let mut config = StreamableHttpServerConfig::default();
    config.allowed_hosts.extend(allowed_hosts.iter().cloned());
    if !loopback {
        // Make the bind address itself pass Host validation.
        config.allowed_hosts.push(host.to_string());
        config.allowed_hosts.push(format!("{host}:{port}"));
        if allowed_hosts.is_empty() {
            tracing::warn!(
                %host,
                "binding to a non-loopback address; set ALLOWED_HOSTS to the public hostname(s) \
                 clients will use"
            );
        }
    }

    let listener = tokio::net::TcpListener::bind((host, port))
        .await
        .with_context(|| format!("failed to bind {host}:{port}"))?;
    tracing::info!("listening on http://{host}:{port}/mcp");
    axum::serve(listener, router(server, config, bearer_token))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("stopped");
    Ok(())
}

/// The application: `/mcp` (authenticated when a token is set) and
/// `/healthz`. Separate from [`serve`] so a test can mount it on a listener
/// of its own.
pub fn router(
    server: AtlassianServer,
    config: StreamableHttpServerConfig,
    bearer_token: Option<String>,
) -> Router {
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let mcp = Router::new().nest_service("/mcp", service);
    let mcp = match bearer_token {
        Some(token) => mcp.layer(middleware::from_fn_with_state(
            Arc::new(token),
            require_bearer,
        )),
        None => mcp,
    };
    // Liveness only; it does not touch Atlassian, so it cannot flap on a
    // rate limit and cannot be used to probe credentials.
    mcp.route("/healthz", axum::routing::get(|| async { "ok" }))
}

/// Rejects a request whose `Authorization: Bearer …` does not match.
async fn require_bearer(
    State(expected): State<Arc<String>>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    match presented {
        Some(token) if constant_time_eq(token.as_bytes(), expected.as_bytes()) => {
            next.run(request).await
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            "missing or invalid bearer token",
        )
            .into_response(),
    }
}

/// Compares without short-circuiting on the first differing byte, so timing
/// does not leak how much of the token was right. Length is compared first;
/// it is not a secret.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Resolves on Ctrl-C, or SIGTERM where there is one (what `docker stop`
/// and Kubernetes send).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown requested");
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn tokens_compare_by_value() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"a"));
    }
}
