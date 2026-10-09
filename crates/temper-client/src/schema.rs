//! Typed sub-client for the `/api/schema` reads: the document types' JSON Schemas and the
//! open-meta convention, as the server describes them.

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::schema::{DocTypeDescription, DocTypeSummary, OpenMetaConvention};

/// Sub-client for schema reads.
pub struct SchemaClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for SchemaClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemaClient").finish_non_exhaustive()
    }
}

impl<'a> SchemaClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// GET /api/schema/doc-types — every document type, with whether it has a schema and its
    /// required fields.
    pub async fn list_doc_types(&self) -> Result<Vec<DocTypeSummary>> {
        self.read(&ops::LIST_DOC_TYPES, &[]).await
    }

    /// GET /api/schema/doc-types/{name} — one document type's JSON Schema, required fields,
    /// closed vocabularies, and an example managed tier.
    pub async fn describe_doc_type(&self, name: &str) -> Result<DocTypeDescription> {
        self.read(&ops::DESCRIBE_DOC_TYPE, &[&name]).await
    }

    /// GET /api/schema/open-meta — the open-meta convention: its schema and the keys it
    /// discourages, with what to use instead.
    pub async fn describe_open_meta(&self) -> Result<OpenMetaConvention> {
        self.read(&ops::DESCRIBE_OPEN_META, &[]).await
    }

    async fn read<T: serde::de::DeserializeOwned>(
        &self,
        op: &ops::Op,
        args: &[&dyn std::fmt::Display],
    ) -> Result<T> {
        let token = self.http.resolve_token()?;
        let path = op.path(args)?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }
}
