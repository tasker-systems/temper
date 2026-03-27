use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Request body for `POST /api/upload/init`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadInitRequest {
    pub filename: String,
    pub size: u64,
    pub mime: String,
}

/// Response body for `POST /api/upload/init`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadInitResponse {
    /// Presigned PUT URL for direct R2 upload
    pub upload_url: String,
    /// Object key in R2 (e.g., "{context_id}/{resource_id}/{filename}")
    pub key: String,
}

/// Request body for `POST /api/upload/complete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadCompleteRequest {
    pub key: String,
    pub resource_ids: Vec<Uuid>,
    pub manifest_hash: String,
}

/// Processing status for an upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadProcessingStatus {
    /// Upload received, not yet processed
    Queued,
    /// Chunking and embedding in progress
    Processing,
    /// Chunks and embeddings written to Neon
    Complete,
    /// Processing failed
    Failed,
}

/// Response body for `GET /api/upload/:key/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadStatusResponse {
    pub key: String,
    pub status: UploadProcessingStatus,
    pub resources_processed: u32,
    pub resources_total: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upload_init_request_serde() {
        let req = UploadInitRequest {
            filename: "sync-batch-2026-03-27.zip".to_string(),
            size: 1024 * 1024,
            mime: "application/zip".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: UploadInitRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.filename, req.filename);
        assert_eq!(parsed.size, 1024 * 1024);
    }

    #[test]
    fn test_upload_processing_status_serde() {
        assert_eq!(
            serde_json::to_string(&UploadProcessingStatus::Queued).unwrap(),
            "\"queued\""
        );
        assert_eq!(
            serde_json::to_string(&UploadProcessingStatus::Complete).unwrap(),
            "\"complete\""
        );
    }
}
