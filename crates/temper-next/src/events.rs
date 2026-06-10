//! The Rust fire-event action — the single "speak-as" firing surface for temper-next seeding, scenario
//! loading, and tests.
//!
//! The decided design (deliverable 2) is a **hybrid**: the SQL functions stay the atomic
//! event+materialize+commit mechanism, and this Rust action lets Rust *speak-as* the firing — mirroring
//! production's `append_and_project` while keeping temper-next's single-SQL-call atomicity. Every
//! mutation goes through [`fire`] instead of an inline `sqlx::query!("SELECT some_fn(...)")`, so there is
//! exactly one firing surface. [`fire`] dispatches each [`SeedAction`] to its SQL function (which emits
//! the event + projects in one txn) and returns the produced ids as a typed [`Fired`] record-set —
//! sparing callers a re-fetch (convergence guidance).
//!
//! The event taxonomy is a temper-next-local [`EventKind`] surfaced by [`SeedAction::event_type`]. It is
//! written **for parity** with `temper_events::EventType`'s seeding variants (same names, same
//! `as_canonical_name` values) so unifying the two at deliverable 6 is a mechanical merge — but
//! temper-next deliberately does **not** depend on temper-events: its `kb_events` shape is incommensurate
//! with the artifact's (`emitter_entity_id`/`producing_anchor_*` vs `profile_id`/`topic_id`/`scope_id`),
//! and temper-events' live sqlx macros can't co-compile under the `temper_next` search_path during
//! `prepare-next`. temper-next keeps its own SQL-function write path; this enum is the typed source for
//! the canonical names only.

use crate::affinity::EdgeKind;
use crate::content::PreparedBlock;
use crate::ids::{CogmapId, EdgeId, EntityId, EventId, LensId, ProfileId, PropertyId, ResourceId};
use crate::payloads;
use crate::scenario::model::LensDef;
use anyhow::{Context, Result};
use uuid::Uuid;

/// The seeding event taxonomy (mirrors the `kb_event_types` seeding names registered in
/// `schema-artifact/seeds/system.yaml`). Parity-shaped with `temper_events::EventType`'s seeding variants
/// so deliverable-6 unification is a rename-free merge. `RelationshipAsserted` is the only one that
/// overlaps production's existing taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    CogmapSeeded,
    ResourceCreated,
    RelationshipAsserted,
    PropertyAsserted,
    LensCreated,
    RegionMaterialized,
}

impl EventKind {
    /// The canonical `kb_event_types.name` for this kind.
    pub fn as_canonical_name(self) -> &'static str {
        match self {
            EventKind::CogmapSeeded => "cogmap_seeded",
            EventKind::ResourceCreated => "resource_created",
            EventKind::RelationshipAsserted => "relationship_asserted",
            EventKind::PropertyAsserted => "property_asserted",
            EventKind::LensCreated => "lens_created",
            EventKind::RegionMaterialized => "region_materialized",
        }
    }
}

/// One seeding mutation, carrying its params (typed ids — bare `Uuid` only at the SQL-bind boundary).
/// One variant per reusable SQL mutation function, plus `Materialize` (whose event has no SQL function —
/// it is the raw `region_materialized` INSERT, reconciled here so it shares the one firing surface).
pub enum SeedAction<'a> {
    CogmapGenesis {
        name: &'a str,
        telos_title: &'a str,
        /// The Rust-prepared charter blocks (block-0 statement, questions-with-context, framing).
        charter: &'a [PreparedBlock],
        owner: ProfileId,
        emitter: EntityId,
    },
    ResourceCreate {
        title: &'a str,
        origin_uri: &'a str,
        home: CogmapId,
        owner: ProfileId,
        blocks: &'a [PreparedBlock],
        doc_type: Option<&'a str>,
        emitter: EntityId,
    },
    RelationshipAssert {
        src: ResourceId,
        tgt: ResourceId,
        kind: EdgeKind,
        label: Option<&'a str>,
        weight: f64,
        home: CogmapId,
        emitter: EntityId,
    },
    FacetSet {
        resource: ResourceId,
        values: &'a serde_json::Value,
        weight: f64,
        emitter: EntityId,
    },
    LensCreate {
        /// `None` ⇒ a global system lens (`cogmap_id NULL`).
        cogmap: Option<CogmapId>,
        lens: &'a LensDef,
        emitter: EntityId,
    },
    Materialize {
        cogmap: CogmapId,
        emitter: EntityId,
    },
}

impl SeedAction<'_> {
    /// The taxonomy tag this action fires (the typed source for the canonical event name).
    pub fn event_type(&self) -> EventKind {
        match self {
            SeedAction::CogmapGenesis { .. } => EventKind::CogmapSeeded,
            SeedAction::ResourceCreate { .. } => EventKind::ResourceCreated,
            SeedAction::RelationshipAssert { .. } => EventKind::RelationshipAsserted,
            SeedAction::FacetSet { .. } => EventKind::PropertyAsserted,
            SeedAction::LensCreate { .. } => EventKind::LensCreated,
            SeedAction::Materialize { .. } => EventKind::RegionMaterialized,
        }
    }
}

/// The ids a fired action produced (record-set return). Variant matches the [`SeedAction`] fired; the
/// caller statically knows which it fired, so the accessors below extract the expected payload.
#[derive(Debug, Clone)]
pub enum Fired {
    CogmapGenesis {
        cogmap: CogmapId,
        telos_resource: ResourceId,
    },
    Resource(ResourceId),
    Relationship(EdgeId),
    Facet(PropertyId),
    Lens(LensId),
    Materialize(EventId),
}

impl Fired {
    /// Extract the cogmap + telos-resource ids a `CogmapGenesis` fire produced.
    pub fn cogmap_genesis(self) -> Result<(CogmapId, ResourceId)> {
        match self {
            Fired::CogmapGenesis {
                cogmap,
                telos_resource,
            } => Ok((cogmap, telos_resource)),
            other => anyhow::bail!("expected Fired::CogmapGenesis, got {other:?}"),
        }
    }

    /// Extract the resource id a `ResourceCreate` fire produced.
    pub fn resource(self) -> Result<ResourceId> {
        match self {
            Fired::Resource(id) => Ok(id),
            other => anyhow::bail!("expected Fired::Resource, got {other:?}"),
        }
    }

    /// Extract the event id a `Materialize` fire produced.
    pub fn materialize_event(self) -> Result<EventId> {
        match self {
            Fired::Materialize(id) => Ok(id),
            other => anyhow::bail!("expected Fired::Materialize, got {other:?}"),
        }
    }
}

/// Fire one seeding action: dispatch it to its SQL function (event + projection, one txn) and return the
/// produced ids. The caller threads a transaction (`&mut *tx`) so a run of fires commits atomically.
pub async fn fire(conn: &mut sqlx::PgConnection, action: SeedAction<'_>) -> Result<Fired> {
    match action {
        SeedAction::CogmapGenesis {
            name,
            telos_title,
            charter,
            owner,
            emitter,
        } => {
            let payload = payloads::CogmapSeeded {
                cogmap_id: CogmapId::from(Uuid::now_v7()),
                name: name.to_owned(),
                owner_profile_id: owner,
                telos: payloads::TelosManifest {
                    resource_id: ResourceId::from(Uuid::now_v7()),
                    title: telos_title.to_owned(),
                    origin_uri: "temper://genesis".into(),
                    blocks: charter.iter().map(payloads::BlockManifest::from).collect(),
                },
            };
            let sidecar = serde_json::to_value(payloads::content_sidecar(charter))?;
            let row = sqlx::query!(
                "SELECT cogmap_id, telos_resource_id FROM cogmap_genesis($1,$2,$3)",
                serde_json::to_value(&payload)?,
                sidecar,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?;
            Ok(Fired::CogmapGenesis {
                cogmap: row
                    .cogmap_id
                    .context("cogmap_genesis returned null cogmap_id")?
                    .into(),
                telos_resource: row
                    .telos_resource_id
                    .context("cogmap_genesis returned null telos_resource_id")?
                    .into(),
            })
        }

        SeedAction::ResourceCreate {
            title,
            origin_uri,
            home,
            owner,
            blocks,
            doc_type,
            emitter,
        } => {
            let payload = payloads::ResourceCreated {
                resource_id: ResourceId::from(Uuid::now_v7()),
                title: title.to_owned(),
                origin_uri: origin_uri.to_owned(),
                home: payloads::AnchorRef::cogmap(home),
                owner_profile_id: owner,
                doc_type: doc_type.map(str::to_owned),
                blocks: blocks.iter().map(payloads::BlockManifest::from).collect(),
            };
            let sidecar = serde_json::to_value(payloads::content_sidecar(blocks))?;
            let id = sqlx::query_scalar!(
                "SELECT resource_create($1,$2,$3)",
                serde_json::to_value(&payload)?,
                sidecar,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?
            .context("resource_create returned null")?;
            Ok(Fired::Resource(id.into()))
        }

        SeedAction::RelationshipAssert {
            src,
            tgt,
            kind,
            label,
            weight,
            home,
            emitter,
        } => {
            let payload = payloads::RelationshipAsserted {
                edge_id: EdgeId::from(Uuid::now_v7()),
                source: payloads::AnchorRef::resource(src),
                target: payloads::AnchorRef::resource(tgt),
                edge_kind: kind,
                polarity: payloads::EdgePolarity::Forward,
                label: label.map(str::to_owned),
                weight,
                home: payloads::AnchorRef::cogmap(home),
            };
            let id = sqlx::query_scalar!(
                "SELECT relationship_assert($1,$2)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?
            .context("relationship_assert returned null")?;
            Ok(Fired::Relationship(EdgeId::from(id)))
        }

        SeedAction::FacetSet {
            resource,
            values,
            weight,
            emitter,
        } => {
            let payload = payloads::PropertyAsserted {
                property_id: PropertyId::from(Uuid::now_v7()),
                owner: payloads::AnchorRef::resource(resource),
                property_key: "facet".into(),
                value: values.clone(),
                weight,
            };
            let id = sqlx::query_scalar!(
                "SELECT facet_set($1,$2)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?
            .context("facet_set returned null")?;
            Ok(Fired::Facet(PropertyId::from(id)))
        }

        SeedAction::LensCreate {
            cogmap,
            lens,
            emitter,
        } => {
            let payload = payloads::LensCreated {
                lens_id: LensId::from(Uuid::now_v7()),
                cogmap_id: cogmap,
                name: lens.name.clone(),
                selection_kind: "homed".into(),
                weights: payloads::LensWeights {
                    express: lens.w_express,
                    contains: lens.w_contains,
                    leads_to: lens.w_leads_to,
                    near: lens.w_near,
                    prop: lens.w_prop,
                },
                salience: payloads::SalienceWeights {
                    telos: lens.s_telos,
                    reference: lens.s_ref,
                    central: lens.s_central,
                },
                resolution: lens.resolution,
            };
            let id = sqlx::query_scalar!(
                "SELECT lens_create($1,$2)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?
            .context("lens_create returned null")?;
            Ok(Fired::Lens(id.into()))
        }

        SeedAction::Materialize { cogmap, emitter } => {
            // No SQL function: the materialization event is a raw INSERT (reconciled out of write.rs).
            // The emitter is the actor on whose behalf materialization runs — passed explicitly, never
            // derived from "latest event" (NULL on an empty log, arbitrary on occurred_at ties).
            let id = sqlx::query_scalar!(
                "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id) \
                 SELECT (SELECT id FROM kb_event_types WHERE name=$2), $3, 'kb_cogmaps', $1 RETURNING id",
                cogmap.uuid(),
                EventKind::RegionMaterialized.as_canonical_name(),
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?;
            Ok(Fired::Materialize(id.into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_maps_each_action_to_its_canonical_name() {
        // The fire surface is the typed source for the seeding event names.
        let charter: Vec<PreparedBlock> = Vec::new();
        let owner = ProfileId::from(Uuid::nil());
        let emitter = EntityId::from(Uuid::nil());
        assert_eq!(
            SeedAction::CogmapGenesis {
                name: "n",
                telos_title: "t",
                charter: &charter,
                owner,
                emitter,
            }
            .event_type()
            .as_canonical_name(),
            "cogmap_seeded"
        );
        assert_eq!(
            SeedAction::Materialize {
                cogmap: CogmapId::from(Uuid::nil()),
                emitter,
            }
            .event_type()
            .as_canonical_name(),
            "region_materialized"
        );
    }
}
