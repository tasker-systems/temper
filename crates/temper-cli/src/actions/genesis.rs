//! Operator-side manifest model + the client-side embed bridge for `temper cogmap create` (genesis).
//!
//! Genesis births a NEW cognitive map (cogmap + telos charter resource) from an authored manifest. Like
//! reconcile, the charter is authored *prose* and embedded CLIENT-SIDE (`compute_body_chunks`) so the
//! server stays embed-free on the POST path. Identity (`cogmap_id` / `telos_resource_id`) is
//! manifest-supplied uuidv7 — when absent the CLI mints `Uuid::now_v7()` so the operator gets a stable,
//! reproducible id (rather than letting the server mint an opaque one).

use crate::error::{Result, TemperError};

use crate::actions::reconcile::ManifestTelos;

/// A parsed genesis manifest — the new map's desired birth state.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct GenesisManifestDoc {
    /// The new map's id (uuidv7). Absent ⇒ the CLI mints one.
    #[serde(default)]
    pub cogmap_id: Option<uuid::Uuid>,
    /// The telos charter resource's id (uuidv7). Absent ⇒ the CLI mints one.
    #[serde(default)]
    pub telos_resource_id: Option<uuid::Uuid>,
    /// The cognitive map's name.
    pub name: String,
    /// The telos charter resource's title.
    pub telos_title: String,
    /// The authored telos charter (statement / questions / framing). Absent ⇒ the map is born with an
    /// empty charter (deliverable later via `temper cogmap reconcile`).
    #[serde(default)]
    pub telos: Option<ManifestTelos>,
}

/// Parse a YAML genesis manifest into the [`GenesisManifestDoc`] model.
pub fn parse_manifest(yaml: &str) -> Result<GenesisManifestDoc> {
    serde_yaml::from_str(yaml)
        .map_err(|e| TemperError::Config(format!("parsing genesis manifest: {e}")))
}

/// Bridge a parsed genesis manifest to a pre-embedded `CreateCogmapRequest`.
///
/// The telos charter is embedded CLIENT-SIDE (`embed_telos`, ONNX). Ids resolve as `flag → manifest →
/// freshly-minted uuidv7`: `id_override`/`name_override` come from CLI flags and win when present.
#[cfg(feature = "embed")]
pub fn manifest_to_request(
    doc: &GenesisManifestDoc,
    id_override: Option<uuid::Uuid>,
    name_override: Option<&str>,
) -> Result<temper_core::types::reconcile::CreateCogmapRequest> {
    use crate::actions::reconcile::embed_telos;

    let cogmap_id = id_override
        .or(doc.cogmap_id)
        .unwrap_or_else(uuid::Uuid::now_v7);
    let telos_resource_id = doc.telos_resource_id.unwrap_or_else(uuid::Uuid::now_v7);
    let name = name_override.map(str::to_owned).unwrap_or(doc.name.clone());

    let telos = doc.telos.as_ref().map(embed_telos).transpose()?;

    Ok(temper_core::types::reconcile::CreateCogmapRequest {
        cogmap_id: Some(cogmap_id),
        telos_resource_id: Some(telos_resource_id),
        name,
        telos_title: doc.telos_title.clone(),
        telos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_YAML: &str = r#"
name: "Org provisioning map"
telos_title: "Org telos"
telos:
  statement: "Orient an arriving agent."
  questions:
    - question: "Where am I?"
      context: "the first thing any agent asks"
  framing:
    - "Self-referential."
"#;

    #[test]
    fn parse_manifest_round_trips() {
        let doc = parse_manifest(SAMPLE_YAML).unwrap();
        assert_eq!(doc.name, "Org provisioning map");
        assert_eq!(doc.telos_title, "Org telos");
        // No ids in the manifest → the CLI will mint them.
        assert!(doc.cogmap_id.is_none());
        assert!(doc.telos_resource_id.is_none());
        let telos = doc.telos.as_ref().unwrap();
        assert_eq!(telos.statement, "Orient an arriving agent.");
        assert_eq!(telos.questions.len(), 1);
        assert_eq!(telos.framing, vec!["Self-referential.".to_string()]);
    }

    const WITH_IDS_YAML: &str = r#"
cogmap_id: "019f03f4-2ace-76cb-b1fc-260239dd16a5"
telos_resource_id: "019f03f4-2acf-7c45-bd12-a2a7152644a1"
name: "Fixed map"
telos_title: "Fixed telos"
"#;

    #[test]
    fn parse_manifest_reads_supplied_ids_and_optional_telos() {
        let doc = parse_manifest(WITH_IDS_YAML).unwrap();
        assert_eq!(
            doc.cogmap_id.unwrap().to_string(),
            "019f03f4-2ace-76cb-b1fc-260239dd16a5"
        );
        assert_eq!(
            doc.telos_resource_id.unwrap().to_string(),
            "019f03f4-2acf-7c45-bd12-a2a7152644a1"
        );
        // No telos → an empty-charter genesis.
        assert!(doc.telos.is_none());
    }

    #[cfg(feature = "test-embed")]
    #[test]
    fn manifest_to_request_embeds_telos_and_mints_ids_when_absent() {
        let doc = parse_manifest(SAMPLE_YAML).unwrap();
        let req = manifest_to_request(&doc, None, None).unwrap();
        // Ids minted client-side.
        assert!(req.cogmap_id.is_some());
        assert!(req.telos_resource_id.is_some());
        assert_eq!(req.name, "Org provisioning map");
        // statement + 1 question + 1 framing = 3 embedded blocks.
        let blocks = &req.telos.unwrap().blocks;
        assert_eq!(blocks.len(), 3);
        assert!(blocks.iter().all(|b| !b.chunks_packed.is_empty()));
    }

    #[cfg(feature = "test-embed")]
    #[test]
    fn manifest_to_request_honors_flag_overrides() {
        let doc = parse_manifest(SAMPLE_YAML).unwrap();
        let forced = uuid::Uuid::now_v7();
        let req = manifest_to_request(&doc, Some(forced), Some("Renamed")).unwrap();
        assert_eq!(req.cogmap_id, Some(forced));
        assert_eq!(req.name, "Renamed");
    }
}
