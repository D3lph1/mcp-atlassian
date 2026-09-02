//! Append-only audit log of write operations (JSONL).
//!
//! Every tool that is not annotated `readOnlyHint: true` gets its handler
//! wrapped: the call is logged after it completes, with its arguments and its
//! outcome. Reads are never logged — they are the bulk of the traffic and
//! change nothing.
//!
//! What counts as a write comes from the same annotation `READ_ONLY`
//! filters on (D22), so a tool cannot be audited by one and ignored by the
//! other, and an unannotated tool is audited rather than silently skipped.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rmcp::handler::server::router::tool::{ToolRoute, ToolRouter};
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::{CallToolResponse, ErrorData};
use serde::Serialize;
use serde_json::{Map, Value};

/// Handle to the audit file. Cloning shares the same handle, so all routes
/// append to one file under one lock.
#[derive(Clone)]
pub struct AuditLog {
    file: Arc<Mutex<File>>,
    path: Arc<PathBuf>,
    /// Stamped onto every record when `DRY_RUN` is on (D26). Without it the
    /// log would claim writes that never left the process.
    dry_run: bool,
}

/// One line of the log.
#[derive(Serialize)]
struct Entry<'a> {
    /// RFC 3339 UTC, millisecond precision.
    ts: String,
    tool: &'a str,
    /// Arguments as the client sent them; `{}` for a tool that takes none.
    args: &'a Map<String, Value>,
    /// `ok`, `error`, or the non-terminal MRTR outcomes `input_required` /
    /// `task` — those mean the write has not happened yet.
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// What the write produced, when the result names it: the issue key of a
    /// create, the id of a page or comment. Enough to find or undo it from
    /// the log alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<String>,
    duration_ms: u64,
    /// Mirrors `destructiveHint`; present only when true, so `grep` finds the
    /// deletes and status changes without a JSON parser.
    #[serde(skip_serializing_if = "is_false")]
    destructive: bool,
    /// Present only when true: the call was intercepted by `DRY_RUN` and
    /// nothing was written (D26).
    #[serde(skip_serializing_if = "is_false")]
    dry_run: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl AuditLog {
    /// Opens (or creates) the log for appending. `dry_run` marks every record
    /// this log will write as an intercepted call.
    pub fn open(path: &Path, dry_run: bool) -> std::io::Result<Self> {
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        // The arguments hold whatever the client sent — comment bodies, page
        // content — so a file this process creates is owner-only (D23).
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
            path: Arc::new(path.to_path_buf()),
            dry_run,
        })
    }

    /// Appends one record for a finished tool call.
    fn record(
        &self,
        tool: &str,
        args: Option<Map<String, Value>>,
        destructive: bool,
        elapsed: Duration,
        result: &Result<CallToolResponse, ErrorData>,
    ) {
        let (outcome, error) = outcome_of(result);
        self.append(&Entry {
            ts: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            tool,
            args: &args.unwrap_or_default(),
            outcome,
            error,
            result: result_of(result),
            duration_ms: elapsed.as_millis() as u64,
            destructive,
            dry_run: self.dry_run,
        });
    }

    /// One `write_all` per record: the file is opened `O_APPEND`, so whole
    /// lines from concurrent calls do not interleave.
    ///
    /// A failure here is reported and dropped rather than propagated — losing
    /// the server because a disk filled up would be a worse outcome than a
    /// gap in the log, and the gap is visible in the logs at ERROR level.
    fn append(&self, entry: &Entry<'_>) {
        let mut line = match serde_json::to_vec(entry) {
            Ok(line) => line,
            Err(error) => {
                tracing::error!(%error, tool = entry.tool, "failed to encode an audit record");
                return;
            }
        };
        line.push(b'\n');
        // A poisoned lock only means some other call panicked mid-write; the
        // file handle itself is still usable, so keep auditing.
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(error) = file.write_all(&line).and_then(|()| file.flush()) {
            tracing::error!(
                path = %self.path.display(),
                %error,
                tool = entry.tool,
                "failed to append to the audit log"
            );
        }
    }
}

fn outcome_of(result: &Result<CallToolResponse, ErrorData>) -> (&'static str, Option<String>) {
    match result {
        Ok(CallToolResponse::Complete(result)) if result.is_error == Some(true) => (
            "error",
            result
                .content
                .iter()
                .find_map(|block| block.as_text().map(|text| text.text.clone())),
        ),
        Ok(CallToolResponse::Complete(_)) => ("ok", None),
        Ok(CallToolResponse::InputRequired(_)) => ("input_required", None),
        Ok(CallToolResponse::Task(_)) => ("task", None),
        Err(error) => ("error", Some(error.message.to_string())),
        // `CallToolResponse` is `#[non_exhaustive]`: a future variant is not
        // a completed write, so treat it like the other pending outcomes.
        _ => ("pending", None),
    }
}

/// The identifier a successful write reported, if its structured result
/// carries one at the top level: `key` (a created issue) before `id` (a
/// page, a comment, a remote link).
fn result_of(result: &Result<CallToolResponse, ErrorData>) -> Option<String> {
    let Ok(CallToolResponse::Complete(result)) = result else {
        return None;
    };
    if result.is_error == Some(true) {
        return None;
    }
    let structured = result.structured_content.as_ref()?;
    ["key", "id"]
        .iter()
        .find_map(|name| match structured.get(name)? {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
}

/// Wraps every write route of `router` so its calls are appended to `log`.
/// Read-only routes are passed through untouched.
pub fn instrument_writes<S>(router: ToolRouter<S>, log: AuditLog) -> ToolRouter<S>
where
    S: Send + Sync + 'static,
{
    let mut instrumented = ToolRouter::new();
    for (_, route) in router.map {
        let annotations = route.attr.annotations.as_ref();
        // Unannotated counts as a write, matching READ_ONLY (D22).
        let read_only = annotations.and_then(|a| a.read_only_hint).unwrap_or(false);
        if read_only {
            instrumented.add_route(route);
            continue;
        }
        let destructive = annotations
            .and_then(|a| a.destructive_hint)
            .unwrap_or(false);
        let inner_call = route.call.clone();
        let log = log.clone();
        instrumented.add_route(ToolRoute::new_dyn(
            route.attr,
            move |context: ToolCallContext<'_, S>| {
                let inner_call = inner_call.clone();
                let log = log.clone();
                let tool = context.name.to_string();
                let args = context.arguments.clone();
                Box::pin(async move {
                    let started = Instant::now();
                    let result = inner_call(context).await;
                    log.record(&tool, args, destructive, started.elapsed(), &result);
                    result
                })
            },
        ));
    }
    instrumented
}
