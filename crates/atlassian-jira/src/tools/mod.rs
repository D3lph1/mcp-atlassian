//! Jira tools, split by domain. Each submodule owns its argument schemas and
//! contributes a named `ToolRouter` (an associated function on
//! `AtlassianServer` — hence the product prefix in the names); [`router`]
//! merges them.

pub mod agile;
pub mod attachments;
pub mod comments;
pub mod fields;
pub mod issues;
pub mod links;
pub mod meta;
pub mod search;
pub mod transitions;
pub mod users;

use rmcp::handler::server::router::tool::ToolRouter;

use crate::JiraTools;

/// All Jira tool routes.
pub fn router() -> ToolRouter<JiraTools> {
    let mut router = ToolRouter::default();
    router += JiraTools::jira_meta_router();
    router += JiraTools::jira_search_router();
    router += JiraTools::jira_issues_router();
    router += JiraTools::jira_transitions_router();
    router += JiraTools::jira_comments_router();
    router += JiraTools::jira_agile_router();
    router += JiraTools::jira_attachments_router();
    router += JiraTools::jira_users_router();
    router += JiraTools::jira_links_router();
    router += JiraTools::jira_fields_router();
    router
}
