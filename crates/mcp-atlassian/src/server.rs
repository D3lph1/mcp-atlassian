use std::sync::Arc;

use crate::audit::{instrument_writes, AuditLog};
use crate::router_ext::project_router;
use atlassian_client::Config;
use atlassian_confluence::{ConfluenceClient, ConfluenceTools};
use atlassian_jira::{JiraClient, JiraTools};
use rmcp::{
    handler::server::router::tool::ToolRouter,
    model::{
        Implementation, ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
        ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, ResourceTemplate,
        ServerCapabilities, ServerInfo,
    },
    service::RequestContext,
    tool_handler, ErrorData as McpError, RoleServer, ServerHandler,
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
    /// Products serving resources: a product qualifies when its service is
    /// configured *and* at least one of its tools survived filtering, so
    /// `jira://` cannot be a way around an allowlist that removed Jira (D24).
    jira_resources: bool,
    confluence_resources: bool,
}

impl AtlassianServer {
    pub fn new(config: &Config) -> atlassian_client::Result<Self> {
        // Reference data is cached only when a TTL is configured (D25).
        let jira = config
            .jira
            .as_ref()
            .map(JiraClient::new)
            .transpose()?
            .map(|client| match config.cache_ttl {
                Some(ttl) => client.with_cache(ttl),
                None => client,
            })
            .map(|client| JiraTools::new(Arc::new(client)));
        let confluence = config
            .confluence
            .as_ref()
            .map(ConfluenceClient::new)
            .transpose()?
            .map(|client| match config.cache_ttl {
                Some(ttl) => client.with_cache(ttl),
                None => client,
            })
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

        // Auditing wraps the surviving write routes, so a tool pruned above is
        // never logged — it cannot be called in the first place (D23).
        let tool_router = match &config.audit_log {
            Some(path) => {
                let log = AuditLog::open(path).map_err(|e| {
                    atlassian_client::Error::Config(format!(
                        "failed to open the audit log `{}`: {e}",
                        path.display()
                    ))
                })?;
                tracing::info!(path = %path.display(), "auditing write operations");
                instrument_writes(tool_router, log)
            }
            None => tool_router,
        };

        // Resources follow the tool surface, and are decided after filtering.
        let registered: Vec<String> = tool_router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        let jira_resources = jira.is_some() && registered.iter().any(|n| n.starts_with("jira_"));
        let confluence_resources =
            confluence.is_some() && registered.iter().any(|n| n.starts_with("confluence_"));

        Ok(Self {
            jira,
            confluence,
            tool_router,
            jira_resources,
            confluence_resources,
        })
    }

    /// The resource templates this server advertises, in a stable order.
    pub fn resource_templates(&self) -> Vec<ResourceTemplate> {
        let mut templates = Vec::new();
        if self.jira_resources {
            templates.extend(atlassian_jira::resources::templates());
        }
        if self.confluence_resources {
            templates.extend(atlassian_confluence::resources::templates());
        }
        templates
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
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new(
            "mcp-atlassian",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(
            "Tools for Jira and Confluence. Prefer JQL/CQL search tools with small \
             limits (10 or fewer results) before fetching individual entities. \
             Entities can also be attached as resources: `jira://PROJ-123`, \
             `confluence://123456`.",
        )
    }

    /// Always empty: the resources are issues and pages, and enumerating them
    /// is unbounded. Discovery goes through the search tools, which is what
    /// `resources/templates/list` says by naming the URI shapes (D24).
    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult::default())
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult::with_all_items(
            self.resource_templates(),
        ))
    }

    /// Dispatches by URI scheme; each product parses the rest of the URI
    /// itself. Resources are reads, so `READ_ONLY_MODE` does not touch them.
    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let uri = request.uri.as_str();
        let contents = if uri.starts_with(atlassian_jira::resources::URI_PREFIX) {
            self.jira
                .as_ref()
                .filter(|_| self.jira_resources)
                .ok_or_else(|| unavailable("Jira", "JIRA_URL", self.jira.is_some()))?
                .read_resource(uri)
                .await?
        } else if uri.starts_with(atlassian_confluence::resources::URI_PREFIX) {
            self.confluence
                .as_ref()
                .filter(|_| self.confluence_resources)
                .ok_or_else(|| {
                    unavailable("Confluence", "CONFLUENCE_URL", self.confluence.is_some())
                })?
                .read_resource(uri)
                .await?
        } else {
            return Err(McpError::invalid_params(
                format!(
                    "unknown resource URI `{uri}`: this server serves `jira://ISSUE-KEY` \
                     and `confluence://PAGE_ID`"
                ),
                None,
            ));
        };
        Ok(ReadResourceResult::new(contents).into())
    }
}

/// Explains why a scheme this server knows is nevertheless not served — the
/// two causes need different fixes (D13).
fn unavailable(product: &str, url_var: &str, configured: bool) -> McpError {
    let reason = if configured {
        format!("all {product} tools are disabled by ENABLED_TOOLS")
    } else {
        format!("{product} is not configured: set {url_var} and its credentials")
    };
    McpError::invalid_params(
        format!("{product} resources are not available: {reason}"),
        None,
    )
}
