//! Typed sub-client for the blob surfaces (`/api/blobs*`).
//!
//! Bytes over the wire as bytes: the commit is one multipart request (at or under the
//! server's single-request threshold, D7), the segmented append carries RAW BINARY
//! segments — the segment's identity is the SERVER's own sha256 of the bytes received,
//! and the whole assembly's integrity is finalize's `expected_content_hash` — and the
//! read streams the response body: the CLI writes it to a file or stdout without ever
//! needing the whole blob in memory unless it wants it.

use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::authorship::ActInput;
use temper_core::types::blob::{
    BlobCommitResponse, BlobDeleteAck, BlobRelationAck, BlobRelationAssertRequest, BlobRelationRow,
    BlobSummary, BlobUploadBeginRequest, BlobUploadBeginResponse, BlobUploadFinalizeRequest,
    BlobUploadProgress,
};

/// Sub-client for blob commit/read/list/relate + segmented upload.
pub struct BlobClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for BlobClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlobClient").finish_non_exhaustive()
    }
}

impl<'a> BlobClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// POST /api/blobs — commit bytes as one multipart request. `content_type` is the
    /// media type the blob commits under (the server allowlist-checks it, D9); the
    /// server refuses bodies over its single-request threshold — that refusal is the
    /// client's cue to take [`BlobClient::begin`]/[`BlobClient::append`] instead.
    pub async fn commit(
        &self,
        home_table: &str,
        home_id: Uuid,
        content_type: &str,
        filename: &str,
        bytes: Vec<u8>,
    ) -> Result<BlobCommitResponse> {
        let token = self.http.resolve_token()?;
        let form = reqwest::multipart::Form::new()
            .text("home_table", home_table.to_string())
            .text("home_id", home_id.to_string())
            .part(
                "file",
                reqwest::multipart::Part::bytes(bytes)
                    .file_name(filename.to_string())
                    .mime_str(content_type)
                    .map_err(|e| {
                        crate::error::ClientError::Other(format!(
                            "invalid content type {content_type:?}: {e}"
                        ))
                    })?,
            );
        let op = &ops::COMMIT_BLOB;
        let path = op.path(&[]);
        let req = self.http.request(op, &path).multipart(form);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET /api/blobs/{id} — the raw streaming response. The caller consumes
    /// `Response::bytes()` (whole) or `chunk()`/`bytes_stream()` (piped to a file);
    /// content type, length, and `Cache-Control: private, immutable` ride the headers.
    pub async fn read_response(&self, blob_id: Uuid) -> Result<reqwest::Response> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_BLOB;
        let path = op.path(&[&blob_id]);
        let req = self.http.request(op, &path);
        self.http.send(&op.method(), &path, req, Some(&token)).await
    }

    /// POST /api/blobs/uploads — begin a segmented upload (declare home + media type).
    pub async fn begin(&self, request: &BlobUploadBeginRequest) -> Result<BlobUploadBeginResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::BEGIN_BLOB_UPLOAD;
        let path = op.path(&[]);
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// POST /api/blobs/uploads/{id}/segments — append one RAW segment. The segment's
    /// identity is the SERVER's own sha256 of the exact bytes received: re-sending the
    /// same segment at the same seq is a no-op; a different one is a 409 — occupied
    /// seqs are never superseded.
    pub async fn append(
        &self,
        upload_id: Uuid,
        seq: u32,
        bytes: Vec<u8>,
    ) -> Result<BlobUploadProgress> {
        let token = self.http.resolve_token()?;
        let op = &ops::APPEND_BLOB_SEGMENT;
        let path = format!("{}?seq={seq}", op.path(&[&upload_id]));
        let req = self.http.request(op, &path).body(bytes);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET /api/blobs/uploads/{id} — the currently-landed segment set (the resume read).
    pub async fn progress(&self, upload_id: Uuid) -> Result<BlobUploadProgress> {
        let token = self.http.resolve_token()?;
        let op = &ops::BLOB_UPLOAD_PROGRESS;
        let path = op.path(&[&upload_id]);
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// POST /api/blobs/uploads/{id}/finalize — assemble, hash, commit. The request's
    /// expected values are CONCURRENCY tokens the server handed over — echo them
    /// verbatim; a mismatch refuses with the staging kept (resumable).
    pub async fn finalize(
        &self,
        upload_id: Uuid,
        request: &BlobUploadFinalizeRequest,
    ) -> Result<BlobCommitResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::FINALIZE_BLOB_UPLOAD;
        let path = op.path(&[&upload_id]);
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET /api/blobs — list the caller's readable blobs, optionally scoped to one home
    /// anchor (`home_table`/`home_id` as a pair).
    pub async fn list(&self, home: Option<(&str, Uuid)>) -> Result<Vec<BlobSummary>> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_BLOBS;
        let mut path = op.path(&[]);
        if let Some((table, id)) = home {
            path.push_str(&format!("?home_table={table}&home_id={id}"));
        }
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// POST /api/blobs/{id}/relations — assert one relation between the blob and a peer.
    pub async fn relate(
        &self,
        blob_id: Uuid,
        request: &BlobRelationAssertRequest,
    ) -> Result<BlobRelationAck> {
        let token = self.http.resolve_token()?;
        let op = &ops::RELATE_BLOB;
        let path = op.path(&[&blob_id]);
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// DELETE /api/blobs/{id} — release the caller's hold on a blob. Per-act authorship rides the
    /// query string (DELETE carries no body), as on [`ResourceClient::delete`](crate::resources::ResourceClient::delete);
    /// an empty [`ActInput`] appends nothing. `released` reports whether this call released it.
    pub async fn delete(&self, blob_id: Uuid, act: &ActInput) -> Result<BlobDeleteAck> {
        let token = self.http.resolve_token()?;
        let op = &ops::DELETE_BLOB;
        let path = op.path(&[&blob_id]);
        let req = self.http.request(op, &path).query(act);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET /api/blobs/{id}/relations — the edges incident to the blob.
    pub async fn relations(&self, blob_id: Uuid) -> Result<Vec<BlobRelationRow>> {
        let token = self.http.resolve_token()?;
        let op = &ops::BLOB_RELATIONS;
        let path = op.path(&[&blob_id]);
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }
}
