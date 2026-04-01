//! Markdown-aware text chunker.
//!
//! Splits markdown on headings, maintains a hierarchical header breadcrumb
//! stack, and SHA-256 hashes each chunk's trimmed content.
//!
//! Port of `packages/temper-cloud/src/processing/chunk.ts`.

use regex::Regex;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

static HEADING_RE: OnceLock<Regex> = OnceLock::new();

fn heading_re() -> &'static Regex {
    HEADING_RE.get_or_init(|| Regex::new(r"^(#{1,6})\s+(.+)$").expect("heading regex is valid"))
}

fn sha256_hex(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// A single chunk produced by [`chunk_markdown`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkData {
    pub chunk_index: u32,
    /// Heading breadcrumb trail, e.g. `"Design > API > Auth"`.
    pub header_path: String,
    pub content: String,
    /// Lowercase hex SHA-256 of `content.trim()`.
    pub content_hash: String,
}

/// Split `text` into heading-delimited chunks.
///
/// Empty input or input that produces no non-empty chunks returns an empty
/// `Vec`. The algorithm mirrors the TypeScript `chunkText` function exactly so
/// that content hashes are identical for the same input.
pub fn chunk_markdown(text: &str) -> Vec<ChunkData> {
    if text.trim().is_empty() {
        return vec![];
    }

    let re = heading_re();
    let lines: Vec<&str> = text.split('\n').collect();

    let mut chunks: Vec<ChunkData> = Vec::new();
    // Stack of (level, heading_text).
    let mut header_stack: Vec<(usize, String)> = Vec::new();
    let mut current_content: Vec<&str> = Vec::new();
    let mut chunk_index: u32 = 0;

    let flush = |header_stack: &Vec<(usize, String)>,
                 current_content: &mut Vec<&str>,
                 chunks: &mut Vec<ChunkData>,
                 chunk_index: &mut u32| {
        let content = current_content.join("\n");
        let trimmed = content.trim();
        if trimmed.is_empty() {
            current_content.clear();
            return;
        }
        let header_path = header_stack
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join(" > ");
        let content_hash = sha256_hex(trimmed);
        chunks.push(ChunkData {
            chunk_index: *chunk_index,
            header_path,
            content: trimmed.to_string(),
            content_hash,
        });
        *chunk_index += 1;
        current_content.clear();
    };

    for line in &lines {
        if let Some(caps) = re.captures(line) {
            flush(
                &header_stack,
                &mut current_content,
                &mut chunks,
                &mut chunk_index,
            );

            let level = caps[1].len();
            let text = caps[2].trim().to_string();

            // Pop headers at same or deeper level.
            while header_stack
                .last()
                .map(|(l, _)| *l >= level)
                .unwrap_or(false)
            {
                header_stack.pop();
            }
            header_stack.push((level, text));
        } else {
            current_content.push(line);
        }
    }

    flush(
        &header_stack,
        &mut current_content,
        &mut chunks,
        &mut chunk_index,
    );

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_simple_document_by_headers() {
        let input = "# Title\n\nIntro.\n\n## Section One\n\nContent one.\n\n## Section Two\n\nContent two.\n";
        let chunks = chunk_markdown(input);
        assert_eq!(chunks.len(), 3, "expected 3 chunks, got {}", chunks.len());

        assert_eq!(chunks[0].header_path, "Title");
        assert_eq!(chunks[0].content, "Intro.");

        assert_eq!(chunks[1].header_path, "Title > Section One");
        assert_eq!(chunks[1].content, "Content one.");

        assert_eq!(chunks[2].header_path, "Title > Section Two");
        assert_eq!(chunks[2].content, "Content two.");
    }

    #[test]
    fn produces_deterministic_content_hash() {
        let input = "# Hash Test\n\nHello, world.\n";
        let chunks = chunk_markdown(input);
        assert_eq!(chunks.len(), 1);

        // Run twice — must be identical.
        let chunks2 = chunk_markdown(input);
        assert_eq!(chunks[0].content_hash, chunks2[0].content_hash);

        // Verify hash manually.
        let expected = sha256_hex(chunks[0].content.trim());
        assert_eq!(chunks[0].content_hash, expected);
        // SHA-256 is 64 hex chars.
        assert_eq!(chunks[0].content_hash.len(), 64);
        // Must be lowercase hex only.
        assert!(chunks[0]
            .content_hash
            .chars()
            .all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn handles_text_with_no_headers_as_single_chunk() {
        let input = "Just some plain text.\nNo headings here.\n";
        let chunks = chunk_markdown(input);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].header_path, "");
        assert!(chunks[0].content.contains("plain text"));
    }

    #[test]
    fn handles_empty_text() {
        assert_eq!(chunk_markdown(""), vec![]);
        assert_eq!(chunk_markdown("   \n\n  "), vec![]);
    }

    #[test]
    fn handles_nested_headers() {
        let input = "# Top\n\n## Mid\n\n### Deep\n\nDeep content.\n";
        let chunks = chunk_markdown(input);
        // Only the deepest section has actual content.
        let deep = chunks
            .iter()
            .find(|c| c.content == "Deep content.")
            .expect("should find deep content chunk");
        assert_eq!(deep.header_path, "Top > Mid > Deep");
    }

    #[test]
    fn skips_empty_chunks() {
        let input = "# First\n\n# Second\n\nActual content.\n";
        let chunks = chunk_markdown(input);
        // Every chunk must have non-empty trimmed content.
        for chunk in &chunks {
            assert!(
                !chunk.content.trim().is_empty(),
                "chunk {} has empty content",
                chunk.chunk_index
            );
        }
        // Only one chunk should survive (the "Second" section with content).
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].header_path, "Second");
        assert_eq!(chunks[0].content, "Actual content.");
    }

    #[test]
    fn header_stack_pops_on_same_or_higher_level() {
        // # A → ## B → ### C → C content → ## D → D content
        let input = "# A\n\n## B\n\n### C\n\nC content.\n\n## D\n\nD content.\n";
        let chunks = chunk_markdown(input);

        let d_chunk = chunks
            .iter()
            .find(|c| c.content == "D content.")
            .expect("should find D content chunk");
        // ## D follows ### C — B and C should have been popped, leaving just A > D.
        assert_eq!(d_chunk.header_path, "A > D");
    }
}
