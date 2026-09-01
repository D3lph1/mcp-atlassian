//! Shared HTTP client, authentication and configuration for Atlassian REST APIs.
//!
//! This crate knows nothing about MCP or about concrete Atlassian products —
//! it only provides the transport layer that `atlassian-jira` and
//! `atlassian-confluence` build on.

#[cfg(feature = "mcp")]
pub mod mcp;

mod config;
mod error;
mod http;
pub mod oauth;

pub use config::{Auth, Config, ServiceConfig};
pub use error::{Error, Result};
pub use http::AtlassianClient;
