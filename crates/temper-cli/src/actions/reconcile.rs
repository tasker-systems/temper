//! Operator-side manifest model + the client-side embed bridge for
//! `temper cogmap reconcile`.
//!
//! The committed L0 manifest is authored *text* (bodies are prose). This module parses it
//! (serde_yaml, mirroring the workbench seed `resources:`/`edges:` shape) and bridges each entry to a
//! pre-embedded `ReconcileEntry` by running `compute_body_chunks` CLIENT-SIDE — exactly as
//! `temper resource create` embeds before sending. The server stays embed-free on the PUT path.
//!
//! Facet model (plan Decision #6): manifest facets are CLUSTERING only (e.g. `{layer: concept}`).
//! Provenance is *server-stamped* on create, never carried in the wire payload — so a manifest must
//! not (and the round-trip does not) inject a `provenance` key.

use crate::error::{Result, TemperError};

/// A parsed reconcile manifest — the desired-state document.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ManifestDoc {
    pub entries: Vec<ManifestEntry>,
    #[serde(default)]
    pub fold_resources: Vec<ManifestTombstone>,
    #[serde(default)]
    pub fold_edges: Vec<ManifestEdgeTombstone>,
}

/// One authored kernel landmark (pre-embed: body is raw prose).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ManifestEntry {
    /// The STABLE landmark identity (pre-generated uuidv7) — the reconcile diff key. `origin_uri` is
    /// pure attribution, never a key.
    pub id: uuid::Uuid,
    pub origin_uri: String,
    pub title: String,
    #[serde(default = "default_doc_type")]
    pub doc_type: String,
    pub body: String,
    /// Clustering facets only (e.g. `{"layer": "concept"}`). No provenance (server-stamped).
    #[serde(default)]
    pub facets: serde_json::Value,
    #[serde(default)]
    pub edges: Vec<ManifestEdge>,
}

/// An outgoing edge keyed by the target landmark's stable `id`.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ManifestEdge {
    pub to: uuid::Uuid,
    pub kind: String,
    #[serde(default = "default_polarity")]
    pub polarity: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default = "default_weight")]
    pub weight: f64,
}

/// Explicit resource-removal tombstone, keyed by the landmark's stable `id` (absence alone never folds).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ManifestTombstone {
    pub id: uuid::Uuid,
}

/// Explicit edge-removal tombstone, keyed by source + target landmark `id` + kind.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ManifestEdgeTombstone {
    pub from: uuid::Uuid,
    pub to: uuid::Uuid,
    pub kind: String,
}

fn default_doc_type() -> String {
    "kernel_landmark".to_string()
}

fn default_polarity() -> String {
    "forward".to_string()
}

fn default_weight() -> f64 {
    1.0
}

/// Parse a YAML manifest into the [`ManifestDoc`] model.
pub fn parse_manifest(yaml: &str) -> Result<ManifestDoc> {
    serde_yaml::from_str(yaml)
        .map_err(|e| TemperError::Config(format!("parsing reconcile manifest: {e}")))
}

/// Bridge a parsed manifest to a pre-embedded `ReconcileCogmapRequest`.
///
/// Each entry's body is embedded CLIENT-SIDE via `compute_body_chunks` (ONNX) into
/// `content_hash` + `chunks_packed`. Facets pass through as clustering-only — provenance is stamped
/// server-side (plan Decision #6), never in the wire payload.
#[cfg(feature = "embed")]
pub fn manifest_to_request(
    doc: &ManifestDoc,
) -> Result<temper_core::types::reconcile::ReconcileCogmapRequest> {
    use crate::actions::ingest::{compute_body_chunks, BodyChunks};
    use temper_core::types::reconcile::{
        ReconcileCogmapRequest, ReconcileEdge, ReconcileEdgeTombstone, ReconcileEntry,
        ReconcileTombstone,
    };

    let mut entries = Vec::with_capacity(doc.entries.len());
    for e in &doc.entries {
        let BodyChunks {
            content_hash,
            chunks_packed,
        } = compute_body_chunks(&e.body)?;
        entries.push(ReconcileEntry {
            id: e.id,
            origin_uri: e.origin_uri.clone(),
            title: e.title.clone(),
            doc_type: e.doc_type.clone(),
            content_hash,
            chunks_packed,
            facets: e.facets.clone(),
            edges: e
                .edges
                .iter()
                .map(|x| ReconcileEdge {
                    to: x.to,
                    kind: x.kind.clone(),
                    polarity: x.polarity.clone(),
                    label: x.label.clone(),
                    weight: x.weight,
                })
                .collect(),
        });
    }

    let fold_resources = doc
        .fold_resources
        .iter()
        .map(|t| ReconcileTombstone { id: t.id })
        .collect();
    let fold_edges = doc
        .fold_edges
        .iter()
        .map(|t| ReconcileEdgeTombstone {
            from: t.from,
            to: t.to,
            kind: t.kind.clone(),
        })
        .collect();

    Ok(ReconcileCogmapRequest {
        entries,
        fold_resources,
        fold_edges,
        telos: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TELOS_ID: &str = "019f03f4-2acf-7c45-bd12-a2a7152644a1";

    const SAMPLE_YAML: &str = r#"
entries:
  - id: "019f03f4-2ace-76cb-b1fc-260239dd16a5"
    origin_uri: "temper://kernel/concept/cogmap"
    title: "cogmap"
    body: "A cognitive map: a bounded, telos-governed view."
    facets:
      layer: concept
    edges:
      - to: "019f03f4-2acf-7c45-bd12-a2a7152644a1"
        kind: express
        label: governs
  - id: "019f03f4-2acf-7c45-bd12-a2a7152644a1"
    origin_uri: "temper://kernel/concept/telos"
    title: "telos"
    body: "A telos: the governing purpose a map expresses."
"#;

    #[test]
    fn parse_manifest_round_trips() {
        let doc = parse_manifest(SAMPLE_YAML).unwrap();
        assert_eq!(doc.entries.len(), 2);
        assert_eq!(doc.entries[0].origin_uri, "temper://kernel/concept/cogmap");
        // doc_type defaults to "kernel_landmark".
        assert_eq!(doc.entries[0].doc_type, "kernel_landmark");
        assert_eq!(doc.entries[0].facets["layer"], "concept");
        // One edge, with defaulted polarity/weight, keyed on the target landmark's stable id.
        assert_eq!(doc.entries[0].edges.len(), 1);
        assert_eq!(
            doc.entries[0].edges[0].to,
            TELOS_ID.parse::<uuid::Uuid>().unwrap()
        );
        assert_eq!(doc.entries[0].edges[0].polarity, "forward");
        assert_eq!(doc.entries[0].edges[0].weight, 1.0);
        assert_eq!(doc.entries[0].edges[0].label.as_deref(), Some("governs"));
        // Second entry has no edges.
        assert!(doc.entries[1].edges.is_empty());
        // No tombstones in this manifest.
        assert!(doc.fold_resources.is_empty());
        assert!(doc.fold_edges.is_empty());
    }

    #[cfg(feature = "test-embed")]
    #[test]
    fn manifest_to_request_embeds_each_entry() {
        let doc = parse_manifest(SAMPLE_YAML).unwrap();
        let req = manifest_to_request(&doc).unwrap();
        assert_eq!(req.entries.len(), 2);
        // Embed ran client-side: every entry carries a non-empty hash + packed chunks.
        assert!(req
            .entries
            .iter()
            .all(|e| !e.content_hash.is_empty() && !e.chunks_packed.is_empty()));
        // Decision #6: no provenance in the wire payload (server stamps it).
        assert!(req
            .entries
            .iter()
            .all(|e| e.facets.get("provenance").is_none()));
    }
}
