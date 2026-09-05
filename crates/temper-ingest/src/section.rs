//! Heading-section byte slicing for the re-block operation.
//!
//! [`slice_sections`] splits a body at ATX heading boundaries with the exact
//! same rule the chunker and the streaming segmenter use (`chunk::heading_re`
//! plus the pop-on-same-or-higher-level stack rule). It is the partition
//! primitive for re-cutting a resource's blocks: the slices are byte-verbatim
//! with terminators retained, so concatenating them in `seq` order reproduces
//! the source byte-for-byte.

use crate::chunk::heading_re;

/// One heading-delimited slice of a source body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionSlice {
    pub seq: u32,
    /// The section's bytes, verbatim. A section that begins at a heading line
    /// carries that line first; the leading preamble (body text before the
    /// first heading) is its own slice.
    pub text: String,
    /// The heading-stack breadcrumb (title-only, outermost first) in force at
    /// the section's start — the ancestors its own heading nests under, and
    /// exactly what [`crate::chunk::chunk_markdown_with_prefix`] takes as
    /// `initial_breadcrumb` for the slice.
    pub initial_breadcrumb: Vec<String>,
}

/// Split `body` into heading-delimited sections, byte-verbatim.
///
/// Every heading line starts a new section regardless of level, and belongs
/// to the section it opens. A body with no headings is one slice — the whole
/// body — which is what makes re-blocking a single un-sectioned block a
/// natural no-op. Empty input yields no slices.
pub fn slice_sections(body: &str) -> Vec<SectionSlice> {
    let mut sections: Vec<SectionSlice> = Vec::new();
    let mut header_stack: Vec<(usize, String)> = Vec::new();
    let mut breadcrumb: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut seq = 0u32;

    for line in body.split_inclusive('\n') {
        // Terminators are retained in the slice bytes; heading detection runs
        // against the trimmed line, exactly like the streaming segmenter.
        if let Some(caps) = heading_re().captures(line.trim_end()) {
            if !current.is_empty() {
                sections.push(SectionSlice {
                    seq,
                    text: std::mem::take(&mut current),
                    initial_breadcrumb: std::mem::take(&mut breadcrumb),
                });
                seq += 1;
            }
            // Snapshot the ancestor stack before this section's own heading
            // mutates it — the segmenter's `begin_segment_with` ordering.
            breadcrumb = header_stack.iter().map(|(_, t)| t.clone()).collect();

            let level = caps[1].len();
            let title = caps[2].trim().to_string();
            while header_stack
                .last()
                .map(|(l, _)| *l >= level)
                .unwrap_or(false)
            {
                header_stack.pop();
            }
            header_stack.push((level, title));
        }
        current.push_str(line);
    }

    if !current.is_empty() {
        sections.push(SectionSlice {
            seq,
            text: current,
            initial_breadcrumb: breadcrumb,
        });
    }
    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stream segmenter's verbatim fixture set: CRLF, a missing trailing
    /// newline, blank lines, and multibyte UTF-8 must all survive a
    /// slice→rejoin round-trip unchanged — on heading-bearing bodies too.
    #[test]
    fn slices_rejoin_byte_exactly() {
        for doc in [
            "# T\n\nalpha\nbeta\n",
            "# T\n\nalpha\nbeta",
            "# T\r\n\r\nalpha\r\nbeta\r\n",
            "# T\n\nnaïve — ünïcode ✅\n",
            "# A\n\n## B\n\none\n\n\n# C\n\ntwo\n",
        ] {
            let slices = slice_sections(doc);
            let rejoined: String = slices.iter().map(|s| s.text.as_str()).collect();
            assert_eq!(rejoined, doc, "slices must rejoin byte-exactly: {doc:?}");
        }
    }

    /// A body with no headings is one slice — the precondition that makes
    /// re-blocking a single un-sectioned block a no-op.
    #[test]
    fn no_headings_is_one_slice() {
        let body = "Just some plain text.\nNo headings here.\n";
        let slices = slice_sections(body);
        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0].seq, 0);
        assert_eq!(slices[0].text, body);
        assert!(slices[0].initial_breadcrumb.is_empty());
    }

    /// Text before the first heading is its own slice, with an empty
    /// breadcrumb — it belongs to no section. A heading-led slice's
    /// breadcrumb holds the ANCESTORS only: the slice carries its own
    /// heading line, and `chunk_markdown_with_prefix` pushes it itself.
    #[test]
    fn preamble_before_first_heading_is_its_own_slice() {
        let slices = slice_sections("Intro line.\n\n# A\n\n## B\n\nbody\n");
        assert_eq!(slices.len(), 3);
        assert_eq!(slices[0].text, "Intro line.\n\n");
        assert!(slices[0].initial_breadcrumb.is_empty());
        assert_eq!(slices[1].text, "# A\n\n");
        assert!(slices[1].initial_breadcrumb.is_empty());
        assert_eq!(slices[2].text, "## B\n\nbody\n");
        assert_eq!(slices[2].initial_breadcrumb, vec!["A".to_string()]);
    }

    /// Each heading opens its own slice carrying the heading line first, and
    /// the breadcrumb stack pops on same-or-higher levels — the chunker's
    /// rule, so slice chunking agrees with whole-document chunking. The
    /// snapshot is taken before the section's own heading pops, exactly like
    /// the segmenter's `begin_segment_with`.
    #[test]
    fn nested_and_sibling_headings_pop_the_stack() {
        let slices = slice_sections("# A\n\n## B\n\n### C\n\nc\n\n## D\n\nd\n");
        let got: Vec<(u32, &str, Vec<&str>)> = slices
            .iter()
            .map(|s| {
                (
                    s.seq,
                    s.text.lines().next().unwrap(),
                    s.initial_breadcrumb.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (0, "# A", vec![]),
                (1, "## B", vec!["A"]),
                (2, "### C", vec!["A", "B"]),
                (3, "## D", vec!["A", "B", "C"]),
            ]
        );

        // The proof the ancestor snapshot is the right contract: chunking
        // each slice with its snapshot reproduces whole-document chunking —
        // path, depth, and content-hash sequence (chunk_index restarts per
        // slice; the re-block payload renumbers it).
        let keys = |chunks: Vec<crate::chunk::ChunkData>| {
            chunks
                .into_iter()
                .map(|c| (c.header_path, c.heading_depth, c.content_hash))
                .collect::<Vec<_>>()
        };
        let whole = keys(crate::chunk::chunk_markdown(
            "# A\n\n## B\n\n### C\n\nc\n\n## D\n\nd\n",
        ));
        let mut sliced = Vec::new();
        for s in &slices {
            sliced.extend(crate::chunk::chunk_markdown_with_prefix(
                &s.text,
                &s.initial_breadcrumb,
            ));
        }
        assert_eq!(keys(sliced), whole);
    }

    /// Empty input has nothing to partition.
    #[test]
    fn empty_body_yields_no_slices() {
        assert!(slice_sections("").is_empty());
    }
}
