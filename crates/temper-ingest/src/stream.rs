//! Bounded-memory streaming segmentation for large markdown bodies.
//!
//! [`segment_reader`] reads a `BufRead` source line-by-line (never `read_to_string`s the
//! whole body) and emits [`Segment`]s of at most `budget` bytes, preferring to cut at a
//! heading boundary and never splitting a line. Each segment carries the running heading
//! stack — scanned across the whole document, not reset per segment — into the next
//! segment's `initial_breadcrumb`, so [`crate::chunk::chunk_markdown_with_prefix`] can
//! reconstruct full ancestor `header_path`s for content that begins mid-section.
//!
//! **The reader does not normalize.** It reads with [`BufRead::read_line`], which *retains*
//! each line's terminator (`\n`, `\r\n`, or none at EOF), so concatenating the segments in
//! `seq` order reproduces the source **byte-for-byte** — CRLF, blank lines, and a missing
//! trailing newline all survive. Heading detection runs against the trimmed line; only the
//! emitted bytes are left untouched. (Before this, `BufRead::lines()` + `join("\n")` silently
//! collapsed CRLF to LF and dropped the trailing newline.)

use std::io::{self, BufRead};

use crate::chunk::heading_re;

/// One streamed slice of a source body, ready for
/// [`crate::chunk::chunk_markdown_with_prefix`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub seq: u32,
    pub text: String,
    /// The heading-stack breadcrumb (title-only, outermost first) carried in from headings
    /// scanned in prior segments — the ancestor path this segment's own headings nest under.
    pub initial_breadcrumb: Vec<String>,
}

/// Default segment (block) budget in bytes of text — also the one-shot/segmented ingest
/// threshold (`temper_cli::actions::ingest::ingest_mode`): a body at or under this size takes
/// the existing single-block create path; a larger body segments.
pub const SEGMENT_BUDGET_BYTES: usize = 262_144;

/// Read `src` line-by-line, emitting segments of at most `budget` bytes (never splitting a
/// line), preferring to cut right before a heading line once the current segment already
/// holds at least half the budget. Peak memory is one segment's accumulated text — the
/// source is never materialized in full.
pub fn segment_reader<R: BufRead>(
    src: R,
    budget: usize,
) -> impl Iterator<Item = io::Result<Segment>> {
    SegmentReader {
        src,
        budget,
        seq: 0,
        header_stack: Vec::new(),
        buffer: String::new(),
        segment_start_breadcrumb: Vec::new(),
        done: false,
    }
}

struct SegmentReader<R: BufRead> {
    src: R,
    budget: usize,
    seq: u32,
    /// Heading stack scanned across the *whole* document (never reset per segment) —
    /// `(level, title)` pairs, mirroring `chunk::collect_sections_with_stack`'s pop-on-
    /// same-or-higher-level rule.
    header_stack: Vec<(usize, String)>,
    /// The in-progress segment's *raw* bytes — each line appended verbatim, terminator and
    /// all. Its `len()` is the running budget measure.
    buffer: String,
    /// The breadcrumb snapshot for the segment currently being accumulated in `buffer`.
    segment_start_breadcrumb: Vec<String>,
    done: bool,
}

impl<R: BufRead> SegmentReader<R> {
    fn breadcrumb(&self) -> Vec<String> {
        self.header_stack.iter().map(|(_, t)| t.clone()).collect()
    }

    /// Scan `line` (already trimmed of its terminator); if it's an ATX heading, pop/push the
    /// running stack exactly like `chunk::collect_sections_with_stack` does.
    fn scan_heading(&mut self, line: &str) {
        let Some(caps) = heading_re().captures(line) else {
            return;
        };
        let level = caps[1].len();
        let title = caps[2].trim().to_string();
        while self
            .header_stack
            .last()
            .map(|(l, _)| *l >= level)
            .unwrap_or(false)
        {
            self.header_stack.pop();
        }
        self.header_stack.push((level, title));
    }

    /// Append `line` — terminator and all — to the in-progress segment buffer, updating the
    /// running heading stack from the trimmed line. The bytes are never mutated.
    fn append_line(&mut self, line: String) {
        self.scan_heading(line.trim_end());
        self.buffer.push_str(&line);
    }

    /// Start a fresh segment buffer with `line` as its first line, snapshotting the current
    /// heading stack as that segment's `initial_breadcrumb` before `line` (if a heading)
    /// mutates the stack further.
    fn begin_segment_with(&mut self, line: String) {
        self.segment_start_breadcrumb = self.breadcrumb();
        self.append_line(line);
    }

    /// Emit the current buffer as a `Segment` and reset for the next one. `None` if the
    /// buffer is empty (nothing accumulated yet).
    fn flush(&mut self) -> Option<Segment> {
        if self.buffer.is_empty() {
            return None;
        }
        let text = std::mem::take(&mut self.buffer);
        let initial_breadcrumb = std::mem::take(&mut self.segment_start_breadcrumb);
        let seg = Segment {
            seq: self.seq,
            text,
            initial_breadcrumb,
        };
        self.seq += 1;
        Some(seg)
    }
}

impl<R: BufRead> Iterator for SegmentReader<R> {
    type Item = io::Result<Segment>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            let mut line = String::new();
            match self.src.read_line(&mut line) {
                Ok(0) => {
                    self.done = true;
                    return self.flush().map(Ok);
                }
                Ok(_) => {}
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
            }

            // Heading detection ignores the retained terminator; budget accounting counts it.
            let is_heading = heading_re().is_match(line.trim_end());
            let buffer_len = self.buffer.len();
            let would_exceed_budget = buffer_len > 0 && buffer_len + line.len() > self.budget;
            // Prefer to cut right before a heading once the buffer already holds at least
            // half the budget — a cleaner boundary than waiting for the hard budget cut,
            // without inventing an untested precise threshold.
            let prefers_heading_cut = is_heading && buffer_len > 0 && buffer_len * 2 >= self.budget;

            if (would_exceed_budget || prefers_heading_cut) && !self.buffer.is_empty() {
                if let Some(seg) = self.flush() {
                    self.begin_segment_with(line);
                    return Some(Ok(seg));
                }
            }

            self.append_line(line);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn segments_are_bounded_and_seq_ordered() {
        let doc = "# A\n".to_string() + &"x\n".repeat(200_000); // ~400 KB
        let segs: Vec<_> = super::segment_reader(std::io::Cursor::new(doc), 262_144)
            .map(|r| r.unwrap())
            .collect();
        assert!(segs.len() >= 2, "large doc splits");
        for (i, s) in segs.iter().enumerate() {
            assert_eq!(s.seq as usize, i);
            assert!(s.text.len() <= 262_144 + 4096, "each segment near-budget");
        }
    }

    #[test]
    fn small_doc_is_one_segment() {
        let segs: Vec<_> =
            super::segment_reader(std::io::Cursor::new("# H\n\nshort".to_string()), 262_144)
                .map(|r| r.unwrap())
                .collect();
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].seq, 0);
    }

    #[test]
    fn never_splits_a_line_and_reassembles_verbatim() {
        let doc = (0..1000).map(|i| format!("line {i}\n")).collect::<String>();
        let segs: Vec<_> = super::segment_reader(std::io::Cursor::new(doc.clone()), 512)
            .map(|r| r.unwrap())
            .collect();
        assert!(
            segs.len() > 1,
            "expected multiple segments at a tiny budget"
        );
        // Each segment's text retains its own line terminators, so segments concatenate
        // (no join separator) back to the exact source bytes.
        let reassembled: String = segs.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(
            reassembled, doc,
            "segments rejoin to the original source verbatim"
        );
    }

    #[test]
    fn segments_reassemble_byte_exactly() {
        // The segmenter must not normalize: CRLF, a missing trailing newline, blank lines,
        // and multibyte UTF-8 all survive a segment→rejoin round-trip unchanged.
        for doc in [
            "# T\n\nalpha\nbeta\n",         // trailing newline
            "# T\n\nalpha\nbeta",           // NO trailing newline
            "# T\r\n\r\nalpha\r\nbeta\r\n", // CRLF
            "# T\n\nnaïve — ünïcode ✅\n",  // multibyte
        ] {
            let segs: Vec<_> = super::segment_reader(std::io::Cursor::new(doc), 16)
                .map(|r| r.unwrap())
                .collect();
            let rejoined: String = segs.iter().map(|s| s.text.as_str()).collect();
            assert_eq!(rejoined, doc, "segments must rejoin byte-exactly: {doc:?}");
        }
    }

    #[test]
    fn same_source_and_budget_are_deterministic() {
        let doc = "# A\n".to_string() + &"line of body text\n".repeat(20_000);
        let first: Vec<_> = super::segment_reader(std::io::Cursor::new(doc.clone()), 8192)
            .map(|r| r.unwrap())
            .collect();
        let second: Vec<_> = super::segment_reader(std::io::Cursor::new(doc), 8192)
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            first, second,
            "identical source+budget must re-derive identical segments"
        );
    }

    #[test]
    fn breadcrumb_carries_the_running_heading_stack_into_the_next_segment() {
        // Force a mid-section cut: one heading followed by enough body text to blow the
        // (tiny) budget without a second heading to prefer-cut at.
        let doc = "# Top\n\n".to_string() + &"filler line\n".repeat(200);
        let segs: Vec<_> = super::segment_reader(std::io::Cursor::new(doc), 256)
            .map(|r| r.unwrap())
            .collect();
        assert!(segs.len() > 1, "expected a mid-section split");
        assert!(
            segs[0].initial_breadcrumb.is_empty(),
            "the first segment has no ancestor headings"
        );
        assert_eq!(
            segs[1].initial_breadcrumb,
            vec!["Top".to_string()],
            "the second segment must carry 'Top' forward as its ancestor"
        );
    }
}
