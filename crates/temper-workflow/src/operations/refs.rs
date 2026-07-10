//! Resource addressing primitives — the one decorated-ref resolver.
//!
//! Identity contract (Adjudication 5): a resource is addressed by a bare
//! UUID or the decorated form `sluggify(title)-<uuid>`. Resolution is
//! trailing-UUID-only — the decoration is parsed off and ignored, so a
//! stale or wrong slug half is harmless. Decorations are never stored,
//! never authoritative. This module migrates to `temper-workflow` at
//! post-cutover crate extraction.

use temper_core::error::TemperError;
use temper_core::types::ids::ResourceId;
use temper_core::types::provenance::ProvenanceSource;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

/// Slugify a title for a resource's stored slug and the decoration half of a
/// ref / filename. The output is always
/// [`validate_slug`](super::actions::validate_slug)-conformant (ASCII lowercase
/// alphanumerics separated by single hyphens) — this is what lets a
/// title-derived slug pass create-time validation, and it holds **by
/// construction** because the keep-test below is ASCII-restricted.
///
/// Non-ASCII characters are **transliterated to ASCII**, not passed through and
/// not silently dropped (issue #320). The title is first Unicode-normalized
/// with NFKD (compatibility decomposition), which turns superscript/subscript
/// digits into plain digits (`⁷` → `7`), splits accented letters into base +
/// combining mark, and expands vulgar fractions (`½` → `1⁄2`). Combining marks
/// are then dropped (`é` → `e`), a small set of common non-decomposable symbols
/// is mapped to sensible ASCII (`§` → `sec`, `°` → `deg`, dashes/bullets → `-`,
/// curly quotes → `'`/`"`), and any other non-ASCII char collapses to a
/// separator. Finally the string is lowercased and every run of
/// non-ASCII-alphanumeric chars collapses to a single `-` with leading/trailing
/// `-` trimmed.
///
/// A title with no ASCII alphanumerics after transliteration (e.g. wholly
/// non-Latin script) slugs to the empty string, which `validate_slug` then
/// rejects with a clear error rather than producing a silent bad slug.
pub fn sluggify(title: &str) -> String {
    // Step 1 — NFKD-normalize and fold to ASCII: drop combining marks, keep
    // ASCII verbatim, map the common non-decomposable symbols, and collapse any
    // other non-ASCII char to a separator. `folded` is pure ASCII by construction.
    let mut folded = String::with_capacity(title.len());
    for c in title.nfkd() {
        if is_combining_mark(c) {
            continue;
        }
        if c.is_ascii() {
            folded.push(c);
        } else {
            folded.push_str(fold_non_ascii_symbol(c));
        }
    }
    // Step 2 — lowercase, split on every run of non-ASCII-alphanumeric chars,
    // and rejoin with single `-` (drops empty leading/trailing/interior runs).
    folded
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|seg| !seg.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Map a non-ASCII char that NFKD left undecomposed to sensible ASCII, mirroring
/// the client-side reference fold in issue #320. `§`/`°` become words (padded so
/// they never fuse with an adjacent digit); dashes/bullets and typographic
/// quotes/ellipsis map to their ASCII forms. Any char not listed collapses to a
/// space — a slug separator — so non-Latin scripts degrade to hyphens rather
/// than surviving as slug-invalid bytes.
fn fold_non_ascii_symbol(c: char) -> &'static str {
    match c {
        // en/em/figure/horizontal dash, bullet, middle dot, fraction slash
        '\u{2013}' | '\u{2014}' | '\u{2012}' | '\u{2015}' | '\u{2022}' | '\u{00B7}'
        | '\u{2044}' => "-",
        '\u{2018}' | '\u{2019}' | '\u{201A}' => "'", // single curly quotes
        '\u{201C}' | '\u{201D}' | '\u{201E}' => "\"", // double curly quotes
        '\u{2026}' => "...",                         // horizontal ellipsis
        '\u{00A7}' => " sec ",                       // section sign
        '\u{00B0}' => " deg ",                       // degree sign
        _ => " ",
    }
}

/// The decorated, self-resolving form printed for every resource:
/// `sluggify(title)-<uuid>`.
pub fn decorated_ref(title: &str, id: ResourceId) -> String {
    format!("{}-{}", sluggify(title), id.0)
}

/// Resolve a ref string to a `ResourceId`. Accepts a bare UUID or a
/// decorated `…-<uuid>` form; resolution is trailing-UUID-only (the
/// decoration is ignored). No fuzzy/fragment matching — unparseable input
/// is an error, never a guess.
pub fn parse_ref(s: &str) -> Result<ResourceId, TemperError> {
    let s = s.trim();
    // Bare UUID.
    if let Ok(id) = Uuid::parse_str(s) {
        return Ok(ResourceId(id));
    }
    // Decorated: the trailing UUID is the last 5 hyphen-delimited groups
    // (UUIDs contain 4 internal hyphens). Walk from the right.
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() >= 5 {
        let tail = parts[parts.len() - 5..].join("-");
        if let Ok(id) = Uuid::parse_str(&tail) {
            return Ok(ResourceId(id));
        }
    }
    Err(TemperError::Project(format!(
        "not a resource ref (expected a UUID or `slug-<uuid>`): {s:?}"
    )))
}

/// Classify one `--sources` value into a [`ProvenanceSource`]: an http/https/file URI becomes
/// [`ProvenanceSource::Remote`] (an external source, carried verbatim — the projector normalizes it);
/// anything else is a ref (UUID or decorated) resolved to [`ProvenanceSource::Resource`] via
/// [`parse_ref`]. A value that is neither a URI nor a parseable ref is a hard error — never a silent
/// drop (parse-don't-validate / escalate). Shared by the CLI `--sources` flag and the MCP `sources`
/// input so both surfaces classify identically (one classifier, no send/receive drift).
pub fn resolve_provenance_source(value: &str) -> Result<ProvenanceSource, TemperError> {
    let value = value.trim();
    if is_remote_provenance_uri(value) {
        Ok(ProvenanceSource::Remote(value.to_owned()))
    } else {
        Ok(ProvenanceSource::Resource(Uuid::from(parse_ref(value)?)))
    }
}

/// A value is a remote URL iff it carries an http/https scheme (case-insensitive). Scheme-only +
/// conservative, so a bare UUID or decorated ref can never be mistaken for a URL (and a non-web
/// scheme like `ftp://` is not a provenance source — it falls through to `parse_ref` and errors).
///
/// The one canonical **network-fetchable** URL classifier, shared by the CLI `--from` URL/path split
/// (a match is fetched over HTTP) and the server-side origin-URI provenance default (issue #352) so
/// both classify identically. `file://` is deliberately **not** remote here — it is a local path, not
/// something to fetch over the network; the `--sources` provenance path admits it separately via
/// `is_remote_provenance_uri`.
pub fn is_remote_url(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// A `--sources` value is an external (Remote) provenance URI iff it is a network-fetchable URL
/// ([`is_remote_url`]) **or** a `file://` URI (issue #353). `file://` lets a bulk importer record a
/// local working-tree path as provenance — the server's `normalize_remote_uri` already accepts any
/// scheme, so the value is carried verbatim with no server change. This is a strict superset of
/// `is_remote_url`, scoped to provenance classification: it must NOT gate the `--from` fetch path
/// (you cannot reqwest a `file://`) nor the origin-URI default, which stay http/https-only.
fn is_remote_provenance_uri(value: &str) -> bool {
    is_remote_url(value) || value.trim().to_ascii_lowercase().starts_with("file://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sluggify_lowercases_and_dashes() {
        // Runs of non-ASCII-alphanumeric chars collapse to a single `-`;
        // leading/trailing `-` are trimmed.
        assert_eq!(sluggify("Hello, World!"), "hello-world");
        assert_eq!(sluggify("  Trim --Me-- "), "trim-me");
    }

    #[test]
    fn sluggify_output_is_validate_slug_conformant() {
        // Regression guard (bugs B2 2026-07-06 + #320): the generator's output is
        // checked by the *same* `validate_slug` the request path uses, so the two
        // can never diverge again. Non-ASCII is NFKD-transliterated to ASCII —
        // superscript digits and accented letters survive as their ASCII
        // equivalents rather than being dropped or passed through.
        use crate::operations::actions::validate_slug;
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
        // Transliteration preserves information rather than dropping it.
        assert_eq!(
            sluggify("Three distinct map telē"),
            "three-distinct-map-tele"
        );
        assert_eq!(sluggify("Café déjà"), "cafe-deja");
        assert_eq!(
            sluggify("Some Kind of Terms⁷ (part 3 of 12)"),
            "some-kind-of-terms7-part-3-of-12"
        );
        assert_eq!(sluggify("Notice Period⁶"), "notice-period6");
        assert_eq!(sluggify("§5 Payment Terms"), "sec-5-payment-terms");
        assert_eq!(sluggify("Ambient 20° Room"), "ambient-20-deg-room");
        assert_eq!(sluggify("One ½ portion"), "one-1-2-portion");
        // Wholly non-Latin script has no ASCII alphanumerics to transliterate.
        assert_eq!(sluggify("Ολοκλήρωμα"), "");
    }

    #[test]
    fn sluggify_never_emits_an_invalid_slug() {
        // Property (#320): for ANY title, the derived slug is either empty (no
        // ASCII alphanumerics) or validate_slug-conformant — the generator can
        // never emit a slug the validator refuses. Sweep a wide codepoint range,
        // including the compatibility/symbol blocks that motivated the bug.
        use crate::operations::actions::validate_slug;
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

    #[test]
    fn decorated_ref_is_slug_dash_uuid() {
        let id = ResourceId(Uuid::parse_str("019e84ab-26ba-7560-9d34-c60d74a9fbe2").unwrap());
        assert_eq!(
            decorated_ref("My Task", id),
            "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2"
        );
    }

    #[test]
    fn parse_ref_accepts_bare_uuid() {
        let s = "019e84ab-26ba-7560-9d34-c60d74a9fbe2";
        assert_eq!(
            parse_ref(s).unwrap(),
            ResourceId(Uuid::parse_str(s).unwrap())
        );
    }

    #[test]
    fn parse_ref_accepts_decorated_and_ignores_slug_half() {
        let uuid = "019e84ab-26ba-7560-9d34-c60d74a9fbe2";
        let want = ResourceId(Uuid::parse_str(uuid).unwrap());
        // correct decoration
        assert_eq!(parse_ref(&format!("my-task-{uuid}")).unwrap(), want);
        // STALE/WRONG decoration resolves identically — harmless by construction
        assert_eq!(
            parse_ref(&format!("totally-wrong-slug-{uuid}")).unwrap(),
            want
        );
    }

    #[test]
    fn parse_ref_round_trips_decorated_ref() {
        let id = ResourceId(Uuid::now_v7());
        for title in ["A B C", "", "punct!@#", "already-slug"] {
            assert_eq!(parse_ref(&decorated_ref(title, id)).unwrap(), id);
        }
    }

    #[test]
    fn parse_ref_rejects_fragments_and_garbage() {
        // no trailing uuid → error, NO fuzzy fallback
        assert!(parse_ref("just-a-slug").is_err());
        assert!(parse_ref("").is_err());
        assert!(parse_ref("not-a-uuid-1234").is_err());
    }

    #[test]
    fn resolve_provenance_source_classifies_url_as_remote() {
        // http/https (any scheme case) → Remote, carrying the URL verbatim (not lowercased whole).
        assert_eq!(
            resolve_provenance_source("https://Example.com/Issue/1").unwrap(),
            ProvenanceSource::Remote("https://Example.com/Issue/1".to_owned())
        );
        assert_eq!(
            resolve_provenance_source("  HTTP://a.test/x  ").unwrap(),
            ProvenanceSource::Remote("HTTP://a.test/x".to_owned())
        );
    }

    #[test]
    fn resolve_provenance_source_classifies_ref_as_resource() {
        let uuid = "019e84ab-26ba-7560-9d34-c60d74a9fbe2";
        let want = ProvenanceSource::Resource(Uuid::parse_str(uuid).unwrap());
        assert_eq!(resolve_provenance_source(uuid).unwrap(), want);
        // decorated ref resolves to the same Resource (trailing-UUID-only)
        assert_eq!(
            resolve_provenance_source(&format!("my-task-{uuid}")).unwrap(),
            want
        );
    }

    #[test]
    fn resolve_provenance_source_classifies_file_uri_as_remote() {
        // `file://` (issue #353): a local working-tree path is a legitimate provenance source —
        // carried verbatim as Remote (the server's normalize_remote_uri accepts any scheme).
        assert_eq!(
            resolve_provenance_source("file:///path/to/doc.md").unwrap(),
            ProvenanceSource::Remote("file:///path/to/doc.md".to_owned())
        );
        // Case-insensitive scheme match; casing preserved verbatim (not lowercased whole).
        assert_eq!(
            resolve_provenance_source("  FILE:///Path/To/Doc.md  ").unwrap(),
            ProvenanceSource::Remote("FILE:///Path/To/Doc.md".to_owned())
        );
    }

    #[test]
    fn resolve_provenance_source_rejects_non_url_non_ref() {
        // neither a URL nor a parseable ref → hard error (escalate, never a silent drop)
        assert!(resolve_provenance_source("just-a-slug").is_err());
        assert!(resolve_provenance_source("ftp://host/x").is_err()); // non-web/non-file scheme is not remote
    }

    #[test]
    fn file_uri_is_a_provenance_source_but_not_network_remote() {
        // Collision guard (issue #353 vs #352): `file://` is a valid *provenance* URI but is NOT a
        // network-fetchable URL. `is_remote_url` gates the `--from` HTTP-fetch path and the origin-URI
        // default — routing `file://` there would try to reqwest a local path — so it must stay false;
        // only the provenance-scoped predicate admits it.
        assert!(!is_remote_url("file:///path/to/doc.md"));
        assert!(is_remote_provenance_uri("file:///path/to/doc.md"));
        // http/https answer yes to both; the provenance predicate is a strict superset.
        assert!(is_remote_url("https://a.test/x"));
        assert!(is_remote_provenance_uri("https://a.test/x"));
        // ftp is neither.
        assert!(!is_remote_url("ftp://h/x"));
        assert!(!is_remote_provenance_uri("ftp://h/x"));
    }
}
