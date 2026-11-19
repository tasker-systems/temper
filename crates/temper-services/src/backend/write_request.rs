//! Refusals a resource-write request earns from its shape alone, before any command is built.
//!
//! The server is the only enforcing copy of request shaping: a door (the CLI, MCP) keeps only its
//! own parse errors, so every rule a door used to check before sending lives here, where every
//! door's request passes. Each refusal names the field the caller can correct.
//!
//! These run on the wire request because the command a handler builds from it cannot express the
//! mistake: a create's sources ride inside its body, so a create with no body has nowhere to put
//! them, and a patch with no `content` builds no body at all. Building the command first would
//! drop the field silently.

use temper_core::error::TemperError;
use temper_core::types::ingest::IngestPayload;
use temper_core::types::resource::ResourceUpdateRequest;

/// A create's sources name the blocks its body was distilled from; with no body there is nothing
/// to attribute.
pub const SOURCES_WITHOUT_CONTENT: &str =
    "sources supplied without content — there is no body block to attribute";

/// A patch's `content_block` addresses which block its body revise writes; with no `content`
/// there is no revise to address.
pub const CONTENT_BLOCK_WITHOUT_CONTENT: &str =
    "content_block supplied without content — there is no body revise to address";

/// A create has exactly one home.
pub const TWO_HOMES: &str =
    "context_ref and home_cogmap_id are mutually exclusive; supply exactly one home";

/// Refuse an `IngestPayload` (`POST /api/ingest`, one-shot or segmented begin) whose shape no
/// create can honour.
///
/// - `sources` with an empty `content`: an empty body is no body, so the sources would be dropped.
/// - a non-empty `context_ref` together with `home_cogmap_id`: two homes.
pub fn check_ingest_payload(payload: &IngestPayload) -> Result<(), TemperError> {
    if payload.home_cogmap_id.is_some() && !payload.context_ref.is_empty() {
        return Err(TemperError::BadRequest(TWO_HOMES.to_owned()));
    }
    if payload.content.is_empty() && !payload.sources.is_empty() {
        return Err(TemperError::BadRequest(SOURCES_WITHOUT_CONTENT.to_owned()));
    }
    Ok(())
}

/// Refuse a `ResourceUpdateRequest` (`PATCH /api/resources/{id}`) that carries body-addressing
/// fields with no `content`: there is no body revise for them to ride on.
///
/// Only an absent `content` is refused. A present but empty `content` is a body write the backend
/// judges itself, so it is not second-guessed here.
pub fn check_update_request(req: &ResourceUpdateRequest) -> Result<(), TemperError> {
    if req.content.is_none() {
        if !req.sources.is_empty() {
            return Err(TemperError::BadRequest(SOURCES_WITHOUT_CONTENT.to_owned()));
        }
        if req.content_block.is_some() {
            return Err(TemperError::BadRequest(
                CONTENT_BLOCK_WITHOUT_CONTENT.to_owned(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
