//! Library target of the MCP server binary — exposes the server for
//! integration tests. The binary entry point lives in `main.rs`.
//!
//! Tools themselves live in the product crates (`atlassian-jira`,
//! `atlassian-confluence`) next to the clients they call; this crate only
//! composes them into one server. See [`router_ext`] for how routers defined
//! over a product's own state are re-targeted onto [`server::AtlassianServer`].

pub mod audit;
pub mod dry_run;
pub mod router_ext;
pub mod server;
