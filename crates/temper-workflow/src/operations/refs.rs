//! Resource addressing primitives — re-exported from their canonical home.
//!
//! The implementations moved to [`temper_core::refs`]: [`ResourceView`] derives
//! its `ref` through [`decorated_ref`], and `ResourceView` is read back by
//! temper-substrate, the layer *below* this crate. This module keeps every
//! `temper_workflow::operations::…` call site resolving unchanged.
//!
//! The `validate_slug`-conformance property stays here, because the validator it
//! checks against ([`crate::operations::validate_slug`]) is a temper-workflow
//! symbol. Asserting it from this side is the only acyclic direction.
//!
//! [`ResourceView`]: crate::types::ResourceView

pub use temper_core::refs::{
    decorated_ref, is_remote_url, parse_ref, resolve_provenance_source, sluggify,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::actions::validate_slug;

    #[test]
    fn sluggify_output_is_validate_slug_conformant() {
        // Regression guard (bugs B2 2026-07-06 + #320): the generator's output is
        // checked by the *same* `validate_slug` the request path uses, so the two
        // can never diverge again. Non-ASCII is NFKD-transliterated to ASCII —
        // superscript digits and accented letters survive as their ASCII
        // equivalents rather than being dropped or passed through.
        for title in [
            "Three distinct map telē",            // accented letter → base letter
            "Café déjà",                          // interior accents transliterated
            "Some Kind of Terms⁷ (part 3 of 12)", // superscript footnote → plain digit
            "Notice Period⁶",                     // trailing superscript digit
            "§5 Payment Terms",                   // section sign → word
            "Ambient 20° Room",                   // degree sign → word
            "One ½ portion",                      // vulgar fraction expanded
            "“Smart” quotes — and dashes",        // typographic punctuation
            "Ολοκλήρωμα",                         // wholly non-Latin → empty (rejected)
            "Hello, World!",                      // punctuation run → single hyphen
            "  Trim --Me-- ",                     // leading/trailing separators trimmed
        ] {
            let slug = sluggify(title);
            if slug.is_empty() {
                // Empty is the documented "no ASCII alphanumerics" outcome, which
                // validate_slug rejects with a clear error (never a silent bad slug).
                assert!(validate_slug(&slug).is_err());
            } else {
                assert!(
                    validate_slug(&slug).is_ok(),
                    "sluggify({title:?}) = {slug:?} must be validate_slug-conformant"
                );
            }
        }
    }

    #[test]
    fn sluggify_never_emits_an_invalid_slug() {
        // Property (#320): for ANY title, the derived slug is either empty (no
        // ASCII alphanumerics) or validate_slug-conformant — the generator can
        // never emit a slug the validator refuses. Sweep a wide codepoint range,
        // including the compatibility/symbol blocks that motivated the bug.
        let mut checked = 0usize;
        for cp in (0x20u32..0x2200)
            .chain(0x2C00..0x2E00)
            .chain(0x1F600..0x1F680)
        {
            let Some(ch) = char::from_u32(cp) else {
                continue;
            };
            // Embed the sweep char among ASCII so most cases exercise the
            // "interior non-ASCII" collapse rather than the all-empty edge.
            let title = format!("a{ch}b");
            let slug = sluggify(&title);
            assert!(
                slug.is_empty() || validate_slug(&slug).is_ok(),
                "sluggify({title:?}) = {slug:?} (cp U+{cp:04X}) is neither empty nor valid"
            );
            checked += 1;
        }
        assert!(checked > 8000, "sweep should cover thousands of codepoints");
    }
}
