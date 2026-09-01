//! Confluence tools, split by domain. Each submodule owns its argument
//! schemas and contributes a named `ToolRouter` (an associated function on
//! `AtlassianServer` — hence the product prefix in the names); [`router`]
//! merges them.

pub mod admin;
pub mod attachments;
pub mod comments;
pub mod pages;
pub mod search;
pub mod spaces;
pub mod storage;
pub mod versions;

use rmcp::handler::server::router::tool::ToolRouter;

use crate::ConfluenceTools;

/// All Confluence tool routes.
pub fn router() -> ToolRouter<ConfluenceTools> {
    let mut router = ToolRouter::default();
    router += ConfluenceTools::confluence_search_router();
    router += ConfluenceTools::confluence_pages_router();
    router += ConfluenceTools::confluence_comments_router();
    router += ConfluenceTools::confluence_spaces_router();
    router += ConfluenceTools::confluence_attachments_router();
    router += ConfluenceTools::confluence_versions_router();
    router += ConfluenceTools::confluence_admin_router();
    router
}
