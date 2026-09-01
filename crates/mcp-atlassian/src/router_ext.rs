//! Projecting a `ToolRouter` from one server state onto another.
//!
//! `#[tool_router]` generates *inherent* methods, which Rust only allows in the
//! crate that declares the type. That would force every tool into whichever
//! crate declares the server — splitting each product across two crates (its
//! client here, its tools there).
//!
//! Instead each product crate declares its own small state type (holding just
//! its client) and builds a `ToolRouter` over it; this adapter re-targets those
//! routes onto the composite server, so a product stays in one crate.

use rmcp::handler::server::router::tool::{ToolRoute, ToolRouter};
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::CallToolRequestParams;

/// Re-targets every route of `router` from state `T` onto state `S`, using
/// `project` to reach the inner state. Request data (arguments, MRTR input
/// responses and request state) is carried across unchanged.
pub fn project_router<S, T, F>(router: ToolRouter<T>, project: F) -> ToolRouter<S>
where
    F: Fn(&S) -> &T + Copy + Send + Sync + 'static,
    S: Send + Sync + 'static,
    T: Send + Sync + 'static,
{
    let mut projected = ToolRouter::new();
    for (_, route) in router.map {
        let inner_call = route.call.clone();
        projected.add_route(ToolRoute::new_dyn(
            route.attr,
            move |context: ToolCallContext<'_, S>| {
                let inner_call = inner_call.clone();
                let mut params = CallToolRequestParams::new(context.name.clone());
                if let Some(arguments) = context.arguments.clone() {
                    params = params.with_arguments(arguments);
                }
                if let Some(responses) = context.input_responses.clone() {
                    params = params.with_input_responses(responses);
                }
                if let Some(state) = context.request_state.clone() {
                    params = params.with_request_state(state);
                }
                let inner_context = ToolCallContext::new(
                    project(context.service),
                    params,
                    context.request_context.clone(),
                );
                inner_call(inner_context)
            },
        ));
    }
    projected
}
