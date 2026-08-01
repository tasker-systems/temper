//! Shared paging fetch for `memory`-typed resources — used by both `status` (the discovery
//! report) and `emit` (the writer). One copy, not two: `list_meta` returns a capped page
//! (`ResourceMetaListResponse { rows, total, .. }`), and both callers need the true set rather
//! than a partial one — `status` to report an accurate count, `emit` because it is the one that
//! writes the file. A filter or cap added to one copy and not the other would let them silently
//! disagree about what the memory set *is*.

use temper_client::TemperClient;
use temper_workflow::types::resource::{ResourceDetail, ResourceListParams};

use crate::actions::runtime::client_err_to_temper;
use crate::error::Result;

pub(super) const MEMORY_DOC_TYPE: &str = "memory";

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
) -> Result<Vec<ResourceDetail>> {
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
