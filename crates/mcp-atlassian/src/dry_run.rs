//! `DRY_RUN`: write tools stay callable but never reach Atlassian (D26).
//!
//! `READ_ONLY` (D22) removes write tools from `tools/list` altogether,
//! which is the right answer for an untrusted client but useless for
//! rehearsing a prompt: a tool the model cannot see is a tool it cannot be
//! observed choosing. Dry run keeps the surface advertised and intercepts the
//! call instead — arguments are checked against the tool's own input schema,
//! then described back to the caller.
//!
//! What counts as a write is the `readOnlyHint` annotation, the same source of
//! truth `READ_ONLY` and the audit log use, so a tool cannot be
//! intercepted by one and executed by the other, and an unannotated tool is
//! intercepted rather than silently performed.
//!
//! The check is deliberately shallow: presence of required arguments and
//! nothing else. Types and values are the tool's business, and reproducing
//! them here would mean a second, drifting copy of every schema.

use rmcp::handler::server::common::schema_for_type;
use rmcp::handler::server::router::tool::{ToolRoute, ToolRouter};
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock, ErrorData as McpError};
use serde::Serialize;
use serde_json::{Map, Value};

/// Appended to the description of every intercepted tool. The mode is
/// disclosed rather than hidden: a model that believes its writes landed will
/// report them as done, which is exactly the confusion this mode exists to
/// avoid.
const NOTICE: &str = " DRY_RUN is enabled on this server: calling this tool validates and \
                       describes the operation without performing it.";

/// What an intercepted call returns in place of the tool's own output.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DryRunReport {
    /// Always true. A real tool result never carries this field, so a client
    /// can tell the two apart without knowing the server's configuration.
    pub dry_run: bool,
    /// The tool that would have run.
    pub tool: String,
    /// Mirrors `destructiveHint`.
    pub destructive: bool,
    /// The arguments as the client sent them.
    pub arguments: Map<String, Value>,
    /// Things that would not have failed the real call but are probably
    /// mistakes — an argument the tool does not declare, which serde would
    /// have dropped without a word.
    pub warnings: Vec<String>,
}

/// Replaces every write route of `router` with one that reports what the call
/// would have done. Read-only routes are passed through untouched — they
/// perform the read for real, which is what makes a rehearsal useful.
pub fn intercept_writes<S>(router: ToolRouter<S>) -> ToolRouter<S>
where
    S: Send + Sync + 'static,
{
    let output_schema = schema_for_type::<DryRunReport>();
    let mut intercepted = ToolRouter::new();
    for (_, route) in router.map {
        // Unannotated counts as a write, matching READ_ONLY (D22).
        let read_only = route
            .attr
            .annotations
            .as_ref()
            .and_then(|a| a.read_only_hint)
            .unwrap_or(false);
        if read_only {
            intercepted.add_route(route);
            continue;
        }

        let mut attr = route.attr;
        let destructive = attr
            .annotations
            .as_ref()
            .and_then(|a| a.destructive_hint)
            .unwrap_or(false);
        let name = attr.name.to_string();
        let input_schema = attr.input_schema.clone();
        attr.description = Some(match attr.description {
            Some(existing) => format!("{existing}{NOTICE}").into(),
            None => NOTICE.trim_start().into(),
        });
        // The tool's own result never materializes, so its schema would
        // describe something the client will not receive. Advertise the report
        // instead — every tool still carries an output schema (D20).
        attr.output_schema = Some(output_schema.clone());

        intercepted.add_route(ToolRoute::new_dyn(
            attr,
            move |context: ToolCallContext<'_, S>| {
                let name = name.clone();
                let input_schema = input_schema.clone();
                let arguments = context.arguments.clone().unwrap_or_default();
                Box::pin(async move { report(&name, destructive, arguments, &input_schema) })
            },
        ));
    }
    intercepted
}

/// Builds the report, or an error when the call could not have succeeded.
fn report(
    tool: &str,
    destructive: bool,
    arguments: Map<String, Value>,
    input_schema: &Map<String, Value>,
) -> Result<CallToolResponse, McpError> {
    let missing = missing_arguments(&arguments, input_schema);
    if !missing.is_empty() {
        // The real call would have failed to deserialize its parameters.
        // Reporting success here would teach the caller a prompt works when it
        // does not, which defeats the point of the mode.
        return Err(McpError::invalid_params(
            format!(
                "{tool} is missing required argument(s): {}. Nothing was sent to Atlassian \
                 (DRY_RUN is enabled)",
                missing.join(", ")
            ),
            None,
        ));
    }

    let warnings = undeclared_arguments(&arguments, input_schema);
    let report = DryRunReport {
        dry_run: true,
        tool: tool.to_string(),
        destructive,
        arguments,
        warnings,
    };
    let structured = serde_json::to_value(&report).map_err(|e| {
        McpError::internal_error(format!("failed to encode the dry-run report: {e}"), None)
    })?;
    let mut result = CallToolResult::success(vec![ContentBlock::text(describe(&report))]);
    result.structured_content = Some(structured);
    Ok(result.into())
}

/// Human-readable form of the report, for clients that render text only.
fn describe(report: &DryRunReport) -> String {
    let mut text = format!(
        "DRY RUN — `{}` was not performed{}.",
        report.tool,
        if report.destructive {
            " (this tool is destructive)"
        } else {
            ""
        }
    );
    if report.arguments.is_empty() {
        text.push_str(" It takes no arguments.");
    } else {
        let arguments = serde_json::to_string_pretty(&report.arguments)
            .unwrap_or_else(|_| "<unprintable>".into());
        text.push_str(&format!(" It would have run with:\n{arguments}"));
    }
    for warning in &report.warnings {
        text.push_str(&format!("\nWarning: {warning}"));
    }
    text
}

/// Required properties of the schema that the call did not supply.
fn missing_arguments(arguments: &Map<String, Value>, schema: &Map<String, Value>) -> Vec<String> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|required| {
            required
                .iter()
                .filter_map(Value::as_str)
                .filter(|name| !arguments.contains_key(*name))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Arguments the schema does not declare. Not an error — serde ignores unknown
/// fields, so the real call would have run, just without them.
fn undeclared_arguments(
    arguments: &Map<String, Value>,
    schema: &Map<String, Value>,
) -> Vec<String> {
    let Some(declared) = schema.get("properties").and_then(Value::as_object) else {
        // A schema without `properties` declares nothing to compare against.
        return Vec::new();
    };
    arguments
        .keys()
        .filter(|name| !declared.contains_key(*name))
        .map(|name| format!("`{name}` is not an argument of this tool and would have been ignored"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Map<String, Value> {
        json!({
            "type": "object",
            "properties": { "issue_key": { "type": "string" }, "fields": { "type": "object" } },
            "required": ["issue_key"]
        })
        .as_object()
        .cloned()
        .unwrap()
    }

    fn arguments(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn a_missing_required_argument_is_reported_as_missing() {
        assert_eq!(
            missing_arguments(&arguments(json!({ "fields": {} })), &schema()),
            ["issue_key"]
        );
        assert!(
            missing_arguments(&arguments(json!({ "issue_key": "PROJ-1" })), &schema()).is_empty()
        );
    }

    #[test]
    fn a_misspelled_argument_is_a_warning_not_an_error() {
        let arguments = arguments(json!({ "issue_key": "PROJ-1", "issueKey": "PROJ-1" }));
        assert!(missing_arguments(&arguments, &schema()).is_empty());
        let warnings = undeclared_arguments(&arguments, &schema());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("issueKey"), "{warnings:?}");
    }

    #[test]
    fn a_schema_without_properties_warns_about_nothing() {
        let schema = arguments(json!({ "type": "object" }));
        assert!(undeclared_arguments(&arguments(json!({ "anything": 1 })), &schema).is_empty());
    }
}
