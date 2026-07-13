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

/// Matches a markdown ATX heading line (`#`..`######`). `pub(crate)` so
/// `stream.rs`'s streaming segmenter can detect heading-boundary cut points
/// with the exact same rule the whole-document chunker uses.
pub(crate) fn heading_re() -> &'static Regex {
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

/// `pub(crate)` so `merkle.rs`'s block/resource merkle helpers reuse the exact same
/// hex-sha256 primitive as chunk `content_hash` computation.
pub(crate) fn sha256_hex(s: &str) -> String {
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
pub(crate) const MAX_CHARS: usize = (MAX_TOKENS as f64 * CHARS_PER_TOKEN) as usize; // ~1428

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

/// Collect lines into heading-delimited sections, seeding the breadcrumb stack from
/// `initial_stack` (empty for a plain whole-document scan). Used by
/// [`chunk_markdown_with_prefix`] — for an empty `initial_stack` this is exactly what the
/// pre-streaming `collect_sections` did, so `chunk_markdown`'s behavior (which now delegates
/// to `chunk_markdown_with_prefix(text, &[])`) is unchanged.
fn collect_sections_with_stack(text: &str, initial_stack: Vec<(usize, String)>) -> Vec<RawSection> {
    let re = heading_re();
    let lines: Vec<&str> = text.split('\n').collect();

    let mut sections: Vec<RawSection> = Vec::new();
    let mut header_stack: Vec<(usize, String)> = initial_stack;
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
/// A single line that is itself over budget is hard-split at char boundaries
/// rather than packed whole — one long unwrapped line must not escape the
/// size guarantee (issue #316).
fn split_by_lines(text: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut current_len: usize = 0;

    for line in text.split('\n') {
        if line.len() > MAX_CHARS {
            if !current.is_empty() {
                let content = current.join("\n");
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    chunks.push(trimmed.to_string());
                }
                current.clear();
                current_len = 0;
            }
            chunks.extend(hard_split_line(line));
            continue;
        }

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

/// Split a single newline-free line into pieces of at most `MAX_CHARS` bytes,
/// breaking only at char boundaries.
fn hard_split_line(line: &str) -> Vec<String> {
    let mut pieces: Vec<String> = Vec::new();
    let mut start = 0;
    while start < line.len() {
        let mut end = (start + MAX_CHARS).min(line.len());
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        let piece = line[start..end].trim();
        if !piece.is_empty() {
            pieces.push(piece.to_string());
        }
        start = end;
    }
    pieces
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
/// `Vec`. Equivalent to [`chunk_markdown_with_prefix`] with an empty prefix.
pub fn chunk_markdown(text: &str) -> Vec<ChunkData> {
    chunk_markdown_with_prefix(text, &[])
}

/// Split `text` into semantically coherent, token-limited chunks, seeding the heading
/// breadcrumb stack from `initial_breadcrumb` before scanning `text`'s own headings.
///
/// Used by the streaming ingest segmenter (`crate::stream::segment_reader`): a segment that
/// begins mid-section (no heading of its own at the top) still needs its ancestor
/// `header_path` — the running heading stack scanned across prior segments is threaded in
/// here as `initial_breadcrumb`.
///
/// Each `initial_breadcrumb` entry is seeded at its **positional** level: the outermost ancestor
/// at level 1, the next at 2, and so on. The pop-on-same-or-higher-level rule
/// (`while stack.last().level >= new.level`) then treats the prefix exactly as a whole-document
/// scan would, so a segment beginning with `## Usage` under the prefix `Manual > Setup` pops
/// `Setup` and yields `Manual > Usage` — not `Manual > Setup > Usage`.
///
/// Seeding every entry at level 0 (the original approach) made prefix entries unpoppable, so a
/// segment's headings nested under the *whole* prefix forever. That was documented as an
/// approximation confined to the size-fallback case; it was not. A heading-aligned cut — the
/// common case — leaves the previous section's heading in the prefix, and a sibling heading in the
/// next segment must pop it. The e2e equivalence assertion (a segmented ingest must produce the
/// same `header_path` set as a one-shot create) is what surfaced this.
///
/// The remaining approximation is narrow: an ancestor path that **skips heading levels** (`#` then
/// `####`) is seeded as though it descended 1, 2, …, so a following heading whose level falls
/// between the real and positional depths pops a different number of ancestors. `header_path`
/// carries titles only, so real ancestor depths are not recoverable from it; recording them would
/// mean widening the breadcrumb (and `kb_chunks`) to carry per-ancestor levels.
///
/// With an empty `initial_breadcrumb` this is byte-identical to [`chunk_markdown`] — the
/// empty-prefix case is a golden-equivalence invariant (`chunk_markdown` delegates here).
pub fn chunk_markdown_with_prefix(text: &str, initial_breadcrumb: &[String]) -> Vec<ChunkData> {
    if text.trim().is_empty() {
        return vec![];
    }

    let initial_stack: Vec<(usize, String)> = initial_breadcrumb
        .iter()
        .enumerate()
        .map(|(i, title)| (i + 1, title.clone()))
        .collect();
    let sections = collect_sections_with_stack(text, initial_stack);
    let mut chunks: Vec<ChunkData> = Vec::new();
    let mut chunk_index: u32 = 0;

    for section in &sections {
        // Try emphasis sub-splitting first (only activates if oversized).
        let sub_groups = split_at_emphasis_subheadings(&section.lines);

        // `heading_depth` answers "do I BEGIN a section?", not "what section am I in?" —
        // that second question is `header_path`'s job, and every chunk of the section
        // carries it. Only the FIRST chunk of a section opens it; the rest are
        // continuations and carry depth 0 (bare prose).
        //
        // This is the contract `temper_substrate::content::map_heading` documents and that
        // `reconstruct_body` reads: it re-emits a `## Heading` line exactly when depth is
        // non-zero. Stamping the section's depth onto EVERY chunk — as this did — made a
        // size-split section re-emit its heading at every internal chunk boundary, which is
        // the long-standing "show duplicates a line" bug: +12,990 bytes of duplicated
        // headings on one round-trip of a 1.2 MB document.
        //
        // Note the segment-boundary case already behaved this way: a segment cut mid-section
        // yields depth 0 with an inherited ancestor path. This makes the size-split case
        // agree with it.
        let mut section_opened = false;

        for group in &sub_groups {
            let group_chunks = split_oversized(group);

            for content in group_chunks {
                let trimmed = content.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let heading_depth = if section_opened {
                    0
                } else {
                    section_opened = true;
                    section.heading_depth
                };
                chunks.push(ChunkData {
                    chunk_index,
                    header_path: section.header_path.clone(),
                    heading_depth,
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
mod heading_depth_contract_tests {
    use super::*;

    /// **`heading_depth` means "I BEGIN a section", not "I am inside one".**
    ///
    /// A section too long for one chunk splits into several. Only the first opens the
    /// section; the rest are continuations and must carry depth 0, while all of them keep
    /// the `header_path` (that is the field that answers "what section am I in?").
    ///
    /// Stamping the section's depth onto every chunk made `reconstruct_body` re-emit the
    /// heading at every internal chunk boundary — the long-standing "show duplicates a
    /// line" bug. This test FAILS on the old chunker (every chunk reported depth 2), which
    /// is what makes it a regression test.
    #[test]
    fn only_the_first_chunk_of_a_split_section_opens_it() {
        // One H2 whose body is far past the chunk budget, so it must split.
        let body = "sentence with enough words to take up room ".repeat(200);
        let doc = format!("## Goals\n\n{body}");
        let chunks = chunk_markdown(&doc);

        assert!(
            chunks.len() > 1,
            "test needs a section that actually splits; got {} chunk(s)",
            chunks.len()
        );
        assert_eq!(
            chunks[0].heading_depth, 2,
            "the first chunk of the section opens it and carries its depth"
        );
        for (i, c) in chunks.iter().enumerate().skip(1) {
            assert_eq!(
                c.heading_depth, 0,
                "chunk {i} is a continuation and must NOT re-open the section"
            );
        }
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(
                c.header_path, "Goals",
                "chunk {i} still records which section it belongs to"
            );
        }
    }

    /// The fix must not silence genuine headings: consecutive short sections each open.
    #[test]
    fn each_short_section_opens_its_own_heading() {
        let chunks =
            chunk_markdown("## One\n\nAlpha.\n\n## Two\n\nBravo.\n\n### Three\n\nCharlie.");
        let depths: Vec<u8> = chunks.iter().map(|c| c.heading_depth).collect();
        assert_eq!(
            depths,
            vec![2, 2, 3],
            "three distinct sections ⇒ three opened headings"
        );
    }

    /// Two adjacent sections with the SAME title each still open — the case a read-side-only
    /// dedupe cannot get right, and the reason the real fix belongs in the chunker.
    #[test]
    fn two_adjacent_identically_titled_sections_both_open() {
        let chunks = chunk_markdown("## Notes\n\nFirst.\n\n## Notes\n\nSecond.");
        assert_eq!(chunks.len(), 2);
        assert_eq!(
            (chunks[0].heading_depth, chunks[1].heading_depth),
            (2, 2),
            "identically-titled siblings are distinct sections; both must open"
        );
    }
}

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
    fn hard_splits_single_oversized_line() {
        // One line with no internal newline, far over MAX_CHARS — the
        // unwrapped-paragraph / wide-table-row shape from issue #316.
        let line = "cell ".repeat(2000); // ~10,000 chars, zero '\n'
        let input = format!("# Wide\n\n{line}");
        let chunks = chunk_markdown(&input);

        assert!(
            chunks.len() > 1,
            "expected the oversized line to be split, got {} chunk(s)",
            chunks.len()
        );
        for chunk in &chunks {
            assert!(
                chunk.content.len() <= MAX_CHARS,
                "chunk {} exceeds budget: {} chars",
                chunk.chunk_index,
                chunk.content.len()
            );
        }
    }

    #[test]
    fn hard_split_respects_utf8_char_boundaries() {
        // Multi-byte chars with no whitespace: a byte-index split would panic.
        let line = "é".repeat(3000); // 6,000 bytes, zero '\n'
        let chunks = chunk_markdown(&line);

        assert!(chunks.len() > 1, "expected a split, got {}", chunks.len());
        for chunk in &chunks {
            assert!(
                chunk.content.len() <= MAX_CHARS,
                "chunk {} exceeds budget: {} chars",
                chunk.chunk_index,
                chunk.content.len()
            );
        }
        // No content is lost: pieces rejoin to the original line.
        let rejoined: String = chunks.iter().map(|c| c.content.as_str()).collect();
        assert_eq!(rejoined, line);
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

    // --- chunk_markdown_with_prefix (streaming segment breadcrumb carry-over) ---

    #[test]
    fn prefix_breadcrumb_pops_siblings_and_nests_descendants() {
        // Prefix "A > B" means A is level 1 and B is level 2 (their positions in the path).
        //
        // A sibling of B pops it. Previously this asserted "A > B > C" — a path a whole-document
        // scan can never produce, since `## C` at level 2 cannot nest under a level-2 `B`. The old
        // expectation encoded the level-0 seeding bug.
        let sibling = super::chunk_markdown_with_prefix("## C\n\nbody", &["A".into(), "B".into()]);
        assert_eq!(sibling[0].header_path, "A > C");

        // A descendant of B nests under it.
        let child = super::chunk_markdown_with_prefix("### C\n\nbody", &["A".into(), "B".into()]);
        assert_eq!(child[0].header_path, "A > B > C");

        // Prose with no heading of its own inherits the whole prefix.
        let prose = super::chunk_markdown_with_prefix("body", &["A".into(), "B".into()]);
        assert_eq!(prose[0].header_path, "A > B");
    }

    #[test]
    fn empty_prefix_equals_plain_chunk_markdown() {
        let text = "# H\n\npara one\n\n## H2\n\npara two";
        assert_eq!(
            super::chunk_markdown_with_prefix(text, &[]),
            super::chunk_markdown(text)
        );
    }

    // --- estimate_tokens ---

    #[test]
    fn estimate_tokens_reasonable() {
        // "hello world" = 11 chars → ~3 tokens at 3.5 c/t
        let tokens = estimate_tokens("hello world");
        assert!((2..=5).contains(&tokens), "got {tokens}");
    }

    // Splitting a document at heading boundaries and chunking each segment with the running
    // breadcrumb must reproduce whole-document chunking exactly. This is the property the whole
    // streaming/segmented ingest design rests on, and the one a level-0 prefix seed broke: a
    // sibling heading in the next segment could not pop the previous section's heading, so paths
    // grew without bound (`Manual > Setup > Usage > Caveats`).
    #[test]
    fn segmented_chunking_reproduces_whole_document_header_paths() {
        let segments = [
            "# Manual\n\nIntro line.\n\n## Setup\n\nInstall it.\n",
            "## Usage\n\nRun it.\n\n## Caveats\n\nMind the gap.\n",
            "## Appendix\n\nReferences follow.\n",
        ];
        let whole: String = segments.concat();

        let expected: Vec<(String, u8)> = chunk_markdown(&whole)
            .into_iter()
            .map(|c| (c.header_path, c.heading_depth))
            .collect();

        // Walk the segments the way the streaming segmenter does: carry the trailing breadcrumb.
        let mut actual: Vec<(String, u8)> = Vec::new();
        let mut breadcrumb: Vec<String> = Vec::new();
        for segment in segments {
            let chunks = chunk_markdown_with_prefix(segment, &breadcrumb);
            if let Some(last) = chunks.last() {
                breadcrumb = if last.header_path.is_empty() {
                    Vec::new()
                } else {
                    last.header_path.split(" > ").map(str::to_owned).collect()
                };
            }
            actual.extend(chunks.into_iter().map(|c| (c.header_path, c.heading_depth)));
        }

        assert_eq!(actual, expected);
    }

    // A segment cut mid-section (no heading of its own) inherits the full ancestor path, and its
    // innermost ancestor's depth — exactly what a whole-document scan gives the continuation chunk
    // of an oversized section, which is the same situation seen from the other side.
    #[test]
    fn a_mid_section_segment_inherits_its_ancestors() {
        let chunks = chunk_markdown_with_prefix(
            "beta continues here\n",
            &["Manual".to_owned(), "Setup".to_owned()],
        );
        assert_eq!(chunks[0].header_path, "Manual > Setup");
        assert_eq!(chunks[0].heading_depth, 2);
    }
}
