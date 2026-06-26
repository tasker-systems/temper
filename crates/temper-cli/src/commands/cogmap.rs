//! `temper cogmap reconcile` — operator command to reconcile a cognitive map's content to a
//! committed manifest.
//!
//! Reads the authored manifest, embeds each entry CLIENT-SIDE (`compute_body_chunks`), builds a
//! pre-embedded `ReconcileCogmapRequest`, and PUTs it to `/api/cognitive-maps/{id}` (admin-gated,
//! idempotent). Prints the run outcome (`created`/`updated`/`folded`/`unchanged`).
//!
//! The whole path needs the `embed` feature (it runs ONNX). A non-embed build still compiles and
//! returns a clear "requires --features embed" error, mirroring `actions::search::embed_query`.

use crate::error::Result;
use crate::format::OutputFormat;

/// Reconcile the cognitive map addressed by `cogmap_ref` to the manifest at `manifest_path`.
#[cfg(feature = "embed")]
pub fn reconcile(cogmap_ref: &str, manifest_path: &str, fmt: OutputFormat) -> Result<()> {
    use crate::actions::reconcile as reconcile_action;
    use crate::error::TemperError;

    // Resolve the cogmap ref → UUID (trailing-UUID-only; the slug half is ignored).
    let cogmap_id = temper_workflow::operations::parse_ref(cogmap_ref)?.0;

    let yaml = std::fs::read_to_string(manifest_path)
        .map_err(|e| TemperError::Config(format!("reading manifest {manifest_path}: {e}")))?;
    let doc = reconcile_action::parse_manifest(&yaml)?;
    let req = reconcile_action::manifest_to_request(&doc)?;

    let outcome = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .cognitive_maps()
                .reconcile_cognitive_map(cogmap_id, &req)
                .await
                .map_err(crate::commands::client_err)
        })
    })?;

    let rendered = crate::format::render(&outcome, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// Non-embed build: the command cannot run (no ONNX to embed the manifest).
#[cfg(not(feature = "embed"))]
pub fn reconcile(_cogmap_ref: &str, _manifest_path: &str, _fmt: OutputFormat) -> Result<()> {
    Err(crate::error::TemperError::Config(
        "cogmap reconcile requires the 'embed' feature — rebuild with --features embed".into(),
    ))
}
