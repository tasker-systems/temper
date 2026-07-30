//! The resource facet read — the confirming read for a consequential write.
//!
//! A facet asserted on a resource steers region formation
//! (`temper-substrate/src/substrate.rs:110-122` → `expand_facets` → clustering) and Atlas grouping
//! (`context_graph_service.rs:105`). Until this read existed, the principal that asserted one could
//! not see it back faithfully through any door.
//!
//! **"Could not see it back *faithfully*" — not "could not see it".** The distinction matters for
//! anyone reading this module to work out why it exists. `get_meta` collapses every non-folded
//! `kb_properties` row into a map keyed by `property_key`
//! (`temper-substrate/src/readback/mod.rs:264-300`), so a facet has always surfaced inside
//! `open_meta` — as one object, newest-wins, with the weight discarded and any sibling row hidden.
//! That is worse than silence: it reads as complete. Production carries 30 resources with 2 or 3 live
//! facet rows, of which `open_meta` discloses one, and 128 rows with a non-default weight, of which
//! it discloses none.
//!
//! Goal `019fafd9-a978-7860-ae39-23958b4471b8`.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::error::TemperError;
use temper_core::types::facet_requests::ResourceFacetRow;
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_substrate::readback;

/// The refusal both denial arms render, verbatim. One constant rather than two call sites spelling
/// their own message, because the guarantee is that the arms are **indistinguishable** — and two
/// string literals that must stay equal are two literals that will drift.
const FACET_REFUSAL: &str = "resource not found or not readable";

/// Read the live facets of one resource — one row per live `kb_properties` row, no collapse.
///
/// **The gate is asked separately and first**, exactly as `citation_audit_service`'s trail read asks
/// it: [`readback::is_resource_visible`] is the Rust-callable spelling of `resources_visible_to`, so
/// nothing here restates an access predicate. A collection surface has to tell an honest caller's
/// empty result from a refusal, and folding the gate into the row query would answer *unreadable*,
/// *absent* and *readable-but-empty* identically with zero rows.
///
/// **So `200 []` for readable-and-empty, `404` for unreadable-or-absent.** That is not an existence
/// oracle: the empty list is only reachable *after* passing the readability gate, so it discloses
/// nothing a caller could not already learn from `GET /api/resources/{id}`. The register left this
/// shape question open for the build to settle; settled here, CONFORMing to the citation-audit
/// precedent rather than to the bare collection default.
///
/// **Scoped to `property_key = 'facet'` on purpose.** A resource's other property rows *are* its
/// frontmatter, and `get_meta` already serves them; returning them here would be a second, divergent
/// copy of a shipped surface. The row still carries its `property_key` so widening this read later is
/// additive.
///
/// **Attribution is joined here rather than left to the caller**, for the same reason
/// `edge_service::list_edge_facets` joins it: a `property_asserted` payload carries
/// `owner.{table,id}` rather than a resource id, so the lineage surfaces that answer *"what happened
/// to this resource"* are structurally blind to its facets. This read is the only place a facet's
/// author is recoverable — and *"who asserted this, and how heavily"* is the disclosure that answers
/// the question the goal exists for: why did this region form this way.
///
/// Ordered `(property_key, created)` — oldest-first within a key, so two live rows for one logical
/// facet arrive in the order they were asserted. This read takes no position on which of them *wins*;
/// facet supersession is task `019f6d08-2b55-7ee0-b9ac-1959cf4d736b`, and answering it here would
/// bake a guess into a wire contract.
pub async fn list_resource_facets(
    pool: &PgPool,
    profile_id: ProfileId,
    resource: ResourceId,
) -> ApiResult<Vec<ResourceFacetRow>> {
    let readable = readback::is_resource_visible(pool, profile_id, resource)
        .await
        .map_err(|e| ApiError::from(TemperError::Api(e.to_string())))?;
    if !readable {
        return Err(ApiError::NotFound(FACET_REFUSAL.to_string()));
    }

    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            serde_json::Value,
            f64,
            chrono::DateTime<chrono::Utc>,
            Uuid,
            Option<Uuid>,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT p.id, p.property_key, p.property_value, p.weight, p.created,
                p.asserted_by_event_id, pr.id, pr.handle, pr.display_name
           FROM kb_properties p
           JOIN kb_events ev ON ev.id = p.asserted_by_event_id
           LEFT JOIN kb_entities en ON en.id = ev.emitter_entity_id
           LEFT JOIN kb_profiles pr ON pr.id = en.profile_id
          WHERE p.owner_table = 'kb_resources' AND p.owner_id = $1
            AND p.property_key = 'facet' AND NOT p.is_folded
          ORDER BY p.property_key, p.created, p.id",
    )
    .bind(Uuid::from(resource))
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                property_id,
                property_key,
                value,
                weight,
                created,
                authored_by_event_id,
                authored_by_profile_id,
                authored_by_handle,
                authored_by_display_name,
            )| ResourceFacetRow {
                property_id,
                property_key,
                value,
                weight,
                created,
                authored_by_event_id,
                authored_by_profile_id,
                authored_by_handle,
                authored_by_display_name,
            },
        )
        .collect())
}
