//! The committed L0 kernel manifest (`schema-artifact/manifests/l0-kernel.yaml`) is a production
//! artifact: the 22 orientation landmarks delivered to the live `system-default` cogmap via
//! `temper cogmap reconcile`. These tests are the drift guard — they assert the committed manifest
//! parses into the Task-7 `ManifestDoc` model, carries exactly the 22 landmarks across the four
//! `layer` categories, that every authored edge resolves to one of those 22 landmarks, and (under
//! `embed`) that it translates to a fully pre-embedded reconcile request with no stray provenance.
//!
//! Pure tests — no auth/runtime — so no `init_isolated_auth` harness is needed.

use std::collections::{HashMap, HashSet};

use temper_cli::actions::reconcile::parse_manifest;

/// Path to the committed manifest, resolved relative to this crate's manifest dir.
const MANIFEST_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../schema-artifact/manifests/l0-kernel.yaml"
);

fn load_manifest_yaml() -> String {
    std::fs::read_to_string(MANIFEST_PATH)
        .unwrap_or_else(|e| panic!("reading committed L0 manifest at {MANIFEST_PATH}: {e}"))
}

#[test]
fn committed_l0_manifest_parses_with_22_landmarks_and_resolvable_edges() {
    let yaml = load_manifest_yaml();
    let doc = parse_manifest(&yaml).expect("committed L0 manifest parses into ManifestDoc");

    // The reconciler-managed slice is exactly the 22 landmark resources.
    assert_eq!(doc.entries.len(), 22, "expected 22 landmark entries");

    // First delivery is all-additive: no tombstones.
    assert!(
        doc.fold_resources.is_empty(),
        "first delivery carries no resource tombstones"
    );
    assert!(
        doc.fold_edges.is_empty(),
        "first delivery carries no edge tombstones"
    );

    // Stable landmark ids are the diff key — build the resolution set (and assert uniqueness); also
    // confirm origin_uris are unique (attribution, not a key) + a per-layer histogram.
    let mut ids: HashSet<uuid::Uuid> = HashSet::new();
    let mut uris: HashSet<String> = HashSet::new();
    let mut layer_counts: HashMap<String, usize> = HashMap::new();
    for e in &doc.entries {
        assert!(ids.insert(e.id), "duplicate landmark id {}", e.id);
        assert!(
            uris.insert(e.origin_uri.clone()),
            "duplicate origin_uri {}",
            e.origin_uri
        );
        assert_eq!(
            e.doc_type, "kernel_landmark",
            "doc_type for {}",
            e.origin_uri
        );
        assert!(!e.title.is_empty(), "title for {} is empty", e.origin_uri);
        assert!(!e.body.is_empty(), "body for {} is empty", e.origin_uri);
        let layer = e.facets["layer"]
            .as_str()
            .unwrap_or_else(|| panic!("entry {} missing string `layer` facet", e.origin_uri))
            .to_string();
        // Decision #6: manifest facets are clustering-only — no provenance in the wire payload.
        assert!(
            e.facets.get("provenance").is_none(),
            "entry {} must not carry provenance (server-stamped)",
            e.origin_uri
        );
        *layer_counts.entry(layer).or_default() += 1;
    }

    // All four landmark categories present, with the authored counts.
    assert_eq!(
        layer_counts.get("concept").copied(),
        Some(10),
        "concept count"
    );
    assert_eq!(
        layer_counts.get("invariant").copied(),
        Some(5),
        "invariant count"
    );
    assert_eq!(
        layer_counts.get("reference").copied(),
        Some(4),
        "reference count"
    );
    assert_eq!(
        layer_counts.get("boundary").copied(),
        Some(3),
        "boundary count"
    );
    assert_eq!(layer_counts.len(), 4, "exactly four layer categories");

    // Every authored edge target (a stable id) resolves to one of the 22 landmarks.
    let mut edge_total = 0;
    for e in &doc.entries {
        for edge in &e.edges {
            assert!(
                ids.contains(&edge.to),
                "edge {} -> {} target unresolved",
                e.origin_uri,
                edge.to
            );
            edge_total += 1;
        }
    }
    assert_eq!(edge_total, 15, "expected 15 authored edges");
}

#[cfg(feature = "embed")]
#[test]
fn committed_l0_manifest_translates_to_embedded_request() {
    use temper_cli::actions::reconcile::manifest_to_request;

    let yaml = load_manifest_yaml();
    let doc = parse_manifest(&yaml).expect("committed L0 manifest parses");
    let req = manifest_to_request(&doc).expect("manifest translates to a reconcile request");

    assert_eq!(req.entries.len(), 22, "22 embedded entries");
    // Client-side embed ran for every entry: non-empty content hash + packed chunks.
    assert!(
        req.entries
            .iter()
            .all(|e| !e.content_hash.is_empty() && !e.chunks_packed.is_empty()),
        "every entry is pre-embedded (content_hash + chunks_packed)"
    );
    // Decision #6: provenance is server-stamped, never in the wire payload.
    assert!(
        req.entries
            .iter()
            .all(|e| e.facets.get("provenance").is_none()),
        "no entry carries a provenance facet"
    );
    // Additive-only first delivery.
    assert!(req.fold_resources.is_empty());
    assert!(req.fold_edges.is_empty());
}
