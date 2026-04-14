//! Integration tests for `temper_core::frontmatter`.
//!
//! Covers parse + project + mutate + serialize + hash across every
//! doctype, plus alias/hash symmetry and error cases. Golden files
//! are committed; set `REGENERATE_GOLDENS=1` to overwrite them after
//! intentional serializer changes.

use std::fs;
use std::path::{Path, PathBuf};

use temper_core::frontmatter::{DocType, Frontmatter};
use temper_core::types::graph::ResourceRelationships;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/frontmatter")
}

fn load_fixture(name: &str) -> String {
    let path = fixtures_dir().join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()))
}

fn golden_path(stem: &str) -> PathBuf {
    fixtures_dir()
        .join("golden")
        .join(format!("{stem}.canonical.md"))
}

fn assert_golden_matches(stem: &str, actual: &str) {
    let path = golden_path(stem);
    if std::env::var("REGENERATE_GOLDENS").as_deref() == Ok("1") {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create golden dir");
        }
        fs::write(&path, actual).expect("write golden");
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read golden {}: {e} — run with REGENERATE_GOLDENS=1 to create it",
            path.display()
        )
    });
    assert_eq!(actual, expected, "golden mismatch for {stem}");
}

/// All round-trippable fixtures — `(stem, doctype)`.
const ROUND_TRIP_CASES: &[(&str, DocType)] = &[
    ("task_minimal", DocType::Task),
    ("task_full", DocType::Task),
    ("task_with_aliases", DocType::Task),
    ("goal_full", DocType::Goal),
    ("session_full", DocType::Session),
    ("research_full", DocType::Research),
    ("decision_full", DocType::Decision),
    ("concept_full", DocType::Concept),
];

#[test]
fn every_fixture_parses_and_matches_its_golden() {
    for (stem, expected_doctype) in ROUND_TRIP_CASES {
        let content = load_fixture(&format!("{stem}.md"));
        let fm = Frontmatter::try_from(content.as_str())
            .unwrap_or_else(|e| panic!("parse failed for {stem}: {e}"));
        assert_eq!(
            fm.doc_type(),
            *expected_doctype,
            "doctype mismatch for {stem}"
        );
        let serialized = fm
            .serialize()
            .unwrap_or_else(|e| panic!("serialize failed for {stem}: {e}"));
        assert_golden_matches(stem, &serialized);
    }
}

#[test]
fn golden_is_a_fixed_point_of_parse_serialize() {
    // Re-reading the golden and re-serializing must produce byte-identical
    // output. Locks the "canonical form is a fixed point" property.
    if std::env::var("REGENERATE_GOLDENS").as_deref() == Ok("1") {
        return; // skip during regeneration
    }
    for (stem, _) in ROUND_TRIP_CASES {
        let golden_text = fs::read_to_string(golden_path(stem))
            .unwrap_or_else(|e| panic!("read golden {stem}: {e}"));
        let fm = Frontmatter::try_from(golden_text.as_str())
            .unwrap_or_else(|e| panic!("re-parse golden {stem}: {e}"));
        let re_serialized = fm
            .serialize()
            .unwrap_or_else(|e| panic!("re-serialize golden {stem}: {e}"));
        assert_eq!(re_serialized, golden_text, "fixed-point failed for {stem}");
    }
}

#[test]
fn hashes_are_byte_identical_to_legacy_path_per_doctype() {
    use temper_core::frontmatter::parse::{normalize_aliases, parse_yaml, split_frontmatter_block};
    use temper_core::hash::compute_frontmatter_hashes_from_yaml;

    for (stem, dt) in ROUND_TRIP_CASES {
        let content = load_fixture(&format!("{stem}.md"));
        let fm = Frontmatter::try_from(content.as_str()).unwrap();
        let (new_managed, new_open) = fm.hashes();

        let (yaml_text, _body) = split_frontmatter_block(&content).unwrap();
        let mut legacy_value = parse_yaml(&yaml_text).unwrap();
        normalize_aliases(&mut legacy_value);
        let (legacy_managed, legacy_open) =
            compute_frontmatter_hashes_from_yaml(Some(&legacy_value), dt.as_str());

        assert_eq!(new_managed, legacy_managed, "managed hash drift for {stem}");
        assert_eq!(new_open, legacy_open, "open hash drift for {stem}");
    }
}

#[test]
fn alias_form_hashes_match_canonical_form() {
    let alias = Frontmatter::try_from(load_fixture("task_with_aliases.md").as_str()).unwrap();
    let canonical = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: Aliased Task
slug: aliased-task
temper-stage: in-progress
relates_to: [a]
depends_on: [b]
preceded_by: [c]
derived_from: [d]
---
body
"#;
    let c = Frontmatter::try_from(canonical).unwrap();
    assert_eq!(
        alias.hashes(),
        c.hashes(),
        "alias-form and canonical-form must hash identically"
    );
}

#[test]
fn display_ordering_has_zero_effect_on_hashes() {
    let a = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: T
slug: t
temper-stage: in-progress
relates_to: [x]
---
"#;
    let b = r#"---
slug: t
title: T
temper-stage: in-progress
relates_to: [x]
temper-created: "2026-04-13T00:00:00Z"
temper-context: temper
temper-type: task
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
---
"#;
    let c = r#"---
relates_to: [x]
temper-stage: in-progress
temper-created: "2026-04-13T00:00:00Z"
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
title: T
temper-context: temper
slug: t
temper-type: task
---
"#;
    let h_a = Frontmatter::try_from(a).unwrap().hashes();
    let h_b = Frontmatter::try_from(b).unwrap().hashes();
    let h_c = Frontmatter::try_from(c).unwrap().hashes();
    assert_eq!(h_a, h_b);
    assert_eq!(h_b, h_c);
}

#[test]
fn mutate_then_write_round_trips_through_parse() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("task.md");

    let mut fm = Frontmatter::try_from(load_fixture("task_full.md").as_str()).unwrap();
    let new_rels = ResourceRelationships {
        relates_to: vec!["brand-new".to_string()],
        ..ResourceRelationships::default()
    };
    fm.set_relationships(&new_rels);

    fm.write_to(&path).unwrap();
    let re = Frontmatter::parse_file(&path).unwrap();
    let re_rels = ResourceRelationships::from(&re);
    assert_eq!(re_rels.relates_to, vec!["brand-new"]);
    assert!(
        re_rels.depends_on.is_empty(),
        "depends_on should have been cleared by set_relationships"
    );
}

#[test]
fn malformed_yaml_errors() {
    let content = load_fixture("malformed_yaml.md");
    assert!(Frontmatter::try_from(content.as_str()).is_err());
}

#[test]
fn wrong_doc_type_errors() {
    let content = load_fixture("wrong_doc_type.md");
    assert!(Frontmatter::try_from(content.as_str()).is_err());
}

#[test]
fn missing_required_parses_but_fails_validation() {
    // Parsing succeeds because the file is structurally valid — validation
    // surfaces the missing `title` field.
    let content = load_fixture("missing_required.md");
    let fm = Frontmatter::try_from(content.as_str()).unwrap();
    let issues = fm.validate().unwrap();
    assert!(
        !issues.is_empty(),
        "missing required fields should produce issues"
    );
}

#[test]
fn tags_as_strings_do_not_become_parent_of_relationships() {
    // `tags` is metadata — it must not project into any of the
    // `ResourceRelationships` edge-producing fields.
    let content = load_fixture("tags_as_strings.md");
    let fm = Frontmatter::try_from(content.as_str()).unwrap();
    let rels = ResourceRelationships::from(&fm);
    assert!(rels.relates_to.is_empty());
    assert!(rels.depends_on.is_empty());
    assert!(rels.extends.is_empty());
    assert!(rels.references.is_empty());
    assert!(rels.preceded_by.is_empty());
    assert!(rels.derived_from.is_empty());
    assert!(rels.parent.is_none());
    // tags itself lives on the struct for session 1 — verify it got there.
    assert_eq!(
        rels.tags,
        vec![
            "auth".to_string(),
            "observability".to_string(),
            "not-a-resource".to_string()
        ]
    );

    // And the accessor returns the same thing.
    assert_eq!(
        fm.tags(),
        vec![
            "auth".to_string(),
            "observability".to_string(),
            "not-a-resource".to_string()
        ]
    );
}
