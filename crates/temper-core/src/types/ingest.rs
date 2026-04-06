//! Ingest API types — wire format for CLI → Axum ingest pipeline.

use serde::{Deserialize, Serialize};

/// Wire payload for POST /api/ingest — resource + pre-processed chunks.
///
/// The CLI performs extract → chunk → embed locally and sends everything
/// in a single request. `chunks_packed` is a base64-encoded MessagePack
/// blob containing `Vec<PackedChunk>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct IngestPayload {
    pub title: String,
    pub origin_uri: String,
    pub context_name: String,
    pub doc_type_name: String,
    /// `"sha256:<hex>"`
    pub content_hash: String,
    pub slug: String,
    /// Full extracted markdown content.
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// Managed frontmatter (temper-* fields) as JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<serde_json::Value>,
    /// Open frontmatter (user-owned fields) as JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<serde_json::Value>,
    /// Base64-encoded MessagePack of `Vec<PackedChunk>`.
    pub chunks_packed: String,
}

/// A single chunk with its embedding, serialized via MessagePack inside
/// `IngestPayload::chunks_packed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackedChunk {
    pub chunk_index: u32,
    pub header_path: String,
    pub content: String,
    pub content_hash: String,
    /// 768-dimensional embedding vector.
    pub embedding: Vec<f32>,
}

/// Format an embedding vector as a pgvector literal string: `[0.1,0.2,...]`
pub fn format_embedding(embedding: &[f32]) -> String {
    format!(
        "[{}]",
        embedding
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// JSONB-serializable chunk row for the `persist_resource_chunks()` and
/// `replace_resource_chunks()` SQL functions.
///
/// `embedding` is a pre-formatted pgvector literal string (`"[0.1,0.2,...]"`)
/// rather than a `Vec<f32>`. The SQL function casts it to `vector` via `::vector`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRowJsonb {
    pub chunk_index: u32,
    pub header_path: String,
    pub content: String,
    pub content_hash: String,
    /// Pre-formatted pgvector literal: `[0.1,0.2,...]`
    pub embedding: String,
}

impl ChunkRowJsonb {
    pub fn from_packed(chunk: &PackedChunk) -> Self {
        Self {
            chunk_index: chunk.chunk_index,
            header_path: chunk.header_path.clone(),
            content: chunk.content.clone(),
            content_hash: chunk.content_hash.clone(),
            embedding: format_embedding(&chunk.embedding),
        }
    }
}

/// Convert a slice of `PackedChunk` into a JSONB-ready `serde_json::Value`
/// array suitable for the batch chunk SQL functions.
pub fn chunks_to_jsonb(chunks: &[PackedChunk]) -> serde_json::Value {
    let rows: Vec<ChunkRowJsonb> = chunks.iter().map(ChunkRowJsonb::from_packed).collect();
    serde_json::to_value(&rows).expect("ChunkRowJsonb is always serializable")
}

/// Encode chunks into the `chunks_packed` wire format (MessagePack → base64).
pub fn pack_chunks(chunks: &[PackedChunk]) -> Result<String, PackError> {
    use base64::Engine;
    let bytes = rmp_serde::to_vec(chunks).map_err(PackError::Serialize)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

/// Decode `chunks_packed` from wire format (base64 → MessagePack).
pub fn unpack_chunks(packed: &str) -> Result<Vec<PackedChunk>, PackError> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(packed)
        .map_err(PackError::Base64)?;
    rmp_serde::from_slice(&bytes).map_err(PackError::Deserialize)
}

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("MessagePack serialization failed: {0}")]
    Serialize(rmp_serde::encode::Error),
    #[error("MessagePack deserialization failed: {0}")]
    Deserialize(rmp_serde::decode::Error),
    #[error("Base64 decode failed: {0}")]
    Base64(base64::DecodeError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_chunks() -> Vec<PackedChunk> {
        vec![
            PackedChunk {
                chunk_index: 0,
                header_path: "Title".to_owned(),
                content: "Hello world".to_owned(),
                content_hash: "abc123".to_owned(),
                embedding: vec![0.1; 768],
            },
            PackedChunk {
                chunk_index: 1,
                header_path: "Title > Section".to_owned(),
                content: "Section content".to_owned(),
                content_hash: "def456".to_owned(),
                embedding: vec![0.2; 768],
            },
        ]
    }

    #[test]
    fn pack_unpack_roundtrip() {
        let chunks = sample_chunks();
        let packed = pack_chunks(&chunks).unwrap();
        let unpacked = unpack_chunks(&packed).unwrap();

        assert_eq!(unpacked.len(), 2);
        assert_eq!(unpacked[0].chunk_index, 0);
        assert_eq!(unpacked[0].header_path, "Title");
        assert_eq!(unpacked[0].content, "Hello world");
        assert_eq!(unpacked[0].embedding.len(), 768);
        assert_eq!(unpacked[1].chunk_index, 1);
        assert_eq!(unpacked[1].header_path, "Title > Section");
    }

    #[test]
    fn pack_produces_valid_base64() {
        let packed = pack_chunks(&sample_chunks()).unwrap();
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(&packed)
            .unwrap();
    }

    #[test]
    fn unpack_invalid_base64_errors() {
        let result = unpack_chunks("not-valid-base64!!!");
        assert!(result.is_err());
    }

    #[test]
    fn payload_serialization_roundtrip() {
        let payload = IngestPayload {
            title: "Test".to_owned(),
            origin_uri: "kb://ctx/task/test".to_owned(),
            context_name: "ctx".to_owned(),
            doc_type_name: "task".to_owned(),
            content_hash: "sha256:abc".to_owned(),
            slug: "test".to_owned(),
            content: "# Test".to_owned(),
            metadata: None,
            managed_meta: Some(serde_json::json!({"temper-stage": "backlog"})),
            open_meta: Some(serde_json::json!({"tags": ["rust"]})),
            chunks_packed: pack_chunks(&sample_chunks()).unwrap(),
        };

        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: IngestPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.title, "Test");
        assert_eq!(deserialized.context_name, "ctx");
        assert_eq!(
            deserialized.managed_meta,
            Some(serde_json::json!({"temper-stage": "backlog"}))
        );
        assert_eq!(
            deserialized.open_meta,
            Some(serde_json::json!({"tags": ["rust"]}))
        );

        let chunks = unpack_chunks(&deserialized.chunks_packed).unwrap();
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn format_embedding_basic() {
        let result = format_embedding(&[0.1, 0.2, 0.3]);
        assert_eq!(result, "[0.1,0.2,0.3]");
    }

    #[test]
    fn format_embedding_empty() {
        assert_eq!(format_embedding(&[]), "[]");
    }

    #[test]
    fn chunk_row_jsonb_from_packed() {
        let packed = PackedChunk {
            chunk_index: 0,
            header_path: "Title > Section".to_owned(),
            content: "Hello world".to_owned(),
            content_hash: "abc123".to_owned(),
            embedding: vec![0.1, 0.2, 0.3],
        };
        let row = ChunkRowJsonb::from_packed(&packed);
        assert_eq!(row.chunk_index, 0);
        assert_eq!(row.embedding, "[0.1,0.2,0.3]");
        assert_eq!(row.content, "Hello world");
    }

    #[test]
    fn chunks_to_jsonb_produces_array() {
        let chunks = vec![
            PackedChunk {
                chunk_index: 0,
                header_path: "A".into(),
                content: "first".into(),
                content_hash: "h1".into(),
                embedding: vec![0.1, 0.2],
            },
            PackedChunk {
                chunk_index: 1,
                header_path: "B".into(),
                content: "second".into(),
                content_hash: "h2".into(),
                embedding: vec![0.3, 0.4],
            },
        ];
        let json = chunks_to_jsonb(&chunks);
        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 2);
        assert_eq!(json[0]["chunk_index"], 0);
        assert_eq!(json[0]["embedding"], "[0.1,0.2]");
        assert!(json[0]["embedding"].is_string());
    }
}
