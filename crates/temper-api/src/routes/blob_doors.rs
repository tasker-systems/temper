//! The two blob doors whose legal request bodies exceed axum's inherited default, and the
//! body limits sized from the config and the plan numbers.

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::handlers;
use temper_services::state::AppState;

/// `/api/blobs/uploads/{id}/segments` and `POST /api/blobs` — the two blob doors whose
/// legal request bodies exceed axum's inherited 2 MB default. The staging ceiling (the
/// cumulative bound across appends) is `BlobConfig::max_bytes`, enforced with its own
/// vocabulary in the service; the single-request commit's bound is the D7 threshold,
/// enforced with ITS vocabulary by the handler while the file field streams. The body
/// limits are transport plumbing sized FROM those vocabularies (the segment door by the
/// platform ceiling the plan numbers pin, the commit door by the configured threshold
/// itself — see `blob_commit_routes`), applied as `DefaultBodyLimit` layers at the
/// table mount: a `DefaultBodyLimit` applies to every route in the router it is
/// attached to, and the blob doors that accept bodies are exactly these two (every other
/// blob route is JSON of trivial size or body-free). They stay inside `gated_routes`'
/// auth layers, applied by the table's `Gated` tier.
pub(super) const BLOB_SEGMENT_MAX_BODY_BYTES: usize =
    temper_services::transport::VERCEL_REQUEST_BODY_CAP_BYTES;

pub(super) fn blob_segment_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(handlers::blobs::append_segment))
}

/// Multipart framing overhead above the file field's own bytes: the boundary, the two
/// text fields, and per-part headers. 64 KiB is orders of magnitude beyond any real
/// multipart envelope for this two-field form.
pub(super) const BLOB_COMMIT_MULTIPART_OVERHEAD_BYTES: usize = 64 * 1024;

/// `POST /api/blobs` — the single-request commit door. F8: mounted plain, the door
/// inherited the app-wide 2 MB default while its vocabulary allows bodies to the D7
/// threshold — a legal 2–4 MB upload died at the transport as a misleading
/// `malformed multipart body` instead of the threshold refusal. The transport bound is
/// [`blob_commit_body_limit`], applied as a layer at the table mount: the threshold
/// itself plus multipart overhead, so the transport never fires first — any body over
/// the threshold is refused by the handler's mid-stream check (which names the threshold
/// and the segmented path) before the body reaches the transport's number. Derived from
/// the config per app-build, so an operator who raises the threshold raises the
/// transport bound with it — one number, never two to drift.
pub(super) fn blob_commit_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(handlers::blobs::commit))
}

/// The commit door's transport bound: the config's D7 threshold plus the multipart
/// envelope, floored for the unconfigured case (the door refuses on its own vocabulary
/// there; the transport number is then irrelevant, and the plan default keeps the shape
/// honest).
pub(super) fn blob_commit_body_limit(state: &AppState) -> usize {
    state
        .config
        .blob
        .as_ref()
        .map(|b| b.single_request_max_bytes + BLOB_COMMIT_MULTIPART_OVERHEAD_BYTES)
        .unwrap_or(BLOB_SEGMENT_MAX_BODY_BYTES + BLOB_COMMIT_MULTIPART_OVERHEAD_BYTES)
}
