//! Shared building blocks for exposing Atlassian data through MCP.
//!
//! Lives here so both product crates use the same result shapes without
//! depending on each other. Gated behind the `mcp` feature so the clients stay
//! usable as plain REST libraries (D15).

use rmcp::{handler::server::wrapper::Json, ErrorData as McpError};
use serde::Serialize;

/// Search results are capped to keep tool output token-friendly.
pub const MAX_SEARCH_RESULTS: u32 = 50;

/// Resolves a caller-supplied page size: the default when absent, capped at
/// [`MAX_SEARCH_RESULTS`] either way.
///
/// Every list tool goes through this rather than writing
/// `args.limit.unwrap_or(25)` itself. The cap is the point — an uncapped
/// `limit` is passed straight to Atlassian and floods the context window,
/// which is the failure D9 exists to prevent. Ten tools had grown the
/// unwrap without the `.min`, and nothing made that visible.
pub fn page_size(requested: Option<u32>, default: u32) -> u32 {
    requested.unwrap_or(default).min(MAX_SEARCH_RESULTS)
}

/// Wraps a list so the structured result stays a JSON object (D20).
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ListResult<T> {
    pub items: Vec<T>,
    /// Number of items in `items` — not the total available on the server.
    pub count: usize,
}

impl<T> ListResult<T> {
    pub fn new(items: Vec<T>) -> Self {
        Self {
            count: items.len(),
            items,
        }
    }
}

/// Result of an operation whose interesting output is "it worked".
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct StatusResult {
    /// Always true; failures surface as MCP errors, not as `ok: false`.
    pub ok: bool,
    /// Human-readable summary of what changed.
    pub message: String,
}

/// Builds a structured list result.
pub fn list_result<T>(items: Vec<T>) -> Result<Json<ListResult<T>>, McpError> {
    Ok(Json(ListResult::new(items)))
}

/// Builds a structured status result.
pub fn status_result(message: String) -> Result<Json<StatusResult>, McpError> {
    Ok(Json(StatusResult { ok: true, message }))
}

/// Where the attachment tools may read from and write to (D37).
///
/// With `ATTACHMENT_DIR` set, every `save_path` and `file_path` must resolve
/// inside it — after canonicalisation, so neither `..` nor a symlink leads
/// out. Without it the whole filesystem is reachable, which the server logs
/// at startup: a model that can upload `~/.ssh/id_ed25519` to a public Jira
/// is one prompt injection away from doing so.
#[derive(Debug, Clone)]
pub struct FileAccess {
    root: Option<std::path::PathBuf>,
    max_bytes: Option<u64>,
}

impl FileAccess {
    /// Canonicalises `root` (it must exist and be a directory); `max_bytes`
    /// of zero means no size limit.
    pub fn new(root: Option<&std::path::Path>, max_bytes: u64) -> crate::Result<Self> {
        let root = match root {
            Some(root) => {
                let canonical = std::fs::canonicalize(root).map_err(|e| {
                    crate::Error::Config(format!(
                        "ATTACHMENT_DIR `{}` is not usable: {e}",
                        root.display()
                    ))
                })?;
                if !canonical.is_dir() {
                    return Err(crate::Error::Config(format!(
                        "ATTACHMENT_DIR `{}` is not a directory",
                        root.display()
                    )));
                }
                Some(canonical)
            }
            None => None,
        };
        Ok(Self {
            root,
            max_bytes: (max_bytes > 0).then_some(max_bytes),
        })
    }

    /// No directory restriction and the default size limit — what tests use.
    pub fn unrestricted() -> Self {
        Self {
            root: None,
            max_bytes: Some(crate::config::DEFAULT_MAX_ATTACHMENT_BYTES),
        }
    }

    pub fn is_restricted(&self) -> bool {
        self.root.is_some()
    }

    pub fn max_bytes(&self) -> Option<u64> {
        self.max_bytes
    }

    /// The path a download may be written to. Relative paths are taken
    /// under `ATTACHMENT_DIR` when there is one. The parent must exist and
    /// resolve inside the directory; the target may not be a directory or
    /// an existing symlink, because that write would land somewhere else.
    pub fn writable(&self, save_path: &str) -> Result<std::path::PathBuf, McpError> {
        let given = std::path::Path::new(save_path);
        let target = match (&self.root, given.is_absolute()) {
            (Some(root), false) => root.join(given),
            _ => given.to_path_buf(),
        };
        let Some(file_name) = target.file_name() else {
            return Err(McpError::invalid_params(
                format!("save_path `{save_path}` does not name a file"),
                None,
            ));
        };
        let parent = target.parent().filter(|p| !p.as_os_str().is_empty());
        let parent =
            std::fs::canonicalize(parent.unwrap_or(std::path::Path::new("."))).map_err(|e| {
                McpError::invalid_params(
                    format!("the directory of save_path `{save_path}` is not usable: {e}"),
                    None,
                )
            })?;
        self.inside(&parent, "save_path", save_path)?;
        let resolved = parent.join(file_name);
        if let Ok(meta) = std::fs::symlink_metadata(&resolved) {
            if meta.is_dir() {
                return Err(McpError::invalid_params(
                    format!("save_path `{save_path}` is a directory"),
                    None,
                ));
            }
            if meta.file_type().is_symlink() {
                return Err(McpError::invalid_params(
                    format!("save_path `{save_path}` is a symlink; refusing to write through it"),
                    None,
                ));
            }
        }
        Ok(resolved)
    }

    /// The path an upload may be read from: it must exist, resolve inside
    /// `ATTACHMENT_DIR` when there is one, and fit the size limit.
    pub fn readable(&self, file_path: &str) -> Result<std::path::PathBuf, McpError> {
        let given = std::path::Path::new(file_path);
        let target = match (&self.root, given.is_absolute()) {
            (Some(root), false) => root.join(given),
            _ => given.to_path_buf(),
        };
        let resolved = std::fs::canonicalize(&target)
            .map_err(|e| McpError::invalid_params(format!("cannot read {file_path}: {e}"), None))?;
        self.inside(&resolved, "file_path", file_path)?;
        let size = std::fs::metadata(&resolved)
            .map_err(|e| McpError::invalid_params(format!("cannot read {file_path}: {e}"), None))?
            .len();
        if let Some(max) = self.max_bytes {
            if size > max {
                return Err(McpError::invalid_params(
                    format!(
                        "{file_path} is {size} bytes, over the MAX_ATTACHMENT_BYTES limit of {max} bytes"
                    ),
                    None,
                ));
            }
        }
        Ok(resolved)
    }

    fn inside(
        &self,
        canonical: &std::path::Path,
        argument: &str,
        given: &str,
    ) -> Result<(), McpError> {
        match &self.root {
            Some(root) if !canonical.starts_with(root) => Err(McpError::invalid_params(
                format!(
                    "{argument} `{given}` is outside ATTACHMENT_DIR `{}`; attachments may only \
                     be read from and written to that directory",
                    root.display()
                ),
                None,
            )),
            _ => Ok(()),
        }
    }
}

/// The result of a download that landed on disk.
pub fn saved(
    file_name: &str,
    size: u64,
    path: &std::path::Path,
) -> Result<Json<StatusResult>, McpError> {
    status_result(format!(
        "Saved {file_name} ({size} bytes) to {}",
        path.display()
    ))
}

/// Maps transport-layer errors to MCP errors, preserving the actionable
/// message (D13) and choosing the code a client can act on.
///
/// `invalid_params` (-32602) says "the call was wrong — fix the arguments":
/// a missing issue, a malformed id, a 400 from Atlassian, a parameter of the
/// other deployment. `internal_error` (-32603) says "the call was fine and
/// something else failed": auth, permissions, rate limits, 5xx, the network.
/// A client that retries internal errors and not invalid params would
/// otherwise retry a 404 three times. The HTTP status rides along in `data`
/// where there is one.
pub fn to_mcp_error(err: crate::Error) -> McpError {
    use crate::Error;
    let status = match &err {
        Error::Unauthorized => Some(401),
        Error::Forbidden => Some(403),
        Error::NotFound(_) => Some(404),
        Error::RateLimited => Some(429),
        Error::Api { status, .. } => Some(*status),
        _ => None,
    };
    let data = status.map(|status| serde_json::json!({ "status": status }));
    let callers_fault = match &err {
        Error::NotFound(_) | Error::InvalidUrl { .. } | Error::Config(_) | Error::File(_) => true,
        Error::Api { status, .. } => {
            (400..500).contains(status) && !matches!(status, 401 | 403 | 429)
        }
        _ => false,
    };
    if callers_fault {
        McpError::invalid_params(err.to_string(), data)
    } else {
        McpError::internal_error(err.to_string(), data)
    }
}

#[cfg(test)]
mod tests {
    use super::{page_size, to_mcp_error, MAX_SEARCH_RESULTS};
    use crate::Error;
    use rmcp::model::ErrorCode;

    #[test]
    fn what_the_caller_can_fix_is_invalid_params() {
        for error in [
            Error::NotFound("issue PROJ-404".into()),
            Error::InvalidUrl {
                url: "x".into(),
                message: "y".into(),
            },
            Error::Config("no Epic Link field".into()),
            Error::File("attachment is 1 bytes over".into()),
            Error::Api {
                status: 400,
                message: "bad JQL".into(),
            },
        ] {
            let message = error.to_string();
            let mapped = to_mcp_error(error);
            assert_eq!(mapped.code, ErrorCode::INVALID_PARAMS, "{message}");
            assert_eq!(mapped.message, message);
        }
        assert_eq!(
            to_mcp_error(Error::NotFound("x".into())).data,
            Some(serde_json::json!({ "status": 404 }))
        );
    }

    #[test]
    fn what_the_caller_cannot_fix_is_internal() {
        for error in [
            Error::Unauthorized,
            Error::Forbidden,
            Error::RateLimited,
            Error::Api {
                status: 503,
                message: "down".into(),
            },
            Error::Decode("bad json".into()),
            Error::OAuth("refresh failed".into()),
        ] {
            let message = error.to_string();
            assert_eq!(
                to_mcp_error(error).code,
                ErrorCode::INTERNAL_ERROR,
                "{message}"
            );
        }
        assert_eq!(
            to_mcp_error(Error::Unauthorized).data,
            Some(serde_json::json!({ "status": 401 }))
        );
    }

    #[test]
    fn an_absent_page_size_falls_back_to_the_default() {
        assert_eq!(page_size(None, 25), 25);
        assert_eq!(page_size(Some(5), 25), 5);
    }

    #[test]
    fn a_page_size_is_capped_however_it_arrives() {
        assert_eq!(page_size(Some(100_000), 25), MAX_SEARCH_RESULTS);
        // Including one a tool author defaulted above the cap.
        assert_eq!(page_size(None, 100), MAX_SEARCH_RESULTS);
    }
}

#[cfg(test)]
mod file_access_tests {
    use super::FileAccess;

    fn sandbox(case: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mcp-files-{case}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        std::fs::write(dir.join("inner/small.txt"), b"hi").unwrap();
        dir
    }

    #[test]
    fn a_restricted_root_keeps_reads_and_writes_inside_it() {
        let dir = sandbox("inside");
        let files = FileAccess::new(Some(&dir), 0).unwrap();
        assert!(files.is_restricted());

        // Relative paths land under the root; absolute ones must be under it.
        let target = files.writable("inner/out.bin").unwrap();
        assert!(target.starts_with(std::fs::canonicalize(&dir).unwrap()));
        assert!(files
            .writable(dir.join("inner/out.bin").to_str().unwrap())
            .is_ok());
        assert!(files.readable("inner/small.txt").is_ok());

        for outside in ["../escape.bin", "/tmp/escape.bin", "inner/../../escape.bin"] {
            let error = files.writable(outside).unwrap_err().to_string();
            assert!(error.contains("ATTACHMENT_DIR"), "{outside}: {error}");
        }
        let error = files.readable("/etc/hosts").unwrap_err().to_string();
        assert!(error.contains("ATTACHMENT_DIR"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_cannot_lead_out_in_either_direction() {
        let dir = sandbox("symlink");
        let outside =
            std::env::temp_dir().join(format!("mcp-files-outside-{}", std::process::id()));
        std::fs::write(&outside, b"secret").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("inner/link")).unwrap();
        let files = FileAccess::new(Some(&dir), 0).unwrap();

        // Reading through the link resolves to the real file, which is outside.
        assert!(files.readable("inner/link").is_err());
        // Writing onto an existing symlink is refused, wherever it points.
        let error = files.writable("inner/link").unwrap_err().to_string();
        assert!(error.contains("symlink"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn the_size_limit_applies_to_uploads() {
        let dir = sandbox("size");
        let files = FileAccess::new(Some(&dir), 1).unwrap();
        let error = files.readable("inner/small.txt").unwrap_err().to_string();
        assert!(error.contains("MAX_ATTACHMENT_BYTES"), "{error}");
        assert_eq!(files.max_bytes(), Some(1));
        assert_eq!(FileAccess::new(Some(&dir), 0).unwrap().max_bytes(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_root_is_a_configuration_error_naming_the_variable() {
        let error = FileAccess::new(Some(std::path::Path::new("/nonexistent-dir-xyz")), 0)
            .unwrap_err()
            .to_string();
        assert!(error.contains("ATTACHMENT_DIR"), "{error}");
        let files = FileAccess::unrestricted();
        assert!(!files.is_restricted());
        assert!(files.writable("/tmp/anything.bin").is_ok());
    }
}
