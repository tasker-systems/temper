//! Blob put/get/list/relate actions.
//!
//! `put` is the fat one: it decides single-request vs segmented (the D7 threshold is a
//! CLIENT-side choice — the CLI cannot read the server's config, so it carries its own
//! `BLOB_SINGLE_REQUEST_MAX_BYTES`, defaulting to the server's 4 MB default, both
//! deliberately under the platform's 4.5 MB request cap), chunks the segmented path under
//! the same bound, sha256s each segment (the idempotent-append identity) and the whole
//! (the finalize integrity echo), and carries the server-handed progress tokens into
//! finalize verbatim. Everything else is a thin client call.

use std::path::PathBuf;

use temper_client::TemperClient;
use temper_core::error::TemperError;
use temper_core::hash::sha256_hex;
use temper_core::types::blob::{BlobUploadBeginRequest, BlobUploadFinalizeRequest};

use crate::cli::{CliHomeTable, CliPeerTable};

/// The single-request threshold the put decision rides: bodies at or under this are ONE
/// multipart call; beyond it, segmented. Overridable via `BLOB_SINGLE_REQUEST_MAX_BYTES`
/// (bytes) to match a server configured below the default. Deliberately under the
/// platform's hard 4.5 MB request-body cap either way — the threshold is the vocabulary,
/// the platform cap is never the thing a user discovers.
pub const DEFAULT_SINGLE_REQUEST_MAX_BYTES: usize = 4_000_000;

/// The single-request threshold in force, from the env or the default. A malformed value
/// is refused here, loudly — silently falling back to the default would put chunks on the
/// wire that the configured server refuses.
pub fn single_request_max_bytes() -> Result<usize, TemperError> {
    match std::env::var("BLOB_SINGLE_REQUEST_MAX_BYTES") {
        Ok(raw) => raw.trim().parse::<usize>().map_err(|_| {
            TemperError::Config(format!(
                "BLOB_SINGLE_REQUEST_MAX_BYTES must be a byte count — got {raw:?}"
            ))
        }),
        Err(_) => Ok(DEFAULT_SINGLE_REQUEST_MAX_BYTES),
    }
}

/// The six allowlisted media types (D9's seeded set), by extension — the guess is a
/// convenience, never an authority: the server's allowlist refusal is what teaches.
fn guess_content_type(path: &str) -> Option<&'static str> {
    let ext = PathBuf::from(path)
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        "gif" => Some("image/gif"),
        "pdf" => Some("application/pdf"),
        _ => None,
    }
}

/// A resolved home anchor: the table spelled as the wire does, plus the id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedHome {
    pub table: &'static str,
    pub id: uuid::Uuid,
}

/// Resolve `--home` into a home anchor. A name-based ref (`@me/notes`, `+team/shared`) is
/// a CONTEXT by construction and resolves through the caller's readable contexts (with
/// `@me` normalized to the authenticated profile's own handle). A bare or decorated UUID
/// is used as-is, the table taken from `--home-table` — the server gates readability and
/// authorability either way, so a wrong guess refuses with the standing vocabulary, not a
/// silent write into the wrong anchor kind.
pub async fn resolve_home(
    client: &TemperClient,
    home: &str,
    home_table: CliHomeTable,
) -> Result<ResolvedHome, TemperError> {
    // Bare UUID (parse_ref resolves a decorated `slug-<uuid>` to its trailing UUID).
    if let Ok(parsed) = temper_workflow::operations::parse_ref(home) {
        return Ok(ResolvedHome {
            table: home_table.as_str(),
            id: parsed.0,
        });
    }

    // Name-based ref: contexts only. `@me` is the authenticated profile — the list rows
    // carry the sigil'd owner (`@<handle>` / `+<team>`), so `@me` needs the handle.
    let (owner_part, slug) = home.split_once('/').ok_or_else(|| unknown_home(home))?;
    let owner_part = owner_part.to_ascii_lowercase();
    let owner_ref = if owner_part == "@me" {
        let me = client
            .profile()
            .get()
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?;
        format!("@{}", me.slug)
    } else {
        owner_part
    };

    let rows = client
        .contexts()
        .list()
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    rows.iter()
        .find(|c| c.slug == slug && c.owner_ref.eq_ignore_ascii_case(&owner_ref))
        .map(|c| ResolvedHome {
            table: "kb_contexts",
            id: c.id.0,
        })
        .ok_or_else(|| unknown_home(home))
}

fn unknown_home(home: &str) -> TemperError {
    TemperError::Config(format!(
        "unknown home anchor {home:?} — use a context ref (@me/notes, +team/shared) or an \
         anchor id; check `temper context list` for what you can reach"
    ))
}

/// What the caller hands `put`: the bytes plus the identity they commit under. The home
/// rides `put`'s own parameter — it is resolved separately and is not the bytes' business.
pub struct PutParams {
    pub content_type: String,
    pub filename: String,
    pub bytes: Vec<u8>,
}

/// Assemble the put's identity: read the bytes (file or stdin), guess or take the media
/// type, default the filename. The client-side half of the refusal vocabulary: an unknown
/// extension with no `--content-type` refuses HERE, naming the allowlisted set, because
/// committing under a guessed-wrong type would only move the refusal server-side where it
/// teaches less.
pub fn prepare_put(
    file: &str,
    content_type: Option<&str>,
    filename: Option<&str>,
) -> Result<PutParams, TemperError> {
    let bytes = if file == "-" {
        use std::io::Read;
        let mut buf = Vec::new();
        std::io::stdin()
            .read_to_end(&mut buf)
            .map_err(|e| TemperError::Config(format!("read stdin: {e}")))?;
        buf
    } else {
        std::fs::read(file).map_err(|e| {
            TemperError::Config(format!(
                "read {file}: {e} — a blob commits a real file's bytes"
            ))
        })?
    };

    let guessed = (file != "-").then(|| guess_content_type(file)).flatten();
    let content_type = match (content_type, guessed) {
        (Some(ct), _) => ct.to_string(),
        (None, Some(g)) => g.to_string(),
        (None, None) => {
            return Err(TemperError::Config(
                "cannot guess the media type — pass --content-type (the allowlist in force \
                 covers png, jpeg, webp, svg, gif, pdf)"
                    .to_string(),
            ))
        }
    };

    let filename = match filename {
        Some(f) => f.to_string(),
        None if file != "-" => PathBuf::from(file)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| {
                TemperError::Config(format!(
                    "cannot derive a filename from {file:?} — pass --filename"
                ))
            })?,
        None => {
            return Err(TemperError::Config(
                "stdin commits have no filename — pass --filename".to_string(),
            ))
        }
    };

    Ok(PutParams {
        content_type,
        filename,
        bytes,
    })
}

/// Commit bytes: one multipart call at or under the threshold, the segmented
/// begin/append/finalize sequence beyond it. Returns the server's commit response either
/// way (the segmented finalize returns the same shape).
pub async fn put(
    client: &TemperClient,
    home: ResolvedHome,
    params: &PutParams,
    on_segment: impl Fn(usize, usize, u64),
) -> Result<temper_core::types::blob::BlobCommitResponse, TemperError> {
    let threshold = single_request_max_bytes()?;
    if params.bytes.len() <= threshold {
        return client
            .blobs()
            .commit(
                home.table,
                home.id,
                &params.content_type,
                &params.filename,
                params.bytes.clone(),
            )
            .await
            .map_err(crate::actions::runtime::client_err_to_temper);
    }

    // Segmented: chunk under the SAME bound (each segment is a request body of its own,
    // so it must also respect the threshold), whole-file sha256 into finalize as the
    // integrity check. The segment identity is the server's own hash of the bytes it
    // receives; the progress tokens are the SERVER's — each append response carries the
    // landed set, and finalize echoes the LAST one verbatim.
    let whole_hash = sha256_hex(&params.bytes);
    let upload_id = client
        .blobs()
        .begin(&BlobUploadBeginRequest {
            home_table: home.table.to_string(),
            home_id: home.id,
            content_type: params.content_type.clone(),
        })
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?
        .upload_id;

    let chunk_size = threshold;
    let total_chunks = params.bytes.len().div_ceil(chunk_size);
    let mut last_progress = None;
    for (seq, chunk) in params.bytes.chunks(chunk_size).enumerate() {
        let progress = client
            .blobs()
            .append(upload_id, seq as u32, chunk.to_vec())
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?;
        on_segment(seq + 1, total_chunks, progress.total_bytes as u64);
        last_progress = Some(progress);
    }
    let progress =
        last_progress.expect("segmented path runs only with at least one segment to send");

    client
        .blobs()
        .finalize(
            upload_id,
            &BlobUploadFinalizeRequest {
                expected_segments: progress.segments.len() as u32,
                expected_total_bytes: progress.total_bytes,
                expected_content_hash: Some(whole_hash),
            },
        )
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)
}

/// Resolve `--to` into the peer the wire wants: the table spelled as the DDL does, plus
/// the endpoint id. `--peer-table` defaults to resource — the decorated-ref case — and is
/// the explicit override for bare cogmap/blob ids; the flag is the single source of the
/// table.
pub fn resolve_peer(
    to: &str,
    peer_table: CliPeerTable,
) -> Result<(&'static str, uuid::Uuid), TemperError> {
    let peer_id = temper_workflow::operations::parse_ref(to)
        .map_err(|e| {
            TemperError::Config(format!(
                "--to must be a resource ref (UUID or slug-<uuid>) or an anchor id with \
                 --peer-table: {e}"
            ))
        })?
        .0;
    Ok((peer_table.as_str(), peer_id))
}
