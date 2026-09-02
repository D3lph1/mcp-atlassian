//! `CONFIRM_DESTRUCTIVE`: a destructive tool asks the user before it runs
//! (D42).
//!
//! `destructiveHint` already tells a client which tools deserve a
//! confirmation (D22); this wrapper asks for it through MCP elicitation, so
//! a client that shows forms gets a yes/no before a delete, a transition or
//! a restriction change happens. Same shape as the audit and dry-run
//! wrappers: a route is re-targeted, the annotation decides.
//!
//! Opt-in, because a client that does not implement elicitation would see
//! every destructive call fail: the wrapper checks the client's declared
//! capability, and when it is absent runs the tool as before and says so
//! once at WARN.

use std::sync::atomic::{AtomicBool, Ordering};

use rmcp::handler::server::router::tool::{ToolRoute, ToolRouter};
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::{
    BooleanSchema, CallToolResult, ContentBlock, ElicitRequestParams, ElicitationAction,
    ElicitationSchema, PrimitiveSchemaDefinition,
};
use serde_json::{Map, Value};

/// Replaces every destructive route of `router` with one that asks first.
pub fn confirm_destructive<S>(router: ToolRouter<S>) -> ToolRouter<S>
where
    S: Send + Sync + 'static,
{
    let warned = std::sync::Arc::new(AtomicBool::new(false));
    let mut confirmed = ToolRouter::new();
    for (_, route) in router.map {
        let destructive = route
            .attr
            .annotations
            .as_ref()
            .and_then(|a| a.destructive_hint)
            .unwrap_or(false);
        if !destructive {
            confirmed.add_route(route);
            continue;
        }
        let inner_call = route.call.clone();
        let title = route
            .attr
            .title
            .clone()
            .unwrap_or_else(|| route.attr.name.to_string());
        let name = route.attr.name.to_string();
        let warned = warned.clone();
        confirmed.add_route(ToolRoute::new_dyn(
            route.attr,
            move |context: ToolCallContext<'_, S>| {
                let inner_call = inner_call.clone();
                let title = title.clone();
                let name = name.clone();
                let warned = warned.clone();
                let peer = context.request_context.peer.clone();
                let arguments = context.arguments.clone().unwrap_or_default();
                Box::pin(async move {
                    let supported = peer
                        .peer_info()
                        .is_some_and(|info| info.capabilities.elicitation.is_some());
                    if !supported {
                        if !warned.swap(true, Ordering::Relaxed) {
                            tracing::warn!(
                                "CONFIRM_DESTRUCTIVE is set but this client does not support \
                                 elicitation; destructive tools run without confirmation"
                            );
                        }
                        return inner_call(context).await;
                    }
                    let request = ElicitRequestParams::FormElicitationParams {
                        meta: None,
                        message: message(&title, &name, &arguments),
                        requested_schema: schema(),
                    };
                    let answer = peer.create_elicitation(request).await.map_err(|e| {
                        rmcp::ErrorData::internal_error(
                            format!("{name} needs confirmation and asking failed: {e}"),
                            None,
                        )
                    })?;
                    let accepted = answer.action == ElicitationAction::Accept
                        && answer
                            .content
                            .as_ref()
                            .and_then(|c| c.get("confirm"))
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                    if !accepted {
                        // An error result, not a success: the audit log records
                        // it as such, and a model reading it will not report
                        // the write as done.
                        return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                            "{name} was not performed: the user did not confirm it"
                        ))])
                        .into());
                    }
                    inner_call(context).await
                })
            },
        ));
    }
    confirmed
}

fn message(title: &str, name: &str, arguments: &Map<String, Value>) -> String {
    let args = serde_json::to_string(arguments).unwrap_or_else(|_| "{}".into());
    format!("{title}: perform this destructive operation?\n{name} {args}")
}

fn schema() -> ElicitationSchema {
    ElicitationSchema::builder()
        .required_property(
            "confirm",
            PrimitiveSchemaDefinition::Boolean(BooleanSchema::new().title("Confirm")),
        )
        .build()
        .expect("a one-field boolean schema is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_question_names_the_tool_and_its_arguments() {
        let args = json!({ "issue_key": "PROJ-1" })
            .as_object()
            .cloned()
            .unwrap();
        let text = message("Delete Jira issue", "jira_delete_issue", &args);
        assert!(text.contains("Delete Jira issue"), "{text}");
        assert!(text.contains("PROJ-1"), "{text}");
        assert!(text.contains("jira_delete_issue"), "{text}");
    }

    #[test]
    fn the_form_has_one_required_boolean() {
        let value = serde_json::to_value(schema()).unwrap();
        assert_eq!(value["required"], json!(["confirm"]));
        assert_eq!(value["properties"]["confirm"]["type"], "boolean");
    }
}
