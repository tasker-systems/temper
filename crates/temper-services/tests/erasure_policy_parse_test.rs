//! A malformed entry in any of the erasure-policy manifest's files fails the parse, and the error
//! names the file it is in.
//!
//! The four erasure fences (`personal_data_surface_test.rs`, `sensitivity_scan_surface_test.rs`,
//! `resource_erasure_surface_test.rs`, `sensitivity_store_test.rs`) read the manifest only through
//! `temper_services::erasure_policy::in_force`, and so only through
//! `temper_core::erasure_policy::parse`. The hand parsers they used to carry each refused a
//! malformed line with its file and line number. These probes hold the parser to the same refusal,
//! one per file: each corrupts one entry of the shipped text, so the probe reads the real file and
//! not a fixture, and asserts that `parse` errs, names that file, and names the corrupted word.
//! Needs no database.

use temper_core::erasure_policy::{parse, ManifestFile, ManifestFiles};
use temper_services::erasure_policy::FILES;

/// FAILS IF: the shipped manifest does not parse. Without it, a probe below could pass on a file
/// that was already broken, not on the corruption it made.
#[test]
fn the_shipped_manifest_parses() {
    if let Err(err) = parse(&FILES) {
        panic!("the shipped erasure-policy manifest must parse: {err}");
    }
}

/// Parse the shipped manifest with the first `from` in `file` replaced by `to`, and assert that the
/// parse fails in `file`, with a message naming the file and `named`.
fn assert_corruption_fails(file: ManifestFile, from: &str, to: &str, named: &str) {
    let mut files: ManifestFiles<'_> = FILES;
    let slot = match file {
        ManifestFile::Root => &mut files.root,
        ManifestFile::Estate => &mut files.estate,
        ManifestFile::PersonalData => &mut files.personal_data,
        ManifestFile::SensitivityScan => &mut files.sensitivity_scan,
        ManifestFile::ResourceErasure => &mut files.resource_erasure,
    };
    let corrupted = slot.replacen(from, to, 1);
    assert_ne!(
        corrupted, *slot,
        "the probe expects `{from}` in the shipped {file}; the manifest changed shape, so the \
         probe would test nothing"
    );
    *slot = &corrupted;

    let err = parse(&files).expect_err("a corrupted manifest must not parse");
    assert_eq!(err.file, file, "the error names the wrong file: {err}");
    let text = err.to_string();
    assert!(
        text.contains(file.file_name()),
        "the message names no file: {text}"
    );
    assert!(
        text.contains(named),
        "the message does not name the corrupted `{named}`: {text}"
    );
}

/// FAILS IF: a personal-data entry with an unknown class parses.
#[test]
fn an_unknown_class_in_personal_data_fails_and_names_its_file() {
    assert_corruption_fails(
        ManifestFile::PersonalData,
        r#"{ class = "identifier", ceiling = "full", achieved = "erased:7" }"#,
        r#"{ class = "identifying", ceiling = "full", achieved = "erased:7" }"#,
        "identifying",
    );
}

/// FAILS IF: a sensitivity-scan entry with a disposition outside the vocabulary parses.
#[test]
fn a_bad_disposition_in_sensitivity_scan_fails_and_names_its_file() {
    assert_corruption_fails(
        ManifestFile::SensitivityScan,
        r#"{ disposition = "scan", note = "erasure spec F3"#,
        r#"{ disposition = "scanned", note = "erasure spec F3"#,
        "scanned",
    );
}

/// FAILS IF: the estate file parses with a field its model does not have.
#[test]
fn an_unknown_field_in_estate_fails_and_names_its_file() {
    assert_corruption_fails(
        ManifestFile::Estate,
        "\n[tables]",
        "\nunreviewed_scope = true\n\n[tables]",
        "unreviewed_scope",
    );
}

/// FAILS IF: a `[payload]` refinement whose `where` names a field other than `key` or `owner`
/// parses.
#[test]
fn a_bad_qualifier_in_resource_erasure_fails_and_names_its_file() {
    assert_corruption_fails(
        ManifestFile::ResourceErasure,
        r#"{ where = { key = "facet" }, disposition = "structural""#,
        r#"{ where = { kind = "facet" }, disposition = "structural""#,
        "kind",
    );
}
