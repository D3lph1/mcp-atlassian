//! Attachments: list, download to disk, upload from disk. Download resolves
//! the attachment id through the issue's attachment list, so the client only
//! ever fetches same-origin URLs it received from the API.

use crate::Attachment;
use crate::JiraTools;
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use atlassian_client::mcp::{list_result, status_result, to_mcp_error, ListResult, StatusResult};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetAttachmentsArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DownloadAttachmentArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
    /// Attachment id (see jira_get_attachments).
    pub attachment_id: String,
    /// Absolute local path to save the file to, e.g. `/tmp/report.pdf`.
    pub save_path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UploadAttachmentArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
    /// Absolute local path of the file to upload.
    pub file_path: String,
}

#[tool_router(router = jira_attachments_router, vis = "pub(crate)")]
impl JiraTools {
    #[tool(
        description = "List attachments of a Jira issue (id, filename, size, mime type).",
        annotations(read_only_hint = true)
    )]
    async fn jira_get_attachments(
        &self,
        Parameters(args): Parameters<GetAttachmentsArgs>,
    ) -> Result<Json<ListResult<Attachment>>, McpError> {
        let attachments = self
            .client()
            .get_attachments(&args.issue_key)
            .await
            .map_err(to_mcp_error)?;
        list_result(attachments)
    }

    #[tool(
        description = "Download a Jira issue attachment to a local file. Get attachment ids from jira_get_attachments first.",
        annotations(read_only_hint = true)
    )]
    async fn jira_download_attachment(
        &self,
        Parameters(args): Parameters<DownloadAttachmentArgs>,
    ) -> Result<Json<StatusResult>, McpError> {
        let jira = self.client();
        let attachments = jira
            .get_attachments(&args.issue_key)
            .await
            .map_err(to_mcp_error)?;
        let attachment = attachments
            .iter()
            .find(|a| a.id == args.attachment_id)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "attachment {} not found on issue {}",
                        args.attachment_id, args.issue_key
                    ),
                    None,
                )
            })?;
        let bytes = jira
            .download_attachment(&attachment.content)
            .await
            .map_err(to_mcp_error)?;
        let size = bytes.len();
        tokio::fs::write(&args.save_path, bytes)
            .await
            .map_err(|e| {
                McpError::internal_error(format!("failed to write {}: {e}", args.save_path), None)
            })?;
        status_result(format!(
            "Saved {} ({size} bytes) to {}",
            attachment.filename, args.save_path
        ))
    }

    #[tool(
        description = "Upload a local file as an attachment to a Jira issue.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn jira_upload_attachment(
        &self,
        Parameters(args): Parameters<UploadAttachmentArgs>,
    ) -> Result<Json<ListResult<Attachment>>, McpError> {
        let bytes = tokio::fs::read(&args.file_path).await.map_err(|e| {
            McpError::invalid_params(format!("cannot read {}: {e}", args.file_path), None)
        })?;
        let file_name = std::path::Path::new(&args.file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("attachment")
            .to_string();
        let attachments = self
            .client()
            .upload_attachment(&args.issue_key, &file_name, bytes)
            .await
            .map_err(to_mcp_error)?;
        list_result(attachments)
    }
}
