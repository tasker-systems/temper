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

/// Golden per-fixture (managed_hash, open_hash) pairs captured in
/// session 2 task 10. These anchor hash stability across future
/// refactors. If the schema or canonicalization algorithm moves,
/// update these deliberately — don't regenerate blindly.
const FIXTURE_HASH_GOLDENS: &[(&str, &str, &str)] = &[
    // (stem, managed_hash, open_hash)
    (
        "task_minimal",
        "sha256:60ec84b155a2a72a53847c6127bbb6ae36ea4f253677d2981050463cba7d1310",
        "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a",
    ),
    (
        "task_full",
        "sha256:9680d621f3dcf03d90c8b5e9362fbd2849b37df232086d345f6794e86aa74fc0",
        "sha256:998a5ed4c7ce3d2b6b453caa05a77c15590e69d63dd4d8d72fa313902fe143dc",
    ),
    (
        "task_with_aliases",
        "sha256:117c9f8b186e8ed88cc028a71546406edd5f48cf832f890e135ab5defc72cbd3",
        "sha256:7c2eb7eddf439213c8a1f9689b313a0dd28acf34cb16e4d3593e07bc9ffcc70c",
    ),
    (
        "goal_full",
        "sha256:ea2c9736c5b33ec28cbc2baf960540b3b49a7790a599eae78b78e1e205254f22",
        "sha256:2cc0d1501ab8d23caaf440e3e96476b2eb88eedfde19517740f237b4ac4aea0c",
    ),
    (
        "session_full",
        "sha256:92d1ce4e713f190c7fb3f79d3ae3e66f1d1a3eef54a9f575058cb0e3c86bdec4",
        "sha256:87347a2d8ad9bbb615e57527313125c80fc7c0750710f48bf3f8ad69c35da811",
    ),
    (
        "research_full",
        "sha256:98620600cfa2a2276ec60a494c5a748f0bbeb85221dc1d4602c3ee7d85aa1ed5",
        "sha256:e48f15a218a6adda5aed8207b201d1d178f20d2b0bffd7a2b619414e08569897",
    ),
    (
        "decision_full",
        "sha256:78d3cc088880e46958f51b42197d533472cacc79ab0073cf0db3be258d94d699",
        "sha256:2f8bc766b2649cb0b8efb9dd3eab08b6d4d37111b473666ddbb173059ecf78ce",
    ),
    (
        "concept_full",
        "sha256:a010537170e654b6277b4efcf6982736557b60a4e98ddf1735887dc38b62004d",
        "sha256:e90feecd8af6e01e61b003b79f0dd7e74b7ccc186df842608985f450f522cd9a",
    ),
];

#[test]
fn fixture_hashes_match_goldens() {
    let mut drift = Vec::new();
    for (stem, expected_managed, expected_open) in FIXTURE_HASH_GOLDENS {
        let content = load_fixture(&format!("{stem}.md"));
        let fm = Frontmatter::try_from(content.as_str()).unwrap();
        let (managed_hash, open_hash) = fm.hashes();
        if managed_hash != *expected_managed {
            drift.push(format!(
                "  {stem} managed: golden={expected_managed}\n              actual={managed_hash}"
            ));
        }
        if open_hash != *expected_open {
            drift.push(format!(
                "  {stem} open:    golden={expected_open}\n              actual={open_hash}"
            ));
        }
    }
    assert!(
        drift.is_empty(),
        "fixture-hash goldens have drifted:\n{}",
        drift.join("\n")
    );
}

#[test]
fn alias_form_hashes_match_canonical_form() {
    let alias = Frontmatter::try_from(load_fixture("task_with_aliases.md").as_str()).unwrap();
    let canonical = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
temper-title: Aliased Task
temper-slug: aliased-task
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
temper-title: T
temper-slug: t
temper-stage: in-progress
relates_to: [x]
---
"#;
    let b = r#"---
temper-slug: t
temper-title: T
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
temper-title: T
temper-context: temper
temper-slug: t
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
    // `ResourceRelationships` edge-producing fields. Session 2 removed
    // the `tags` field from `ResourceRelationships` entirely; the only
    // typed read path is now `Frontmatter::tags()`.
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

    // The typed accessor on Frontmatter is the canonical read path.
    assert_eq!(
        fm.tags(),
        vec![
            "auth".to_string(),
            "observability".to_string(),
            "not-a-resource".to_string()
        ]
    );
}
