//! Drift-guard: `ManagedMeta` (temper-workflow) must stay in lockstep with the
//! authoritative Property vocabulary `MANAGED_PROPERTY_KEYS` + `key_fate`
//! (temper-substrate). This is the single place both crates are visible, so it is
//! the parity gate that kills the `temper-llm-model` drift class (spec P2.4).
//!
//! Two guarantees:
//!   1. The struct literal names EXACTLY the 10 Property fields (no `..Default`),
//!      so adding or removing a `ManagedMeta` field fails to compile until this
//!      test is updated in lockstep.
//!   2. The serialized `temper-*` key set equals `MANAGED_PROPERTY_KEYS`, and each
//!      key is `KeyFate::Property`.

use std::collections::BTreeSet;

use temper_substrate::keys::{key_fate, KeyFate, MANAGED_PROPERTY_KEYS};
use temper_workflow::types::ManagedMeta;

#[test]
fn managed_meta_fields_match_single_source_of_truth() {
    // Naming every field explicitly (no `..Default::default()`) makes this a
    // compile-time guard: the struct must have EXACTLY these Property fields.
    let mm = ManagedMeta {
        stage: Some("in-progress".into()),
        mode: Some("build".into()),
        effort: Some("medium".into()),
        status: Some("active".into()),
        seq: Some(1),
        branch: Some("main".into()),
        pr: Some("#1".into()),
        llm_model: Some("claude".into()),
        llm_run: Some("01947b5c-0000-0000-0000-000000000000".into()),
        provenance: Some("llm-discovered".into()),
    };

    let serialized = serde_json::to_value(&mm).expect("serialize ManagedMeta");
    let field_keys: BTreeSet<String> = serialized
        .as_object()
        .expect("ManagedMeta serializes to an object")
        .keys()
        .cloned()
        .collect();
    let source_keys: BTreeSet<String> = MANAGED_PROPERTY_KEYS
        .iter()
        .map(|s| s.to_string())
        .collect();

    assert_eq!(
        field_keys, source_keys,
        "ManagedMeta's serde keys drifted from MANAGED_PROPERTY_KEYS (temper-substrate)"
    );

    // Every single-sourced Property key must be Property-fated by the §7 classifier.
    for k in MANAGED_PROPERTY_KEYS {
        assert_eq!(
            key_fate(k),
            KeyFate::Property,
            "{k} must be KeyFate::Property"
        );
    }
}
