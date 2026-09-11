//! Edge service — the one live read over the substrate graph.
//!
//! Frontmatter→edge derivation (extract/reconcile/project) was retired with the
//! flip (product decision 1); edge writes route through the backend's
//! relationship commands (`02_functions.sql`). What remains here is the single
//! read the `/api/resources/{id}/edges` handler needs.

use sqlx::PgPool;
use uuid::Uuid;

use crate::backend::substrate_read::block_read_select;
use crate::error::{ApiError, ApiResult};
use crate::services::blob_service::parse_wire_relation_direction;
use temper_core::types::facet_requests::{
    AnchorAddress, AnchorAddressResolution, AnchorVerdict, AnchoredAtEndpoint, EdgeFacetRow,
    ANCHORED_AT_PROPERTY_KEY,
};
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::ids::{EdgeId, ProfileId};
use temper_core::types::provenance::{BlockProvenanceRow, BlockRead};
use temper_workflow::types::graph::GraphEdgeRow;

/// List the edges incident to a resource, scoped to profile visibility.
///
/// Reads the substrate `kb_edges` + `edges_visible_to`. Returns
/// [`GraphEdgeRow`] for the `/edges` handler. §9-non-invariant shaping:
/// - the peer is polymorphic: a resource peer carries its title + derived slug
///   (§7-dissolved in the substrate, so derived here — matching Rust
///   `text::slugify` / the substrate `graph_nodes`); a blob peer is addressed
///   by bare id, title and slug both null (a blob has no slug, so no
///   decorated form).
/// - `direction` keeps the legacy `'outgoing'`/`'incoming'` vocabulary, derived
///   from which endpoint is the queried resource — typed on the wire
///   (`BlobRelationEdgeDirection`) and parsed through the scrubbed choke point.
///
/// Compile-time-checked (`query!`), and the row is constructed explicitly rather
/// than decoded by `FromRow`: `GraphEdgeRow` carries the `EdgeId` newtype, which
/// the macros do not decode into, so the id comes back as `Uuid` and is converted
/// in the mapping closure (the repo's incumbent shape).
pub async fn list_resource_edges(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
) -> ApiResult<Vec<GraphEdgeRow>> {
    // 404 parity: an invisible/absent resource is NotFound (the gate runs before
    // listing, so a visible resource with no edges still returns Ok(empty)).
    // `visible!`: `EXISTS` yields TRUE or FALSE and never NULL, so the non-null override is safe
    // even though sqlx types every expression column as nullable.
    let visible: bool = sqlx::query_scalar!(
        r#"SELECT EXISTS (
            SELECT 1 FROM resources_visible_to($1) rv
             WHERE rv.resource_id = $2
        ) AS "visible!""#,
        profile_id,
        resource_id,
    )
    .fetch_one(pool)
    .await?;

    if !visible {
        return Err(ApiError::NotFound(
            "resource not found or not readable".to_string(),
        ));
    }

    // Nullability overrides, each earned: `peer_table`/`peer_id`/`direction` are total `CASE`s
    // (an ELSE arm, both arms non-null — `kb_edges.source_id`/`target_id` are NOT NULL);
    // `peer_title`/`peer_slug` arrive through the LEFT JOIN and are NULL exactly when the peer
    // is a blob (a blob has no title; the slug derives from the title); `label` is a COALESCE
    // onto a non-null literal. sqlx types every expression column nullable, which is why the
    // total CASEs take the `!` override while the LEFT-JOIN columns keep `?`.
    //
    // `edge_kind`/`polarity` take an explicit type override so the SQL enums decode straight into
    // `EdgeKind`/`Polarity` (both derive `sqlx::Type` with their `type_name`) — no `::text`
    // round-trip, matching what the runtime version decoded.
    //
    // LANDED (2026-09-04, the S6 follow-up): the view renders the blob-ended edges the gate
    // already admitted — the `derivation_source` edge `--preserve-source` asserts now answers
    // "what is this resource derived from" from the resource side. The blob peer rides as
    // `peer_table`/`peer_id` alone; no blob metadata beyond the id is disclosed here. Scope is
    // still declared: resource and blob peers only — cogmap-ended edges are not rendered by
    // this view — and the walk surfaces stay node-typed (D3's exclusion unchanged): this
    // widened the edge LISTING, never a node universe.
    //
    // The negative face rides the same gate, never a restatement: `edges_visible_to` carries
    // the `readable_blobs` set mirroring `blob_readable_by_profile` branch-for-branch
    // (20260903000030), so an edge whose blob endpoint the caller cannot read is not admitted
    // at all — a reader who cannot see a blob learns nothing of it from this listing.
    let edges = sqlx::query!(
        r#"SELECT
            e.id AS edge_id,
            (CASE WHEN e.source_id = $2 THEN e.target_table ELSE e.source_table END)
                AS "peer_table!",
            (CASE WHEN e.source_id = $2 THEN e.target_id ELSE e.source_id END) AS "peer_id!",
            peer.title AS "peer_title?",
            lower(regexp_replace(
                regexp_replace(peer.title, '[^a-zA-Z0-9]+', '-', 'g'),
                '(^-+|-+$)', '', 'g')) AS "peer_slug?",
            e.edge_kind AS "edge_kind: EdgeKind",
            e.polarity AS "polarity: Polarity",
            COALESCE(e.label, '') AS "label!",
            (CASE WHEN e.source_id = $2 THEN 'outgoing' ELSE 'incoming' END) AS "direction!",
            e.weight AS weight,
            e.created AS created
          FROM kb_edges e
          JOIN edges_visible_to($1) v ON v.edge_id = e.id
          LEFT JOIN kb_resources peer
            ON (CASE WHEN e.source_id = $2 THEN e.target_table ELSE e.source_table END)
                = 'kb_resources'
           AND peer.id = (CASE WHEN e.source_id = $2 THEN e.target_id ELSE e.source_id END)
         WHERE (e.source_id = $2 OR e.target_id = $2)
           AND (CASE WHEN e.source_id = $2 THEN e.target_table ELSE e.source_table END)
                 IN ('kb_resources', 'kb_blobs')"#,
        profile_id,
        resource_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| {
        Ok(GraphEdgeRow {
            edge_id: EdgeId::from(r.edge_id),
            peer_table: r.peer_table,
            peer_id: r.peer_id,
            peer_title: r.peer_title,
            peer_slug: r.peer_slug,
            edge_kind: r.edge_kind,
            polarity: r.polarity,
            label: r.label,
            direction: parse_wire_relation_direction(&r.direction)?,
            weight: r.weight,
            created: r.created,
        })
    })
    .collect::<ApiResult<Vec<_>>>()?;

    Ok(edges)
}

/// List the live properties owned by one edge, scoped to profile visibility.
///
/// The read gate is `edges_visible_to` — the same predicate `list_resource_edges` applies and the
/// same one the edge-authorship clauses answer to. Reading an edge's facets is not a wider
/// disclosure than reading the edge: the facet qualifies a link the caller can already see.
/// An edge that is absent *or* invisible is `NotFound`, so the endpoint never becomes an existence
/// oracle for edges in contexts the caller cannot read.
///
/// Folded rows are excluded. Since folding an edge cascades to the properties it owns
/// (`_project_relationship_folded`), a live row here always belongs to a live edge.
///
/// **A folded edge is `NotFound`, not an empty list** — `edges_visible_to` is `WHERE NOT
/// e.is_folded`, so a retracted relationship is invisible and so are its facets. That falls out of
/// the incumbent predicate rather than being a rule for facets, which is why it is not special-cased
/// here.
///
/// Both queries here are compile-time-checked. The facet SELECT became a `query_as!` on 2026-07-29;
/// the visibility gate followed once the exemption it had been resting on was actually tested and
/// found false. That exemption claimed sqlx's compile-time describe could not resolve
/// `edges_visible_to`/`resources_visible_to` because their bodies reference helpers unqualified —
/// but the describe step never inlines a function body, and `team_service::is_visible` had been
/// calling `resources_visible_to` from a `query_scalar!` in this same crate the whole time.
///
/// Recorded rather than quietly corrected because the shape is worth recognising: an exemption
/// stated once at function scope silently covers every query added to that function afterwards, and
/// an unverified one spreads by citation — `lineage_service` adopted this exact claim by reference.
///
/// **Each live `anchored-at` row additionally resolves its address** through the block read's
/// three-state contract — one gated block read per anchored-at row (`block_read_select`, the
/// landed block-addressed service, whose envelope already carries the block's own attribution;
/// this read performed no block read before 2026-09) — and, where the edge declares a direction
/// and the row anchors the declared side, states the verdict against that attribution. The walk
/// never computes any of this: traversal answers *which edges are qualified*, this read answers
/// *what the qualification states and whether it agrees*.
///
/// **Disclosure rests on the gate invariant**: `edges_visible_to` requires the edge's home AND
/// both endpoints readable, and any well-formed anchor's addressed resource IS one of those
/// endpoints — so the block read's refused face (its `Absent` arm, or its not-found error for a
/// home the caller cannot read) collapses no-such-block and unreadable-home into one arm that
/// discloses nothing the address value did not already carry.
pub async fn list_edge_facets(
    pool: &PgPool,
    profile_id: Uuid,
    edge_id: Uuid,
) -> ApiResult<Vec<EdgeFacetRow>> {
    // `visible!`: `EXISTS` is never NULL (see the matching gate in `list_resource_edges`).
    let visible: bool = sqlx::query_scalar!(
        r#"SELECT EXISTS (
            SELECT 1 FROM edges_visible_to($1) v WHERE v.edge_id = $2
        ) AS "visible!""#,
        profile_id,
        edge_id,
    )
    .fetch_one(pool)
    .await?;

    if !visible {
        return Err(ApiError::NotFound(
            "edge not found or not readable".to_string(),
        ));
    }

    // Attribution is joined here rather than left to the caller: the edge's own trail cannot
    // recover it. `element_trail_edge` joins events on `payload->>'edge_id'`, and a
    // `property_asserted` payload carries `owner.{table,id}` instead — so the one surface built to
    // answer "what happened to this edge" is structurally blind to its facets. Until that is
    // reconciled, this read is the only place an author is recoverable, which is why it is not
    // optional here.
    //
    // The row is constructed explicitly rather than decoded by `FromRow` (the incumbent shape
    // `list_resource_edges` uses): two of the wire fields — `address_resolution` and `verdict` —
    // are computed after the query, not selected, so the aliases can no longer BE the mapping.
    // The three author columns take `?` because they arrive through a LEFT JOIN, and sqlx infers
    // nullability from the column definition — `kb_profiles.handle` is NOT NULL, so without the
    // annotation the macro would type an absent author as a non-optional String.
    //
    // ORDER BY is unchanged from the runtime version. This is a form change, not a behaviour change.
    let mut rows = sqlx::query!(
        r#"
        SELECT p.id                    AS property_id,
               p.property_key          AS property_key,
               p.property_value        AS value,
               p.weight                AS weight,
               p.asserted_by_event_id  AS authored_by_event_id,
               pr.id                   AS "authored_by_profile_id?",
               pr.handle               AS "authored_by_handle?",
               pr.display_name         AS "authored_by_display_name?"
          FROM kb_properties p
          JOIN kb_events ev ON ev.id = p.asserted_by_event_id
          LEFT JOIN kb_entities en ON en.id = ev.emitter_entity_id
          LEFT JOIN kb_profiles pr ON pr.id = en.profile_id
         WHERE p.owner_table = 'kb_edges' AND p.owner_id = $1 AND NOT p.is_folded
         ORDER BY p.property_key, p.created
        "#,
        edge_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| EdgeFacetRow {
        property_id: r.property_id,
        property_key: r.property_key,
        value: r.value,
        weight: r.weight,
        authored_by_event_id: r.authored_by_event_id,
        authored_by_profile_id: r.authored_by_profile_id,
        authored_by_handle: r.authored_by_handle,
        authored_by_display_name: r.authored_by_display_name,
        address_resolution: None,
        verdict: None,
    })
    .collect::<Vec<_>>();

    // The anchored-at pass: resolve each row's address and, where a direction is declared for
    // the row's side, compute the verdict from the block read's own `Live` provenance. The edge
    // row is fetched only when an anchored-at row is present — the common no-anchor read pays
    // for none of this.
    if rows
        .iter()
        .any(|r| r.property_key == ANCHORED_AT_PROPERTY_KEY)
    {
        let edge = sqlx::query!(
            r#"SELECT label, source_id, target_id FROM kb_edges WHERE id = $1"#,
            edge_id,
        )
        .fetch_one(pool)
        .await?;
        // `label` is nullable; an unlabeled edge declares no direction.
        let label = edge.label.as_deref().unwrap_or("");
        for row in &mut rows {
            if row.property_key != ANCHORED_AT_PROPERTY_KEY {
                continue;
            }
            // A structurally skewed stored value degrades to resolution-only rather than
            // failing the whole read — this is not a validation pass (validation lives at the
            // write; every row the keyed action landed parses). The row still renders, with
            // its value verbatim.
            let Some((endpoint, address)) = parse_anchor(&row.value) else {
                continue;
            };
            let resolution = match block_read_select(
                pool,
                ProfileId::from(profile_id),
                address.resource,
                address.block,
            )
            .await
            {
                Ok(BlockRead::Live { provenance, .. }) => {
                    row.verdict = declared_peer(label, endpoint, edge.target_id)
                        .map(|peer| anchor_verdict(&provenance, peer));
                    AnchorAddressResolution::Live
                }
                Ok(BlockRead::Folded {
                    folded_by_event_id,
                    disposition,
                    ..
                }) => AnchorAddressResolution::Folded {
                    folded_by_event_id,
                    disposition,
                },
                // The collapsed arm: no block under the addressed resource, or the addressed
                // resource not readable — the block read's own refused face, stated
                // identically either way. Under the gate invariant above the unreadable-home
                // half is unreachable today (the address names one of the edge's own
                // endpoints); the arm is still correct and stays.
                Ok(BlockRead::Absent { .. }) | Err(ApiError::NotFound(_)) => {
                    AnchorAddressResolution::Absent
                }
                // A fault in the block read is a fault of this read — it never masquerades
                // as an anchor state.
                Err(e) => return Err(e),
            };
            row.address_resolution = Some(resolution);
        }
    }

    Ok(rows)
}

/// The anchored-at value's two halves, parsed. `None` for any value that does not carry the
/// declared shape — the degrade-to-resolution-only arm, never a re-validation of stored data.
fn parse_anchor(value: &serde_json::Value) -> Option<(AnchoredAtEndpoint, AnchorAddress)> {
    let endpoint = AnchoredAtEndpoint::parse(value.get("endpoint")?.as_str()?)?;
    let address = AnchorAddress::parse(value.get("address")?.as_str()?)?;
    Some((endpoint, address))
}

/// The verdict direction, as declared per label: `derived_from` — under BOTH kind-shapes that
/// carry it — declares peer = target, checked on source-side anchors only (the lineage
/// convention is uniform across the shapes). Everything else renders resolution-only, never a
/// computed negative: an unlabeled edge declares nothing; the `express` direction is
/// undeclared for non-`derived_from` labels; and a row anchored off the declared side
/// (cross-side) cannot be corroborated by the attribution direction.
fn declared_peer(label: &str, endpoint: AnchoredAtEndpoint, target_id: Uuid) -> Option<Uuid> {
    if label == "derived_from" && endpoint == AnchoredAtEndpoint::Source {
        Some(target_id)
    } else {
        None
    }
}

/// The directional verdict from the anchored block's own attribution — the rows the block
/// read's `Live` arm already returned, which are the live, uncorrected rows (corrected rows
/// are excluded upstream by the same `NOT is_corrected` predicate every incumbent reader
/// selects through). Carried rows corroborate: `is_carried` marks partial coverage, not
/// false testimony.
fn anchor_verdict(provenance: &[BlockProvenanceRow], peer_id: Uuid) -> AnchorVerdict {
    if provenance.is_empty() {
        AnchorVerdict::Unattributed
    } else if provenance
        .iter()
        .any(|r| r.source_kind == "resource" && r.source_id == peer_id)
    {
        AnchorVerdict::Corroborated
    } else {
        AnchorVerdict::Divergent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The direction declaration, pinned arm by arm: `derived_from` fires source-side only,
    /// under any kind-shape carrying the label; every other combination is resolution-only.
    #[test]
    fn declared_peer_fires_only_for_derived_from_source_side() {
        let target = Uuid::nil();
        for label in ["derived_from", "derivation_source", ""] {
            let source_side = declared_peer(label, AnchoredAtEndpoint::Source, target);
            let target_side = declared_peer(label, AnchoredAtEndpoint::Target, target);
            if label == "derived_from" {
                assert_eq!(source_side, Some(target), "the declared direction fires");
                assert!(
                    target_side.is_none(),
                    "the cross-side row is resolution-only, never a computed negative"
                );
            } else {
                assert!(
                    source_side.is_none() && target_side.is_none(),
                    "label {label:?} declares no direction"
                );
            }
        }
    }

    /// The verdict's three arms over the block's live attribution rows, peer-naming
    /// decisive: carried rows corroborate like direct ones, and empty is an absence, never
    /// agreement.
    #[test]
    fn anchor_verdict_names_the_peer_among_live_rows() {
        let peer = Uuid::nil();
        let row = |source_id: Uuid, is_carried: bool| BlockProvenanceRow {
            block_id: Uuid::nil(),
            block_seq: 0,
            source_kind: "resource".to_string(),
            source_id,
            source_uri: None,
            accretion_seq: 0,
            contributed_by_event_id: Uuid::nil(),
            created: chrono::Utc::now(),
            is_carried,
        };

        assert_eq!(
            anchor_verdict(&[], peer),
            AnchorVerdict::Unattributed,
            "no testimony is not agreement"
        );
        assert_eq!(
            anchor_verdict(&[row(peer, false)], peer),
            AnchorVerdict::Corroborated
        );
        assert_eq!(
            anchor_verdict(&[row(peer, true)], peer),
            AnchorVerdict::Corroborated,
            "a carried row corroborates"
        );
        let other = Uuid::now_v7();
        assert_eq!(
            anchor_verdict(&[row(other, false)], peer),
            AnchorVerdict::Divergent
        );
        assert_eq!(
            anchor_verdict(&[row(other, true), row(peer, true)], peer),
            AnchorVerdict::Corroborated,
            "the peer named among several rows decides"
        );
    }

    /// The value parser takes exactly the declared shape; every skew degrades to
    /// resolution-only instead of erroring the read.
    #[test]
    fn parse_anchor_takes_only_the_declared_shape() {
        let resource = Uuid::now_v7();
        let block = Uuid::now_v7();
        let good = serde_json::json!({
            "endpoint": "source",
            "address": format!("{resource}#{block}"),
        });
        let (endpoint, address) = parse_anchor(&good).expect("the declared shape parses");
        assert_eq!(endpoint, AnchoredAtEndpoint::Source);
        assert_eq!(address.resource, resource);
        assert_eq!(address.block, block);

        for bad in [
            serde_json::json!("not an object"),
            serde_json::json!({"endpoint": "source"}),
            serde_json::json!({"address": format!("{resource}#{block}")}),
            serde_json::json!({"endpoint": "middle", "address": format!("{resource}#{block}")}),
            serde_json::json!({"endpoint": "source", "address": "not-an-address"}),
        ] {
            assert!(
                parse_anchor(&bad).is_none(),
                "a skewed value must degrade, not parse: {bad}"
            );
        }
    }

    /// The two anchored-at fields ride the row as `null` when unset — never absent — the
    /// every-other-row rendering the read states for plain facets.
    #[test]
    fn a_row_without_resolution_renders_both_fields_null() {
        let row = EdgeFacetRow {
            property_id: Uuid::nil(),
            property_key: "facet".to_string(),
            value: serde_json::json!({"k": "v"}),
            weight: 1.0,
            authored_by_event_id: Uuid::nil(),
            authored_by_profile_id: None,
            authored_by_handle: None,
            authored_by_display_name: None,
            address_resolution: None,
            verdict: None,
        };
        let v = serde_json::to_value(&row).unwrap();
        assert_eq!(v["address_resolution"], serde_json::Value::Null);
        assert_eq!(v["verdict"], serde_json::Value::Null);
    }
}
