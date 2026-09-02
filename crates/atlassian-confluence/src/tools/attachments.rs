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

use atlassian_client::mcp::{page_size, saved, status_result, to_mcp_error, StatusResult};
use atlassian_client::Upload;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetAttachmentsArgs {
    /// Numeric page id.
    pub page_id: String,
    /// Max attachments to return (default 25, cap 50).
    pub limit: Option<u32>,
    /// Offset of the first attachment; pass the previous page's `start + size`
    /// while `has_more` is true.
    pub start: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DownloadAttachmentArgs {
    /// Numeric page id the attachment belongs to.
    pub page_id: String,
    /// Attachment id from confluence_get_attachments.
    pub attachment_id: String,
    /// Local path to save the file to, e.g. `/tmp/diagram.png`; relative to
    /// ATTACHMENT_DIR when the server has one.
    pub save_path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UploadAttachmentArgs {
    /// Numeric page id to attach the file to.
    pub page_id: String,
    /// Local path of the file to upload; relative to ATTACHMENT_DIR when the
    /// server has one.
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
        title = "List Confluence attachments",
        description = "List attachments of a Confluence page (id, filename, size, media type).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn confluence_get_attachments(
        &self,
        Parameters(args): Parameters<GetAttachmentsArgs>,
    ) -> Result<Json<ResultsPage<ConfluenceAttachment>>, McpError> {
        let attachments = self
            .client()
            .get_attachments(
                &args.page_id,
                page_size(args.limit, 25),
                // An offset, not a page size — capping it would cap paging.
                args.start.unwrap_or(0),
            )
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(attachments))
    }

    #[tool(
        title = "Download Confluence attachment",
        description = "Download a Confluence page attachment to a local file. Writes to the local filesystem and overwrites save_path if it exists. Get attachment ids from confluence_get_attachments first.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn confluence_download_attachment(
        &self,
        Parameters(args): Parameters<DownloadAttachmentArgs>,
    ) -> Result<Json<StatusResult>, McpError> {
        let confluence = self.client();
        let attachments = confluence
            .get_attachments(&args.page_id, 200, 0)
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
        let target = self.files().writable(&args.save_path)?;
        let size = confluence
            .download_attachment_to(download, &target, self.files().max_bytes())
            .await
            .map_err(to_mcp_error)?;
        saved(&attachment.title, size, &target)
    }

    #[tool(
        title = "Upload Confluence attachment",
        description = "Upload a local file as an attachment to a Confluence page.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn confluence_upload_attachment(
        &self,
        Parameters(args): Parameters<UploadAttachmentArgs>,
    ) -> Result<Json<ResultsPage<ConfluenceAttachment>>, McpError> {
        let source = self.files().readable(&args.file_path)?;
        let upload = Upload::file(&source).await.map_err(to_mcp_error)?;
        let uploaded = self
            .client()
            .upload_attachment(&args.page_id, upload)
            .await
            .map_err(to_mcp_error)?;
        Ok(Json(uploaded))
    }

    #[tool(
        title = "Delete Confluence attachment",
        description = "Delete a Confluence attachment permanently. This cannot be undone — confirm with the user before calling.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
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
