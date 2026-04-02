use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Response body from `POST /api/upload` (TypeScript endpoint).
///
/// The upload endpoint stores the file in Vercel Blob and returns
/// a tracking ID with initial status. Processing (extract, chunk,
/// embed) happens asynchronously via Vercel Workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadResponse {
    pub blob_file_id: Uuid,
    pub status: UploadProcessingStatus,
}

/// Processing status for an uploaded file.
///
/// Tracks the lifecycle of the durable Workflow pipeline:
/// pending → processing → processed → failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadProcessingStatus {
    /// File stored in Vercel Blob, workflow not yet started
    Pending,
    /// Extraction, chunking, and embedding in progress
    Processing,
    /// Chunks and embeddings written to kb_chunks
    Processed,
    /// Processing failed (check error_message)
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upload_processing_status_serde() {
        assert_eq!(
            serde_json::to_string(&UploadProcessingStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&UploadProcessingStatus::Processed).unwrap(),
            "\"processed\""
        );
    }

    #[test]
    fn test_upload_response_serde() {
        let resp = UploadResponse {
            blob_file_id: Uuid::nil(),
            status: UploadProcessingStatus::Pending,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: UploadResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.status, UploadProcessingStatus::Pending);
    }
}
