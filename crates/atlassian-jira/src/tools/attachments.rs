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

use atlassian_client::mcp::{list_result, saved, to_mcp_error, ListResult, StatusResult};
use atlassian_client::Upload;

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
    /// Local path to save the file to, e.g. `/tmp/report.pdf`; relative to
    /// ATTACHMENT_DIR when the server has one.
    pub save_path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UploadAttachmentArgs {
    /// Issue key, e.g. `PROJ-123`
    pub issue_key: String,
    /// Local path of the file to upload; relative to ATTACHMENT_DIR when the
    /// server has one.
    pub file_path: String,
}

#[tool_router(router = jira_attachments_router, vis = "pub(crate)")]
impl JiraTools {
    #[tool(
        title = "List Jira attachments",
        description = "List attachments of a Jira issue (id, filename, size, mime type).",
        annotations(read_only_hint = true, open_world_hint = false)
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
        title = "Download Jira attachment",
        description = "Download a Jira issue attachment to a local file. Writes to the local filesystem and overwrites save_path if it exists. Get attachment ids from jira_get_attachments first.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
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
        let target = self.files().writable(&args.save_path)?;
        let size = jira
            .download_attachment_to(attachment, &target, self.files().max_bytes())
            .await
            .map_err(to_mcp_error)?;
        saved(&attachment.filename, size, &target)
    }

    #[tool(
        title = "Upload Jira attachment",
        description = "Upload a local file as an attachment to a Jira issue.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn jira_upload_attachment(
        &self,
        Parameters(args): Parameters<UploadAttachmentArgs>,
    ) -> Result<Json<ListResult<Attachment>>, McpError> {
        let source = self.files().readable(&args.file_path)?;
        let upload = Upload::file(&source).await.map_err(to_mcp_error)?;
        let attachments = self
            .client()
            .upload_attachment(&args.issue_key, upload)
            .await
            .map_err(to_mcp_error)?;
        list_result(attachments)
    }
}
