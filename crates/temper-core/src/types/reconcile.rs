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
/// `id` is the landmark's STABLE identity: a pre-generated uuidv7 the manifest author assigns once and
/// never changes. The reconcile diff matches manifest entry ↔ live resource by THIS id (never by
/// `origin_uri`), and on CREATE the resource is minted under it — so a duplicate id is a primary-key
/// conflict (fail-loud), not a silent twin. `origin_uri` stays as pure ATTRIBUTION (intentionally loose,
/// non-unique) and is set on the created resource, but it is NEVER a key.
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
    pub id: uuid::Uuid,
    pub origin_uri: String,
    pub title: String,
    pub doc_type: String,
    pub content_hash: String,
    pub chunks_packed: String,
    pub facets: serde_json::Value,
    #[serde(default)]
    pub edges: Vec<ReconcileEdge>,
}

/// An outgoing edge from a kernel landmark, keyed by the target landmark's stable `id` (the same
/// uuidv7 the target entry carries — no server-side resolution needed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ReconcileEdge {
    pub to: uuid::Uuid,
    pub kind: String,
    pub polarity: String,
    pub label: Option<String>,
    pub weight: f64,
}

/// Explicit resource-removal tombstone, keyed by the landmark's stable `id` (O3: absence alone never
/// folds).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ReconcileTombstone {
    pub id: uuid::Uuid,
}

/// Explicit edge-removal tombstone, keyed by source + target landmark `id` + kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ReconcileEdgeTombstone {
    pub from: uuid::Uuid,
    pub to: uuid::Uuid,
    pub kind: String,
}

/// One charter block in a telos delivery — **pre-embedded** by the CLI. `role` is the `block_role` the
/// substrate stamps so reads (`resource_blocks(telos, …, role)`) distinguish statement / question /
/// framing. `chunks_packed` is `compute_body_chunks(prose)` output for THIS block's prose — the same
/// packed-blob format `ReconcileEntry::chunks_packed` uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ReconcileTelosBlock {
    pub role: String,
    pub chunks_packed: String,
}

/// The telos charter as an ordered run of pre-embedded blocks (block-0 statement, then questions, then
/// framing). Optional on a reconcile request: absent ⇒ landmark-only reconcile (`charter: Absent`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ReconcileTelos {
    pub blocks: Vec<ReconcileTelosBlock>,
}

/// What the reconcile run did to the telos charter — a DISTINCT grain from the landmark counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CharterDisposition {
    /// The request carried no `telos:` (landmark-only reconcile).
    #[default]
    Absent,
    /// The telos body-merkle matched the live charter — no event fired.
    Unchanged,
    /// The telos was empty (first delivery) and the charter was created.
    Created,
    /// The live charter differed and was replaced.
    Updated,
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
    #[serde(default)]
    pub telos: Option<ReconcileTelos>,
}

/// The `POST /api/cognitive-maps` request body — a cognitive-map **genesis** (create) manifest.
///
/// Genesis identity is manifest-supplied uuidv7: `cogmap_id` is the new map's id and
/// `telos_resource_id` is its telos charter resource's id. Both are `Option` — when absent the backend
/// mints a fresh `Uuid::now_v7()` (mirroring `CreateResource`'s identity-as-input precedent). Supplying
/// them makes genesis reproducible (the operator gets a stable id) and is how the reserved-id L0 kernel
/// is born.
///
/// `name` is the cogmap's name; `telos_title` is the telos charter resource's title. `telos` is the
/// optional pre-embedded charter (same role-tagged block shape `ReconcileCogmapRequest::telos` carries):
/// absent ⇒ the map is born with an empty charter (deliverable later via reconcile). Genesis is
/// idempotent at a given id — re-genesis is a no-op returning `created: false`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct CreateCogmapRequest {
    #[serde(default)]
    pub cogmap_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub telos_resource_id: Option<uuid::Uuid>,
    pub name: String,
    pub telos_title: String,
    #[serde(default)]
    pub telos: Option<ReconcileTelos>,
}

/// The result of one genesis run — the realized identity plus whether this call created it.
/// `created: false` means the map already existed at `cogmap_id` (idempotent no-op).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct CreateCogmapOutcome {
    pub cogmap_id: uuid::Uuid,
    pub telos_resource_id: uuid::Uuid,
    pub created: bool,
}

/// The result of one reconcile run — also serialized into `kb_invocations.outcome`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ReconcileOutcome {
    pub created: u32,
    pub updated: u32,
    pub folded: u32,
    pub unchanged: u32,
    pub charter: CharterDisposition,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_through_json() {
        let telos_id = uuid::Uuid::now_v7();
        let req = ReconcileCogmapRequest {
            entries: vec![ReconcileEntry {
                id: uuid::Uuid::now_v7(),
                origin_uri: "temper://kernel/concept/cogmap".into(),
                title: "cogmap".into(),
                doc_type: "kernel_landmark".into(),
                content_hash: "deadbeef".into(),
                chunks_packed: "[]".into(),
                facets: serde_json::json!({ "provenance": "kernel", "layer": "concept" }),
                edges: vec![ReconcileEdge {
                    to: telos_id,
                    kind: "express".into(),
                    polarity: "forward".into(),
                    label: Some("governs".into()),
                    weight: 1.0,
                }],
            }],
            fold_resources: vec![],
            fold_edges: vec![],
            telos: None,
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

    #[test]
    fn request_carries_optional_telos() {
        let req = ReconcileCogmapRequest {
            entries: vec![],
            fold_resources: vec![],
            fold_edges: vec![],
            telos: Some(ReconcileTelos {
                blocks: vec![
                    ReconcileTelosBlock {
                        role: "statement".into(),
                        chunks_packed: "[]".into(),
                    },
                    ReconcileTelosBlock {
                        role: "question".into(),
                        chunks_packed: "[]".into(),
                    },
                ],
            }),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: ReconcileCogmapRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
        // A request with no `telos:` round-trips with `None` (landmark-only, backward compatible).
        let landmark_only: ReconcileCogmapRequest =
            serde_json::from_str(r#"{"entries":[]}"#).unwrap();
        assert!(landmark_only.telos.is_none());
    }

    #[test]
    fn create_request_round_trips_with_optional_ids_and_telos() {
        let req = CreateCogmapRequest {
            cogmap_id: Some(uuid::Uuid::now_v7()),
            telos_resource_id: Some(uuid::Uuid::now_v7()),
            name: "Org provisioning map".into(),
            telos_title: "Org telos".into(),
            telos: Some(ReconcileTelos {
                blocks: vec![ReconcileTelosBlock {
                    role: "statement".into(),
                    chunks_packed: "[]".into(),
                }],
            }),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: CreateCogmapRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);

        // Ids and telos all default to None/absent — a minimal genesis request.
        let minimal: CreateCogmapRequest =
            serde_json::from_str(r#"{"name":"m","telos_title":"t"}"#).unwrap();
        assert!(minimal.cogmap_id.is_none());
        assert!(minimal.telos_resource_id.is_none());
        assert!(minimal.telos.is_none());
    }

    #[test]
    fn create_outcome_round_trips() {
        let o = CreateCogmapOutcome {
            cogmap_id: uuid::Uuid::now_v7(),
            telos_resource_id: uuid::Uuid::now_v7(),
            created: true,
        };
        let json = serde_json::to_string(&o).unwrap();
        assert_eq!(
            serde_json::from_str::<CreateCogmapOutcome>(&json).unwrap(),
            o
        );
    }

    #[test]
    fn charter_disposition_defaults_absent_and_serializes_snake_case() {
        assert_eq!(CharterDisposition::default(), CharterDisposition::Absent);
        assert_eq!(
            serde_json::to_string(&CharterDisposition::Updated).unwrap(),
            "\"updated\""
        );
        // Outcome default carries charter: Absent.
        assert_eq!(
            ReconcileOutcome::default().charter,
            CharterDisposition::Absent
        );
    }
}
