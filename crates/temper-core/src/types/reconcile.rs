//! Wire types for L0 cognitive-map content reconciliation (see
//! docs/superpowers/plans/2026-06-25-l0-delivery-and-lifecycle.md). The PUT body is a
//! PRE-EMBEDDED desired-state manifest: the CLI embeds (compute_body_chunks) before sending, so the
//! server stays embed-free on the request path.
//!
//! These are Rust-only CLI↔API wire types (the UI never touches them) — they mirror the exact
//! derive stack of `IngestPayload` (no ts-rs / schemars), adding `PartialEq` for round-trip
//! assertions. `ReconcileOutcome` additionally derives `Default`.

use serde::{Deserialize, Serialize};

/// One kernel landmark in a reconcile request — **pre-embedded** by the CLI.
///
/// `chunks_packed` is the `compute_body_chunks` output (the same packed-blob wire format as
/// `IngestPayload::chunks_packed`) and is the SOLE, AUTHORITATIVE body content — the entry carries no
/// raw `body` (the chunks already hold the prose; the authored prose lives in the manifest the CLI
/// reads). The diff/store both derive from `chunks_packed`, so there is no second body to disagree with.
/// `content_hash`
/// is ADVISORY only: the reconcile diff recomputes the body merkle server-side from `chunks_packed`
/// (the same `body_hash_from_chunk_hashes` the substrate stores) and never trusts this field — the CLI
/// fills it via `compute_body_hash` (a whole-body `sha256:`-prefixed hash) which does not equal the
/// stored chunk-merkle, so trusting it would re-block every entry on every run. `facets` is the
/// CLUSTERING facet object (e.g. `{ "layer": "concept" }`); `provenance: kernel` is server-stamped, not
/// carried here (see the reconcile design, Decision #6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ReconcileEntry {
    pub origin_uri: String,
    pub title: String,
    pub doc_type: String,
    pub content_hash: String,
    pub chunks_packed: String,
    pub facets: serde_json::Value,
    #[serde(default)]
    pub edges: Vec<ReconcileEdge>,
}

/// An outgoing edge from a kernel landmark, keyed by the target's `origin_uri`
/// (resolved server-side to the target resource id).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ReconcileEdge {
    pub to_origin_uri: String,
    pub kind: String,
    pub polarity: String,
    pub label: Option<String>,
    pub weight: f64,
}

/// Explicit resource-removal tombstone (O3: absence alone never folds).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ReconcileTombstone {
    pub origin_uri: String,
}

/// Explicit edge-removal tombstone, keyed by source + target `origin_uri` + kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ReconcileEdgeTombstone {
    pub from_origin_uri: String,
    pub to_origin_uri: String,
    pub kind: String,
}

/// The `PUT /api/cognitive-maps/{id}` request body — a desired-state manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ReconcileCogmapRequest {
    pub entries: Vec<ReconcileEntry>,
    #[serde(default)]
    pub fold_resources: Vec<ReconcileTombstone>,
    #[serde(default)]
    pub fold_edges: Vec<ReconcileEdgeTombstone>,
}

/// The result of one reconcile run — also serialized into `kb_invocations.outcome`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ReconcileOutcome {
    pub created: u32,
    pub updated: u32,
    pub folded: u32,
    pub unchanged: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_through_json() {
        let req = ReconcileCogmapRequest {
            entries: vec![ReconcileEntry {
                origin_uri: "temper://kernel/concept/cogmap".into(),
                title: "cogmap".into(),
                doc_type: "kernel_landmark".into(),
                content_hash: "deadbeef".into(),
                chunks_packed: "[]".into(),
                facets: serde_json::json!({ "provenance": "kernel", "layer": "concept" }),
                edges: vec![ReconcileEdge {
                    to_origin_uri: "temper://kernel/concept/telos".into(),
                    kind: "express".into(),
                    polarity: "forward".into(),
                    label: Some("governs".into()),
                    weight: 1.0,
                }],
            }],
            fold_resources: vec![],
            fold_edges: vec![],
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: ReconcileCogmapRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn outcome_default_is_all_zero() {
        let o = ReconcileOutcome::default();
        assert_eq!((o.created, o.updated, o.folded, o.unchanged), (0, 0, 0, 0));
    }
}
