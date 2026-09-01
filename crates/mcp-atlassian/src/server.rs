use std::sync::Arc;

use crate::router_ext::project_router;
use atlassian_client::Config;
use atlassian_confluence::{ConfluenceClient, ConfluenceTools};
use atlassian_jira::{JiraClient, JiraTools};
use rmcp::{
    handler::server::router::tool::ToolRouter,
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool_handler, ServerHandler,
};

/// The MCP server: holds product clients and routes tool calls.
///
/// This is the only crate that knows about MCP — product crates stay
/// protocol-free (see CLAUDE.md dependency rule). Tool implementations live
/// in `crate::tools` (one module per product); this file owns construction,
/// route filtering and the `ServerHandler` impl.
#[derive(Clone)]
pub struct AtlassianServer {
    jira: Option<JiraTools>,
    confluence: Option<ConfluenceTools>,
    /// Filtered per configuration: unconfigured services, read-only mode and
    /// the ENABLED_TOOLS allowlist all prune routes at startup.
    tool_router: ToolRouter<Self>,
}

impl AtlassianServer {
    pub fn new(config: &Config) -> atlassian_client::Result<Self> {
        let jira = config
            .jira
            .as_ref()
            .map(JiraClient::new)
            .transpose()?
            .map(|client| JiraTools::new(Arc::new(client)));
        let confluence = config
            .confluence
            .as_ref()
            .map(ConfluenceClient::new)
            .transpose()?
            .map(|client| ConfluenceTools::new(Arc::new(client)));

        // Product routers are defined over each product's own state; project
        // them onto this server (see `router_ext`). Projection is infallible
        // because a route only runs for a configured service — routes of an
        // unconfigured one are pruned right below.
        let mut tool_router =
            project_router(atlassian_jira::tools::router(), |s: &Self| {
                s.jira
                    .as_ref()
                    .expect("jira tools pruned when unconfigured")
            }) + project_router(atlassian_confluence::tools::router(), |s: &Self| {
                s.confluence
                    .as_ref()
                    .expect("confluence tools pruned when unconfigured")
            });
        // A tool counts as read-only only if it says so through its MCP
        // annotation. Anything unannotated is treated as a write — a new tool
        // cannot silently slip into READ_ONLY_MODE by omission.
        let read_only_tools: std::collections::HashSet<String> = tool_router
            .list_all()
            .into_iter()
            .filter(|t| {
                t.annotations
                    .as_ref()
                    .and_then(|a| a.read_only_hint)
                    .unwrap_or(false)
            })
            .map(|t| t.name.to_string())
            .collect();
        let all_names: Vec<String> = tool_router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        for name in &all_names {
            let unconfigured_service = (jira.is_none() && name.starts_with("jira_"))
                || (confluence.is_none() && name.starts_with("confluence_"));
            let read_only_write = config.read_only && !read_only_tools.contains(name);
            let not_allowlisted = config
                .enabled_tools
                .as_ref()
                .is_some_and(|allow| !allow.contains(name));
            if unconfigured_service || read_only_write || not_allowlisted {
                tool_router.remove_route(name);
            }
        }
        if let Some(allow) = &config.enabled_tools {
            for name in allow {
                if !all_names.iter().any(|n| n == name) {
                    tracing::warn!(tool = %name, "ENABLED_TOOLS names an unknown tool");
                }
            }
        }

        Ok(Self {
            jira,
            confluence,
            tool_router,
        })
    }

    /// The tools actually registered after filtering, with their schemas.
    pub fn tools(&self) -> Vec<rmcp::model::Tool> {
        self.tool_router.list_all()
    }

    /// Names of the tools actually registered after filtering.
    pub fn tool_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .tool_router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        names.sort();
        names
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AtlassianServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "mcp-atlassian",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Tools for Jira and Confluence. Prefer JQL/CQL search tools with small \
                 limits (10 or fewer results) before fetching individual entities.",
            )
    }
}
