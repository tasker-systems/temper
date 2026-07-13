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
    if chunks.len() != embeddings.len() {
        return Err(crate::error::EmbedError::Embedding(format!(
            "chunk/embedding count mismatch: {} chunks, {} embeddings",
            chunks.len(),
            embeddings.len()
        )));
    }
    let packed: Vec<PackedChunk> = chunks
        .into_iter()
        .zip(embeddings)
        .map(|(chunk, embedding)| PackedChunk {
            chunk_index: chunk.chunk_index,
            header_path: chunk.header_path,
            heading_depth: chunk.heading_depth,
            content: chunk.content,
            content_hash: chunk.content_hash,
            embedding,
            // Declare the model we just embedded with. The server stores these vectors verbatim, so
            // this is the ONLY way it can know their provenance — and `embed_texts` has already
            // verified that the loaded model's sha256 IS this constant, so the claim is not a guess.
            embedded_with: Some(crate::embed::EXPECTED_MODEL_SHA256.to_owned()),
        })
        .collect();
    Ok(packed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "test-embed")]
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
    #[cfg(feature = "test-embed")]
    fn prepare_markdown_preserves_chunk_order() {
        let content = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";
        let chunks = prepare_markdown(content).expect("should succeed");
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index as usize, i, "chunks indexed in order");
        }
    }

    #[test]
    #[cfg(feature = "test-embed")]
    fn prepare_markdown_handles_single_oversized_line() {
        // Regression for issue #316: a document whose body is one long
        // unwrapped line (wide table row / rejoined prose) used to escape the
        // chunk size budget unsplit and hang the embed path indefinitely.
        let line = "wage $42.17 rate 1.5x overtime | ".repeat(400); // ~13k chars, zero '\n'
        let content = format!("# Contract\n\n{line}");
        let chunks = prepare_markdown(&content).expect("should embed without hanging");

        assert!(chunks.len() > 1, "oversized line should be split");
        for chunk in &chunks {
            assert!(
                chunk.content.len() <= crate::chunk::MAX_CHARS,
                "chunk {} exceeds budget: {} chars",
                chunk.chunk_index,
                chunk.content.len()
            );
            assert_eq!(chunk.embedding.len(), 768);
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
