//! Shared markdown → packed chunks pipeline.
//!
//! Single source of truth for "turn raw markdown into indexed chunks."
//! Used by the CLI (client-side precomputed path) and by the API service
//! layer (server-side markdown path). Keeps both sides identical.

use crate::error::Result;
use crate::{chunk::chunk_markdown, embed::embed_texts};
use temper_core::types::ingest::PackedChunk;

/// Convert raw markdown content into packed chunks with embeddings.
///
/// Pipeline: chunk_markdown → embed_texts → zip into PackedChunk.
/// Returns an empty vec for empty/whitespace-only input.
pub fn prepare_markdown(content: &str) -> Result<Vec<PackedChunk>> {
    let chunks = chunk_markdown(content);
    if chunks.is_empty() {
        return Ok(Vec::new());
    }
    let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
    let embeddings = embed_texts(&texts)?;
    assert_eq!(
        chunks.len(),
        embeddings.len(),
        "chunk and embedding count must match"
    );
    let packed: Vec<PackedChunk> = chunks
        .into_iter()
        .zip(embeddings)
        .map(|(chunk, embedding)| PackedChunk {
            chunk_index: chunk.chunk_index,
            header_path: chunk.header_path,
            content: chunk.content,
            content_hash: chunk.content_hash,
            embedding,
        })
        .collect();
    Ok(packed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: These tests require the embed feature and ORT to be available.
    // They run on CI/Vercel, not locally on macOS without the static lib.

    #[test]
    fn prepare_markdown_produces_packed_chunks() {
        let content = "# Title\n\nThis is a paragraph.\n\n## Section\n\nMore text here.";
        let result = prepare_markdown(content);
        assert!(
            result.is_ok(),
            "prepare_markdown should succeed: {result:?}"
        );
        let chunks = result.unwrap();
        assert!(!chunks.is_empty(), "should produce at least one chunk");
        for chunk in &chunks {
            assert_eq!(
                chunk.embedding.len(),
                768,
                "each chunk has 768-dim embedding"
            );
            assert!(!chunk.content.is_empty(), "chunk content non-empty");
            assert!(!chunk.content_hash.is_empty(), "chunk has content_hash");
        }
    }

    #[test]
    fn prepare_markdown_preserves_chunk_order() {
        let content = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";
        let chunks = prepare_markdown(content).expect("should succeed");
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index as usize, i, "chunks indexed in order");
        }
    }

    #[test]
    fn prepare_markdown_empty_input() {
        let chunks = prepare_markdown("").expect("should succeed");
        assert!(chunks.is_empty());
    }

    #[test]
    fn prepare_markdown_whitespace_only() {
        let chunks = prepare_markdown("   \n\n  ").expect("should succeed");
        assert!(chunks.is_empty());
    }
}
