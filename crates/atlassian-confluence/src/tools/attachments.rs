//! Page attachments: list, download to disk, upload from disk, delete.
//! Download resolves the attachment through the page's attachment list, so
//! only same-origin URLs the API returned are ever fetched.

use crate::ConfluenceTools;
use crate::{ConfluenceAttachment, ResultsPage};
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router, ErrorData as McpError,
};
use serde::Deserialize;

use atlassian_client::mcp::{status_result, to_mcp_error, StatusResult};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetAttachmentsArgs {
    /// Numeric page id.
    pub page_id: String,
    /// Max attachments to return (default 25).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DownloadAttachmentArgs {
    /// Numeric page id the attachment belongs to.
    pub page_id: String,
    /// Attachment id from confluence_get_attachments.
    pub attachment_id: String,
    /// Absolute local path to save the file to, e.g. `/tmp/diagram.png`.
    pub save_path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UploadAttachmentArgs {
    /// Numeric page id to attach the file to.
    pub page_id: String,
    /// Absolute local path of the file to upload.
    pub file_path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteAttachmentArgs {
    /// Attachment id from confluence_get_attachments.
    pub attachment_id: String,
}

#[tool_router(router = confluence_attachments_router, vis = "pub(crate)")]
impl ConfluenceTools {
    #[tool(
        description = "List attachments of a Confluence page (id, filename, size, media type).",
        annotations(read_only_hint = true)
    )]
    async fn confluence_get_attachments(
        &self,
        Parameters(args): Parameters<GetAttachmentsArgs>,
    ) -> Result<Json<ResultsPage<ConfluenceAttachment>>, McpError> {
        let attachments = self
            .client()
            .get_attachments(&args.page_id, args.limit.unwrap_or(25))
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(attachments))
    }

    #[tool(
        description = "Download a Confluence page attachment to a local file. Get attachment ids from confluence_get_attachments first.",
        annotations(read_only_hint = true)
    )]
    async fn confluence_download_attachment(
        &self,
        Parameters(args): Parameters<DownloadAttachmentArgs>,
    ) -> Result<Json<StatusResult>, McpError> {
        let confluence = self.client();
        let attachments = confluence
            .get_attachments(&args.page_id, 200)
            .await
            .map_err(to_mcp_error)?;
        let attachment = attachments
            .results
            .iter()
            .find(|a| a.id == args.attachment_id)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "attachment {} not found on page {}",
                        args.attachment_id, args.page_id
                    ),
                    None,
                )
            })?;
        let download = attachment
            .links
            .as_ref()
            .and_then(|l| l.download.as_deref())
            .ok_or_else(|| {
                McpError::internal_error(
                    format!("attachment {} has no download link", args.attachment_id),
                    None,
                )
            })?;
        let bytes = confluence
            .download_attachment(download)
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
            attachment.title, args.save_path
        ))
    }

    #[tool(
        description = "Upload a local file as an attachment to a Confluence page.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn confluence_upload_attachment(
        &self,
        Parameters(args): Parameters<UploadAttachmentArgs>,
    ) -> Result<Json<ResultsPage<ConfluenceAttachment>>, McpError> {
        let bytes = tokio::fs::read(&args.file_path).await.map_err(|e| {
            McpError::invalid_params(format!("cannot read {}: {e}", args.file_path), None)
        })?;
        let file_name = std::path::Path::new(&args.file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("attachment")
            .to_string();
        let uploaded = self
            .client()
            .upload_attachment(&args.page_id, &file_name, bytes)
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(uploaded))
    }

    #[tool(
        description = "Delete a Confluence attachment permanently. This cannot be undone — confirm with the user before calling.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn confluence_delete_attachment(
        &self,
        Parameters(args): Parameters<DeleteAttachmentArgs>,
    ) -> Result<Json<StatusResult>, McpError> {
        self.client()
            .delete_attachment(&args.attachment_id)
            .await
            .map_err(to_mcp_error)?;
        status_result(format!("Attachment {} deleted", args.attachment_id))
    }
}
