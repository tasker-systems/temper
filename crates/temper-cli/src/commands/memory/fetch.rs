//! Shared paging fetch for `memory`-typed resources — used by both `status` (the discovery
//! report) and `emit` (the writer). One copy, not two: `list_meta` returns a capped page
//! (`ResourceMetaListResponse { rows, total, .. }`), and both callers need the true set rather
//! than a partial one — `status` to report an accurate count, `emit` because it is the one that
//! writes the file. A filter or cap added to one copy and not the other would let them silently
//! disagree about what the memory set *is*.

use temper_client::TemperClient;
use temper_core::types::resource_view::ResourceView;
use temper_workflow::types::resource::ResourceListParams;

use super::render::{statement_of, PrincipleSection};
use crate::actions::runtime::client_err_to_temper;
use crate::error::Result;

pub(super) const MEMORY_DOC_TYPE: &str = "memory";

/// The doc type whose resources section the index. It is an ordinary context resource, discovered
/// by listing the contexts `[memory]` already names — there is no principle list in config, and
/// deliberately so: a config-side list would be a second place a principle lives, and the one that
/// silently disagrees with the store.
pub(super) const PRINCIPLE_DOC_TYPE: &str = "principle";

/// The edge label that means "this memory is evidence for this principle".
///
/// Read as a **label**, not an edge kind. `--edge-type` elsewhere in this CLI takes a label and
/// filters on kind, which is a known trap; here the kind (`express`) is the mechanical carrier and
/// the label is what carries the meaning, so the label is what this matches on.
pub(super) const EVIDENCES_LABEL: &str = "evidences";

/// Page size for the `list_meta` walk in [`fetch_context_rows`]. Larger than the CLI's own
/// browsing default (`DEFAULT_META_LIST_LIMIT`, 50, in `commands/resource.rs`) because this walk
/// exists to produce an accurate, complete set, not a page to browse.
pub(super) const MEMORY_PAGE_SIZE: i64 = 200;

/// Fetch every `memory`-typed resource in `context_ref`, paging through the full result set.
///
/// Reporting `rows.len()` as the whole count would silently understate the result the moment a
/// context holds more than one page's worth of memories — this pages to the true `total` rather
/// than surfacing a partial page as if it were complete.
pub(super) async fn fetch_context_rows(
    client: &TemperClient,
    context_ref: &str,
) -> Result<Vec<ResourceView>> {
    let mut rows = Vec::new();
    let mut offset: i64 = 0;
    loop {
        let params = ResourceListParams {
            doc_type_name: Some(MEMORY_DOC_TYPE.to_string()),
            context_ref: Some(context_ref.to_string()),
            limit: Some(MEMORY_PAGE_SIZE),
            offset: Some(offset),
            ..Default::default()
        };
        let response = client
            .resources()
            .list_meta(&params)
            .await
            .map_err(client_err_to_temper)?;
        let fetched = response.rows.len() as i64;
        rows.extend(response.rows);
        offset += fetched;
        if fetched == 0 || offset >= response.total {
            break;
        }
    }
    Ok(rows)
}

/// Fetch every principle in `context_ref`, with its statement and its members.
///
/// **Read from the principle side — N calls, not one per memory.** The edge is asserted
/// memory → principle, so the membership set is reachable from either end; measured on the real
/// corpus that is 8 principles against 295 memories, so reading from the principle end costs
/// `2N + 1` requests instead of one per memory. The body is a second call because `list_meta`
/// carries no content by construction and the statement is the body's first paragraph.
///
/// A principle whose body cannot be fetched is **not** a defect: it contributes a section with no
/// preamble rather than failing the whole index. The grouping is the load-bearing part; the
/// preamble is the explanation, and losing an explanation must not cost a cold session its index.
pub(super) async fn fetch_principles(
    client: &TemperClient,
    context_ref: &str,
) -> Result<Vec<PrincipleSection>> {
    let params = ResourceListParams {
        doc_type_name: Some(PRINCIPLE_DOC_TYPE.to_string()),
        context_ref: Some(context_ref.to_string()),
        limit: Some(MEMORY_PAGE_SIZE),
        ..Default::default()
    };
    let listed = client
        .resources()
        .list_meta(&params)
        .await
        .map_err(client_err_to_temper)?;

    let mut sections = Vec::with_capacity(listed.rows.len());
    for row in listed.rows {
        let id = row.id.uuid();
        let edges = client
            .resources()
            .edges(id)
            .await
            .map_err(client_err_to_temper)?;
        // `incoming` is the memory → principle direction. Filtering on direction as well as label
        // keeps a principle that ever cites another principle from absorbing it as a member.
        let members = edges
            .iter()
            .filter(|e| e.direction == "incoming" && e.label == EVIDENCES_LABEL)
            .map(|e| e.peer_resource_id.uuid())
            .collect();
        let statement = client
            .resources()
            .content(id)
            .await
            .ok()
            .and_then(|c| statement_of(&c.markdown));
        sections.push(PrincipleSection {
            id,
            title: row.title,
            statement,
            members,
        });
    }
    Ok(sections)
}
