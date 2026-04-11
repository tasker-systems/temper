//! Markdown-aware text chunker with token-limit enforcement.
//!
//! Splits markdown into semantically coherent chunks that respect the token
//! limit of the embedding model (bge-base-en-v1.5 = 512 tokens).
//!
//! ## Strategy
//!
//! 1. Scan lines for hierarchy markers:
//!    - `#`..`######` headings (primary boundaries)
//!    - `**bold**` or `*italic*` alone on a line (secondary, only when the
//!      enclosing section is oversized)
//! 2. Group lines into heading-delimited *sections*, maintaining a breadcrumb
//!    stack (e.g. `"Design > API > Auth"`).
//! 3. Estimate token count per section.  If within budget → emit as one chunk.
//!    If over budget → split at paragraph boundaries (`\n\n`), then at line
//!    boundaries, preferring to keep semantically coherent blocks together.
//! 4. Small trailing fragments from a split are kept as their own chunk rather
//!    than merged into the next section — a minor storage inefficiency is better
//!    than crossing semantic boundaries.

use regex::Regex;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

static HEADING_RE: OnceLock<Regex> = OnceLock::new();
static BOLD_LINE_RE: OnceLock<Regex> = OnceLock::new();

fn heading_re() -> &'static Regex {
    HEADING_RE.get_or_init(|| Regex::new(r"^(#{1,6})\s+(.+)$").expect("heading regex is valid"))
}

fn bold_line_re() -> &'static Regex {
    BOLD_LINE_RE.get_or_init(|| {
        // Matches lines that are *only* a bold or italic marker:
        //   **Something Here**   or   *Something Here*
        // Must be the entire trimmed line (no surrounding prose).
        Regex::new(r"^\s*\*{1,2}([^*]+)\*{1,2}\s*$").expect("bold-line regex is valid")
    })
}

fn sha256_hex(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Maximum tokens per chunk.  bge-base-en-v1.5 accepts 512 tokens including
/// special tokens (\[CLS\] + \[SEP\] = 2), so usable budget is 510.
const MAX_TOKENS: usize = 510;

/// Conservative chars-per-token ratio for mixed markdown.  The bge tokenizer
/// averages ~3.5–4.0 chars/token on prose, but markdown with code snippets,
/// URLs, and punctuation tokenizes at closer to 2.5–3.0.  We use 2.8 to
/// provide headroom against the model's hard 512-token limit.
const CHARS_PER_TOKEN: f64 = 2.8;

/// Approximate max characters that fit within the token budget.
const MAX_CHARS: usize = (MAX_TOKENS as f64 * CHARS_PER_TOKEN) as usize; // ~1785

/// Estimate token count from character length.
pub fn estimate_tokens(text: &str) -> usize {
    (text.len() as f64 / CHARS_PER_TOKEN).ceil() as usize
}

/// A single chunk produced by [`chunk_markdown`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkData {
    pub chunk_index: u32,
    /// Heading breadcrumb trail, e.g. `"Design > API > Auth"`.
    pub header_path: String,
    /// Depth of the innermost heading: 0 = no heading, 1 = `#`, 2 = `##`, etc.
    pub heading_depth: u8,
    pub content: String,
    /// Lowercase hex SHA-256 of `content.trim()`.
    pub content_hash: String,
}

// ---------------------------------------------------------------------------
// Internal: section collection
// ---------------------------------------------------------------------------

/// A heading-delimited section before token splitting.
struct RawSection {
    header_path: String,
    /// Depth of the innermost heading: 0 = no heading, 1 = `#`, 2 = `##`, etc.
    heading_depth: u8,
    /// Lines of body content (no heading line).
    lines: Vec<String>,
}

/// Collect lines into heading-delimited sections.
fn collect_sections(text: &str) -> Vec<RawSection> {
    let re = heading_re();
    let lines: Vec<&str> = text.split('\n').collect();

    let mut sections: Vec<RawSection> = Vec::new();
    let mut header_stack: Vec<(usize, String)> = Vec::new();
    let mut current_lines: Vec<String> = Vec::new();

    let flush = |stack: &[(usize, String)], lines: &mut Vec<String>, out: &mut Vec<RawSection>| {
        if lines.is_empty() {
            return;
        }
        let path = stack
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join(" > ");
        let depth = stack.last().map(|(l, _)| *l as u8).unwrap_or(0);
        out.push(RawSection {
            header_path: path,
            heading_depth: depth,
            lines: std::mem::take(lines),
        });
    };

    for line in &lines {
        if let Some(caps) = re.captures(line) {
            flush(&header_stack, &mut current_lines, &mut sections);

            let level = caps[1].len();
            let text = caps[2].trim().to_string();

            while header_stack
                .last()
                .map(|(l, _)| *l >= level)
                .unwrap_or(false)
            {
                header_stack.pop();
            }
            header_stack.push((level, text));
        } else {
            current_lines.push(line.to_string());
        }
    }

    flush(&header_stack, &mut current_lines, &mut sections);
    sections
}

// ---------------------------------------------------------------------------
// Internal: splitting oversized sections
// ---------------------------------------------------------------------------

/// Split a block of text into chunks that fit within `MAX_CHARS`.
///
/// Split hierarchy:
/// 1. Paragraph boundaries (`\n\n`)
/// 2. Bold/italic sub-headings alone on a line (`**...**` / `*...*`)
/// 3. Individual lines
///
/// Within each level, we accumulate greedily until adding the next unit would
/// exceed the budget, then flush.
fn split_oversized(lines: &[String]) -> Vec<String> {
    let full_text = lines.join("\n");
    let trimmed = full_text.trim();

    // Fast path: fits in one chunk.
    if trimmed.len() <= MAX_CHARS {
        if trimmed.is_empty() {
            return vec![];
        }
        return vec![trimmed.to_string()];
    }

    // Split into paragraphs first (double-newline boundaries).
    let paragraphs = split_paragraphs(lines);

    let mut chunks: Vec<String> = Vec::new();
    let mut accumulator: Vec<String> = Vec::new();
    let mut acc_len: usize = 0;

    for para in &paragraphs {
        let para_len = para.len() + if acc_len > 0 { 2 } else { 0 }; // +2 for \n\n join

        if acc_len > 0 && acc_len + para_len > MAX_CHARS {
            // Flush accumulator.
            let content = accumulator.join("\n\n");
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                chunks.push(trimmed.to_string());
            }
            accumulator.clear();
            acc_len = 0;
        }

        // If a single paragraph is still oversized, split it by lines.
        if para.len() > MAX_CHARS {
            // Flush anything accumulated first.
            if !accumulator.is_empty() {
                let content = accumulator.join("\n\n");
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    chunks.push(trimmed.to_string());
                }
                accumulator.clear();
                acc_len = 0;
            }

            let line_chunks = split_by_lines(para);
            chunks.extend(line_chunks);
        } else {
            accumulator.push(para.clone());
            acc_len += para_len;
        }
    }

    // Flush remainder.
    if !accumulator.is_empty() {
        let content = accumulator.join("\n\n");
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            chunks.push(trimmed.to_string());
        }
    }

    chunks
}

/// Group lines into paragraphs separated by blank lines.
fn split_paragraphs(lines: &[String]) -> Vec<String> {
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in lines {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(current.join("\n"));
                current.clear();
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current.join("\n"));
    }

    paragraphs
}

/// Last-resort split: accumulate individual lines until the budget is hit.
fn split_by_lines(text: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut current_len: usize = 0;

    for line in text.split('\n') {
        let line_len = line.len() + if current_len > 0 { 1 } else { 0 };

        if current_len > 0 && current_len + line_len > MAX_CHARS {
            let content = current.join("\n");
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                chunks.push(trimmed.to_string());
            }
            current.clear();
            current_len = 0;
        }

        current.push(line);
        current_len += line_len;
    }

    if !current.is_empty() {
        let content = current.join("\n");
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            chunks.push(trimmed.to_string());
        }
    }

    chunks
}

// ---------------------------------------------------------------------------
// Internal: bold/italic sub-section splitting
// ---------------------------------------------------------------------------

/// When a section is oversized and contains bold/italic lines that act as
/// sub-headings, split at those boundaries first before falling back to
/// paragraph/line splitting.
fn split_at_emphasis_subheadings(lines: &[String]) -> Vec<Vec<String>> {
    let re = bold_line_re();
    let full_text = lines.join("\n");

    // Only use emphasis splitting if the section is oversized AND has
    // emphasis-only lines.
    if full_text.len() <= MAX_CHARS || !lines.iter().any(|l| re.is_match(l)) {
        return vec![lines.to_vec()];
    }

    let mut groups: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();

    for line in lines {
        if re.is_match(line) && !current.is_empty() {
            groups.push(std::mem::take(&mut current));
        }
        current.push(line.clone());
    }
    if !current.is_empty() {
        groups.push(current);
    }

    groups
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Split `text` into semantically coherent, token-limited chunks.
///
/// Empty input or input that produces no non-empty chunks returns an empty
/// `Vec`.
pub fn chunk_markdown(text: &str) -> Vec<ChunkData> {
    if text.trim().is_empty() {
        return vec![];
    }

    let sections = collect_sections(text);
    let mut chunks: Vec<ChunkData> = Vec::new();
    let mut chunk_index: u32 = 0;

    for section in &sections {
        // Try emphasis sub-splitting first (only activates if oversized).
        let sub_groups = split_at_emphasis_subheadings(&section.lines);

        for group in &sub_groups {
            let group_chunks = split_oversized(group);

            for content in group_chunks {
                let trimmed = content.trim();
                if trimmed.is_empty() {
                    continue;
                }
                chunks.push(ChunkData {
                    chunk_index,
                    header_path: section.header_path.clone(),
                    heading_depth: section.heading_depth,
                    content: trimmed.to_string(),
                    content_hash: sha256_hex(trimmed),
                });
                chunk_index += 1;
            }
        }
    }

    chunks
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- Basic heading-delimited chunking (preserved from original) ---

    #[test]
    fn chunks_simple_document_by_headers() {
        let input =
            "# Title\n\nIntro.\n\n## Section One\n\nContent one.\n\n## Section Two\n\nContent two.\n";
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

        let chunks2 = chunk_markdown(input);
        assert_eq!(chunks[0].content_hash, chunks2[0].content_hash);

        let expected = sha256_hex(chunks[0].content.trim());
        assert_eq!(chunks[0].content_hash, expected);
        assert_eq!(chunks[0].content_hash.len(), 64);
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
        for chunk in &chunks {
            assert!(
                !chunk.content.trim().is_empty(),
                "chunk {} has empty content",
                chunk.chunk_index
            );
        }
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].header_path, "Second");
        assert_eq!(chunks[0].content, "Actual content.");
    }

    #[test]
    fn header_stack_pops_on_same_or_higher_level() {
        let input = "# A\n\n## B\n\n### C\n\nC content.\n\n## D\n\nD content.\n";
        let chunks = chunk_markdown(input);
        let d_chunk = chunks
            .iter()
            .find(|c| c.content == "D content.")
            .expect("should find D content chunk");
        assert_eq!(d_chunk.header_path, "A > D");
    }

    // --- Token-limit enforcement ---

    #[test]
    fn splits_oversized_section_at_paragraph_boundaries() {
        // Create a section with multiple paragraphs that together exceed MAX_CHARS.
        let para = "x".repeat(1000); // ~1000 chars = ~285 tokens
        let input = format!("# Big Section\n\n{para}\n\n{para}\n\n{para}");
        let chunks = chunk_markdown(&input);

        // Should produce multiple chunks, each within budget.
        assert!(
            chunks.len() > 1,
            "expected multiple chunks, got {}",
            chunks.len()
        );
        for chunk in &chunks {
            assert!(
                chunk.content.len() <= MAX_CHARS + 100, // small tolerance for join overhead
                "chunk {} exceeds limit: {} chars",
                chunk.chunk_index,
                chunk.content.len()
            );
        }
    }

    #[test]
    fn splits_oversized_section_at_line_boundaries() {
        // One giant paragraph with no blank lines — must split by lines.
        let line = "word ".repeat(100); // ~500 chars per line
        let lines: Vec<&str> = (0..10).map(|_| line.as_str()).collect();
        let big_para = lines.join("\n");
        let input = format!("# Huge\n\n{big_para}");
        let chunks = chunk_markdown(&input);

        assert!(
            chunks.len() > 1,
            "expected multiple chunks, got {}",
            chunks.len()
        );
        for chunk in &chunks {
            assert!(
                chunk.content.len() <= MAX_CHARS + 600, // line-granularity tolerance
                "chunk {} exceeds limit: {} chars",
                chunk.chunk_index,
                chunk.content.len()
            );
        }
    }

    #[test]
    fn small_sections_are_not_split() {
        let input = "# Small\n\nJust a sentence.\n";
        let chunks = chunk_markdown(input);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, "Just a sentence.");
    }

    #[test]
    fn emphasis_subheadings_split_oversized_sections() {
        // Create an oversized section with bold sub-headings.
        let block = "content line\n".repeat(100); // ~1300 chars
        let input = format!("# Main\n\n**Part One**\n\n{block}\n**Part Two**\n\n{block}");
        let chunks = chunk_markdown(&input);

        // Should have produced at least 2 chunks from the sub-heading split.
        assert!(
            chunks.len() >= 2,
            "expected >=2 chunks from emphasis split, got {}",
            chunks.len()
        );
    }

    #[test]
    fn all_chunks_have_sequential_indices() {
        let para = "x".repeat(1000);
        let input = format!("# A\n\n{para}\n\n{para}\n\n## B\n\nSmall.");
        let chunks = chunk_markdown(&input);
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(
                chunk.chunk_index, i as u32,
                "chunk indices should be sequential"
            );
        }
    }

    // --- estimate_tokens ---

    #[test]
    fn estimate_tokens_reasonable() {
        // "hello world" = 11 chars → ~3 tokens at 3.5 c/t
        let tokens = estimate_tokens("hello world");
        assert!((2..=5).contains(&tokens), "got {tokens}");
    }
}
