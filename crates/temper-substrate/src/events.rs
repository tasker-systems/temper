//! The Rust fire-event action — the single "speak-as" firing surface for temper-substrate seeding, scenario
//! loading, and tests.
//!
//! The decided design (deliverable 2) is a **hybrid**: the SQL functions stay the atomic
//! event+materialize+commit mechanism, and this Rust action lets Rust *speak-as* the firing — mirroring
//! production's `append_and_project` while keeping temper-substrate's single-SQL-call atomicity. Every
//! mutation goes through [`fire`] instead of an inline `sqlx::query!("SELECT some_fn(...)")`, so there is
//! exactly one firing surface. [`fire`] dispatches each [`SeedAction`] to its SQL function (which emits
//! the event + projects in one txn) and returns the produced ids as a typed [`Fired`] record-set —
//! sparing callers a re-fetch (convergence guidance).
//!
//! The event taxonomy is a temper-substrate-local [`EventKind`] surfaced by [`SeedAction::event_type`]. It is
//! written **for parity** with `temper_events::EventType`'s seeding variants (same names, same
//! `as_canonical_name` values) so unifying the two at deliverable 6 is a mechanical merge — but
//! temper-substrate deliberately does **not** depend on temper-events: its `kb_events` shape is incommensurate
//! with the artifact's (`emitter_entity_id`/`producing_anchor_*` vs `profile_id`/`topic_id`/`scope_id`),
//! and temper-events' live sqlx macros are incompatible with the substrate's sqlx cache preparation.
//! temper-substrate keeps its own SQL-function write path; this enum is the typed source for the
//! canonical names only.

use crate::affinity::EdgeKind;
use crate::content::{PreparedBlock, PreparedChunk};
use crate::ids::{
    BlockId, CogmapId, ContextId, EdgeId, EntityId, EventId, InvocationId, LensId, ProfileId,
    PropertyId, RegionId, ResourceId,
};
use crate::payloads;
use crate::scenario::model::LensDef;
use anyhow::{Context, Result};
use uuid::Uuid;

/// The seeding event taxonomy (mirrors the `kb_event_types` seeding names registered in
/// `tests/fixtures/seeds/system.yaml`). Parity-shaped with `temper_events::EventType`'s seeding variants
/// so deliverable-6 unification is a rename-free merge. `RelationshipAsserted` is the only one that
/// overlaps production's existing taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    CogmapSeeded,
    ResourceCreated,
    ResourceUpdated,
    ResourceDeleted,
    ResourceRehomed,
    RelationshipAsserted,
    RelationshipRetyped,
    RelationshipReweighted,
    PropertyAsserted,
    PropertySet,
    LensCreated,
    RegionMaterialized,
    RelationshipFolded,
    BlockMutated,
    CharterSet,
    DelegatedLaunch,
    InvocationClosed,
}

impl EventKind {
    /// The canonical `kb_event_types.name` for this kind.
    pub fn as_canonical_name(self) -> &'static str {
        match self {
            EventKind::CogmapSeeded => "cogmap_seeded",
            EventKind::ResourceCreated => "resource_created",
            EventKind::ResourceUpdated => "resource_updated",
            EventKind::ResourceDeleted => "resource_deleted",
            EventKind::ResourceRehomed => "resource_rehomed",
            EventKind::RelationshipAsserted => "relationship_asserted",
            EventKind::RelationshipRetyped => "relationship_retyped",
            EventKind::RelationshipReweighted => "relationship_reweighted",
            EventKind::PropertyAsserted => "property_asserted",
            EventKind::PropertySet => "property_set",
            EventKind::LensCreated => "lens_created",
            EventKind::RegionMaterialized => "region_materialized",
            EventKind::RelationshipFolded => "relationship_folded",
            EventKind::BlockMutated => "block_mutated",
            EventKind::CharterSet => "charter_set",
            EventKind::DelegatedLaunch => "delegated_launch",
            EventKind::InvocationClosed => "invocation_closed",
        }
    }

    /// Parse a canonical `kb_event_types.name` back into an `EventKind`.
    ///
    /// The exact inverse of [`Self::as_canonical_name`]; returns `None` for
    /// names outside the owned set. Lets Rust-side dispatch (e.g. ledger
    /// replay) match the typed enum instead of branching on raw strings, so a
    /// new variant is a compile error rather than a runtime `bail!`.
    pub fn from_canonical_name(name: &str) -> Option<Self> {
        Some(match name {
            "cogmap_seeded" => EventKind::CogmapSeeded,
            "resource_created" => EventKind::ResourceCreated,
            "resource_updated" => EventKind::ResourceUpdated,
            "resource_deleted" => EventKind::ResourceDeleted,
            "resource_rehomed" => EventKind::ResourceRehomed,
            "relationship_asserted" => EventKind::RelationshipAsserted,
            "relationship_retyped" => EventKind::RelationshipRetyped,
            "relationship_reweighted" => EventKind::RelationshipReweighted,
            "property_asserted" => EventKind::PropertyAsserted,
            "property_set" => EventKind::PropertySet,
            "lens_created" => EventKind::LensCreated,
            "region_materialized" => EventKind::RegionMaterialized,
            "relationship_folded" => EventKind::RelationshipFolded,
            "block_mutated" => EventKind::BlockMutated,
            "charter_set" => EventKind::CharterSet,
            "delegated_launch" => EventKind::DelegatedLaunch,
            "invocation_closed" => EventKind::InvocationClosed,
            _ => return None,
        })
    }
}

/// Where an asserted edge homes — polymorphic per the payload's `AnchorRef`
/// (`kb_cogmaps` | `kb_contexts`); the typed fire-path mirror of edge-home
/// polymorphism (WS6 adjudication §2: context-homed edges gate by context-share).
#[derive(Debug, Clone, Copy)]
pub enum EdgeHome {
    Cogmap(CogmapId),
    Context(ContextId),
}

impl EdgeHome {
    fn anchor_ref(self) -> payloads::AnchorRef {
        match self {
            EdgeHome::Cogmap(c) => payloads::AnchorRef::cogmap(c),
            EdgeHome::Context(c) => payloads::AnchorRef::context(c),
        }
    }
}

/// One seeding mutation, carrying its params (typed ids — bare `Uuid` only at the SQL-bind boundary).
/// One variant per reusable SQL mutation function, plus `Materialize` (whose event has no SQL function —
/// it is the raw `region_materialized` INSERT, reconciled here so it shares the one firing surface).
#[derive(Debug)]
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
        /// The id to mint the resource under (PR#124 identity-as-input; the projection reads
        /// `resource_id` from the payload). `None` ⇒ a fresh `Uuid::now_v7()` is minted (the scenario
        /// path — every seeded resource is genuinely new). Synthesis passes `Some(prod_id)` so the
        /// production resource id survives the flip verbatim and externally-held `ref`s do not dangle.
        resource_id: Option<ResourceId>,
        /// The resource's home anchor — polymorphic per `AnchorRef` (`kb_cogmaps` for the scenario
        /// path; `kb_contexts` for synthesized, context-homed resources, §2).
        home: payloads::AnchorRef,
        owner: ProfileId,
        /// The home's originator (§2). `None` ⇒ the projector COALESCEs it to `owner` (scenario path,
        /// originator≡owner); synthesis sets it so a distinct production originator survives.
        originator: Option<ProfileId>,
        blocks: &'a [PreparedBlock],
        doc_type: Option<&'a str>,
        emitter: EntityId,
    },
    RelationshipAssert {
        src: ResourceId,
        tgt: ResourceId,
        kind: EdgeKind,
        /// The edge's polarity, carried verbatim (§4). The scenario paths pass
        /// `payloads::EdgePolarity::Forward`; synthesis carries the production value.
        polarity: payloads::EdgePolarity,
        label: Option<&'a str>,
        weight: f64,
        home: EdgeHome,
        emitter: EntityId,
    },
    FacetSet {
        resource: ResourceId,
        values: &'a serde_json::Value,
        weight: f64,
        emitter: EntityId,
    },
    /// Assert one keyed property on a resource (WS6 §7 synthesis: one `kb_properties` row per surviving
    /// manifest key). Unlike `FacetSet` (which hardcodes `property_key="facet"` for the scenario
    /// value-map), this carries an arbitrary `key`/`value` pair — but fires the SAME key-agnostic
    /// `facet_set` SQL function, so the owner resource's home anchors the event (errors if homeless).
    PropertyAssert {
        resource: ResourceId,
        key: &'a str,
        value: &'a serde_json::Value,
        weight: f64,
        emitter: EntityId,
    },
    /// Set a SINGLE-valued property: folds prior active `(owner, key)` rows then asserts this value, so
    /// the key holds one current value (the resource-frontmatter shape). Multi-valued facets use
    /// [`SeedAction::FacetSet`] / [`SeedAction::PropertyAssert`] (append).
    PropertySet {
        resource: ResourceId,
        key: &'a str,
        value: &'a serde_json::Value,
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
        lens: LensId,
        /// Max event id over the substrate at load time — the point-in-time the projection saw.
        watermark: EventId,
        membership_fingerprint: &'a str,
        region_ids: &'a [RegionId],
        emitter: EntityId,
    },
    RelationshipFold {
        edge: EdgeId,
        reason: Option<&'a str>,
        emitter: EntityId,
    },
    BlockMutate {
        block: BlockId,
        /// The revised body as a single prepared block's worth of chunks (re-embedded inline).
        chunks: &'a [PreparedChunk],
        emitter: EntityId,
    },
    /// Replace a cogmap's telos charter with a full role-tagged block set (post-birth populate). The
    /// genesis leaves the telos empty and `BlockMutate` is revise-only, so this is the only primitive that
    /// can deliver (0→N) or re-deliver (N→M) a charter: it folds the prior blocks then projects `blocks`.
    CharterSet {
        cogmap: CogmapId,
        /// The Rust-prepared charter blocks (statement, questions-with-context, framing), pre-embedded.
        blocks: &'a [PreparedBlock],
        emitter: EntityId,
    },
    // ── WS6 4c resource + relationship mutations (live write path) ──
    ResourceDelete {
        resource: ResourceId,
        emitter: EntityId,
    },
    ResourceUpdate {
        resource: ResourceId,
        /// Mutable `kb_resources` columns; `None` ⇒ unchanged (projector COALESCEs).
        title: Option<&'a str>,
        origin_uri: Option<&'a str>,
        emitter: EntityId,
    },
    ResourceRehome {
        resource: ResourceId,
        /// The destination anchor (a `kb_contexts` ref for a context move).
        home: payloads::AnchorRef,
        emitter: EntityId,
    },
    RelationshipRetype {
        edge: EdgeId,
        kind: EdgeKind,
        polarity: payloads::EdgePolarity,
        emitter: EntityId,
    },
    RelationshipReweight {
        edge: EdgeId,
        weight: f64,
        emitter: EntityId,
    },
    InvocationOpen {
        invocation: InvocationId,
        trigger_kind: &'a str,
        originating: CogmapId,
        parent: Option<CogmapId>,
        scoped_entity: EntityId,
        emitter: EntityId,
    },
    InvocationClose {
        invocation: InvocationId,
        disposition: payloads::Disposition,
        outcome: serde_json::Value,
        originating: CogmapId,
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
            SeedAction::PropertyAssert { .. } => EventKind::PropertyAsserted,
            SeedAction::PropertySet { .. } => EventKind::PropertySet,
            SeedAction::LensCreate { .. } => EventKind::LensCreated,
            SeedAction::Materialize { .. } => EventKind::RegionMaterialized,
            SeedAction::RelationshipFold { .. } => EventKind::RelationshipFolded,
            SeedAction::BlockMutate { .. } => EventKind::BlockMutated,
            SeedAction::CharterSet { .. } => EventKind::CharterSet,
            SeedAction::ResourceDelete { .. } => EventKind::ResourceDeleted,
            SeedAction::ResourceUpdate { .. } => EventKind::ResourceUpdated,
            SeedAction::ResourceRehome { .. } => EventKind::ResourceRehomed,
            SeedAction::RelationshipRetype { .. } => EventKind::RelationshipRetyped,
            SeedAction::RelationshipReweight { .. } => EventKind::RelationshipReweighted,
            SeedAction::InvocationOpen { .. } => EventKind::DelegatedLaunch,
            SeedAction::InvocationClose { .. } => EventKind::InvocationClosed,
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
    Block(BlockId),
    /// The telos resource id a `CharterSet` fire replaced the charter on.
    Charter(ResourceId),
    Invocation(InvocationId),
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

    /// Extract the edge id a `RelationshipAssert` fire produced (so a follow-up `RelationshipFold`
    /// can target it — the §4 assert+fold pair).
    pub fn relationship(self) -> Result<EdgeId> {
        match self {
            Fired::Relationship(id) => Ok(id),
            other => anyhow::bail!("expected Fired::Relationship, got {other:?}"),
        }
    }

    /// Extract the event id a `Materialize` fire produced.
    pub fn materialize_event(self) -> Result<EventId> {
        match self {
            Fired::Materialize(id) => Ok(id),
            other => anyhow::bail!("expected Fired::Materialize, got {other:?}"),
        }
    }

    /// Extract the block id a `BlockMutate` fire produced.
    pub fn block(self) -> Result<BlockId> {
        match self {
            Fired::Block(id) => Ok(id),
            other => anyhow::bail!("expected Fired::Block, got {other:?}"),
        }
    }

    /// Extract the telos resource id a `CharterSet` fire produced.
    pub fn charter(self) -> Result<ResourceId> {
        match self {
            Fired::Charter(id) => Ok(id),
            other => anyhow::bail!("expected Fired::Charter, got {other:?}"),
        }
    }

    /// Extract the invocation id an `InvocationOpen` fire produced.
    pub fn invocation(self) -> Result<InvocationId> {
        match self {
            Fired::Invocation(id) => Ok(id),
            other => anyhow::bail!("expected Fired::Invocation, got {other:?}"),
        }
    }
}

/// Per-fire authored-act context: the agent's authorship metadata (→ kb_events.metadata) and the
/// invocation it is acting under (→ kb_events.invocation_id). Default = a keyboard-holder/system act
/// (no authorship, no invocation), so `fire` callers are unchanged.
#[derive(Debug, Default, Clone)]
pub struct EventContext {
    pub authorship: Option<payloads::AgentAuthorship>,
    pub invocation: Option<InvocationId>,
}

impl EventContext {
    fn metadata_json(&self) -> Result<serde_json::Value> {
        Ok(match &self.authorship {
            Some(a) => serde_json::to_value(a)?,
            None => serde_json::json!({}),
        })
    }
    fn invocation_uuid(&self) -> Option<Uuid> {
        self.invocation.map(InvocationId::uuid)
    }
}

/// Fire one seeding action: dispatch it to its SQL function (event + projection, one txn) and return the
/// produced ids. The caller threads a transaction (`&mut *tx`) so a run of fires commits atomically.
pub async fn fire(conn: &mut sqlx::PgConnection, action: SeedAction<'_>) -> Result<Fired> {
    fire_with(conn, action, EventContext::default()).await
}

/// Fire one seeding action under an explicit [`EventContext`] (authorship + invocation). Every
/// correlatable mutation arm threads `ctx` into its SQL call (→ `kb_events.metadata`/`invocation_id`):
/// the authored-4 (`ResourceCreate`/`RelationshipAssert`/`FacetSet`/`RelationshipFold`) plus the
/// non-authored writes (`ResourceUpdate`/`ResourceDelete`/`ResourceRehome`/`PropertySet`/`BlockMutate`/
/// `CharterSet`/`RelationshipRetype`/`RelationshipReweight`). The pure-seed/lens/materialize arms (and
/// the legacy 2-arg `PropertyAssert`) ignore it. [`fire`] is the `EventContext::default()` delegate.
pub async fn fire_with(
    conn: &mut sqlx::PgConnection,
    action: SeedAction<'_>,
    ctx: EventContext,
) -> Result<Fired> {
    let ctx_meta = ctx.metadata_json()?;
    let ctx_inv = ctx.invocation_uuid();
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
            resource_id,
            home,
            owner,
            originator,
            blocks,
            doc_type,
            emitter,
        } => {
            let payload = payloads::ResourceCreated {
                resource_id: resource_id.unwrap_or_else(|| ResourceId::from(Uuid::now_v7())),
                title: title.to_owned(),
                origin_uri: origin_uri.to_owned(),
                home,
                owner_profile_id: owner,
                originator_profile_id: originator,
                doc_type: doc_type.map(str::to_owned),
                blocks: blocks.iter().map(payloads::BlockManifest::from).collect(),
            };
            let sidecar = serde_json::to_value(payloads::content_sidecar(blocks))?;
            let id = sqlx::query_scalar!(
                "SELECT resource_create($1,$2,$3,$4,$5)",
                serde_json::to_value(&payload)?,
                sidecar,
                emitter.uuid(),
                ctx_meta,
                ctx_inv,
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
            polarity,
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
                polarity,
                label: label.map(str::to_owned),
                weight,
                home: home.anchor_ref(),
            };
            let id = sqlx::query_scalar!(
                "SELECT relationship_assert($1,$2,$3,$4)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
                ctx_meta,
                ctx_inv,
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
                "SELECT facet_set($1,$2,$3,$4)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
                ctx_meta,
                ctx_inv,
            )
            .fetch_one(&mut *conn)
            .await?
            .context("facet_set returned null")?;
            Ok(Fired::Facet(PropertyId::from(id)))
        }

        SeedAction::PropertyAssert {
            resource,
            key,
            value,
            weight,
            emitter,
        } => {
            let payload = payloads::PropertyAsserted {
                property_id: PropertyId::from(Uuid::now_v7()),
                owner: payloads::AnchorRef::resource(resource),
                property_key: key.to_owned(),
                value: value.clone(),
                weight,
            };
            // Reuses the same key-agnostic `facet_set` query as `FacetSet` — no new SQL function.
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

        SeedAction::PropertySet {
            resource,
            key,
            value,
            weight,
            emitter,
        } => {
            let payload = payloads::PropertySet {
                property_id: PropertyId::from(Uuid::now_v7()),
                owner: payloads::AnchorRef::resource(resource),
                property_key: key.to_owned(),
                value: value.clone(),
                weight,
            };
            let id = sqlx::query_scalar!(
                "SELECT property_set($1,$2,$3,$4)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
                ctx_meta,
                ctx_inv,
            )
            .fetch_one(&mut *conn)
            .await?
            .context("property_set returned null")?;
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

        SeedAction::Materialize {
            cogmap,
            lens,
            watermark,
            membership_fingerprint,
            region_ids,
            emitter,
        } => {
            // The emitter is the actor on whose behalf materialization runs — passed explicitly, never
            // derived from "latest event" (NULL on an empty log, arbitrary on occurred_at ties). The
            // payload records the act's full identity (lens, watermark, fingerprint, region ids);
            // region ROWS stay Rust-side derived compute (write.rs).
            let payload = payloads::RegionMaterialized {
                cogmap_id: cogmap,
                lens_id: lens,
                watermark_event_id: watermark,
                membership_fingerprint: membership_fingerprint.to_owned(),
                region_ids: region_ids.to_vec(),
            };
            let id = sqlx::query_scalar!(
                "SELECT region_materialize($1,$2)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?
            .context("region_materialize returned null")?;
            Ok(Fired::Materialize(EventId::from(id)))
        }

        SeedAction::RelationshipFold {
            edge,
            reason,
            emitter,
        } => {
            let payload = payloads::RelationshipFolded {
                edge_id: edge,
                reason: reason.map(str::to_owned),
            };
            let id = sqlx::query_scalar!(
                "SELECT relationship_fold($1,$2,$3,$4)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
                ctx_meta,
                ctx_inv,
            )
            .fetch_one(&mut *conn)
            .await?
            .context("relationship_fold returned null")?;
            Ok(Fired::Relationship(EdgeId::from(id)))
        }

        SeedAction::BlockMutate {
            block,
            chunks,
            emitter,
        } => {
            let payload = payloads::BlockMutated {
                block_id: block,
                chunks: chunks.iter().map(payloads::ChunkManifest::from).collect(),
                incorporated: Vec::new(), // body-revision only; provenance accretion deferred
            };
            let mut sidecar = std::collections::HashMap::new();
            payloads::content_sidecar_chunks(&mut sidecar, chunks);
            let id = sqlx::query_scalar!(
                "SELECT block_mutate($1,$2,$3,$4,$5)",
                serde_json::to_value(&payload)?,
                serde_json::to_value(&sidecar)?,
                emitter.uuid(),
                ctx_meta,
                ctx_inv,
            )
            .fetch_one(&mut *conn)
            .await?
            .context("block_mutate returned null")?;
            Ok(Fired::Block(BlockId::from(id)))
        }

        SeedAction::CharterSet {
            cogmap,
            blocks,
            emitter,
        } => {
            let payload = payloads::CharterSet {
                cogmap_id: cogmap,
                blocks: blocks.iter().map(payloads::BlockManifest::from).collect(),
            };
            let sidecar = serde_json::to_value(payloads::content_sidecar(blocks))?;
            let telos = sqlx::query_scalar!(
                "SELECT cogmap_charter_set($1,$2,$3,$4,$5)",
                serde_json::to_value(&payload)?,
                sidecar,
                emitter.uuid(),
                ctx_meta,
                ctx_inv,
            )
            .fetch_one(&mut *conn)
            .await?
            .context("cogmap_charter_set returned null")?;
            Ok(Fired::Charter(telos.into()))
        }

        SeedAction::ResourceDelete { resource, emitter } => {
            let payload = payloads::ResourceDeleted {
                resource_id: resource,
            };
            let id = sqlx::query_scalar!(
                "SELECT resource_delete($1,$2,$3,$4)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
                ctx_meta,
                ctx_inv,
            )
            .fetch_one(&mut *conn)
            .await?
            .context("resource_delete returned null")?;
            Ok(Fired::Resource(ResourceId::from(id)))
        }

        SeedAction::ResourceUpdate {
            resource,
            title,
            origin_uri,
            emitter,
        } => {
            let payload = payloads::ResourceUpdated {
                resource_id: resource,
                title: title.map(str::to_owned),
                origin_uri: origin_uri.map(str::to_owned),
            };
            let id = sqlx::query_scalar!(
                "SELECT resource_update($1,$2,$3,$4)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
                ctx_meta,
                ctx_inv,
            )
            .fetch_one(&mut *conn)
            .await?
            .context("resource_update returned null")?;
            Ok(Fired::Resource(ResourceId::from(id)))
        }

        SeedAction::ResourceRehome {
            resource,
            home,
            emitter,
        } => {
            let payload = payloads::ResourceRehomed {
                resource_id: resource,
                home,
            };
            let id = sqlx::query_scalar!(
                "SELECT resource_rehome($1,$2,$3,$4)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
                ctx_meta,
                ctx_inv,
            )
            .fetch_one(&mut *conn)
            .await?
            .context("resource_rehome returned null")?;
            Ok(Fired::Resource(ResourceId::from(id)))
        }

        SeedAction::RelationshipRetype {
            edge,
            kind,
            polarity,
            emitter,
        } => {
            let payload = payloads::RelationshipRetyped {
                edge_id: edge,
                edge_kind: kind,
                polarity,
            };
            let id = sqlx::query_scalar!(
                "SELECT relationship_retype($1,$2,$3,$4)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
                ctx_meta,
                ctx_inv,
            )
            .fetch_one(&mut *conn)
            .await?
            .context("relationship_retype returned null")?;
            Ok(Fired::Relationship(EdgeId::from(id)))
        }

        SeedAction::RelationshipReweight {
            edge,
            weight,
            emitter,
        } => {
            let payload = payloads::RelationshipReweighted {
                edge_id: edge,
                weight,
            };
            let id = sqlx::query_scalar!(
                "SELECT relationship_reweight($1,$2,$3,$4)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
                ctx_meta,
                ctx_inv,
            )
            .fetch_one(&mut *conn)
            .await?
            .context("relationship_reweight returned null")?;
            Ok(Fired::Relationship(EdgeId::from(id)))
        }

        SeedAction::InvocationOpen {
            invocation,
            trigger_kind,
            originating,
            parent,
            scoped_entity,
            emitter,
        } => {
            let payload = payloads::DelegatedLaunch {
                invocation_id: invocation,
                trigger_kind: trigger_kind.to_owned(),
                originating_cogmap_id: originating,
                parent_cogmap_id: parent,
                scoped_entity_id: scoped_entity,
            };
            let id = sqlx::query_scalar!(
                "SELECT invocation_open($1,$2)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?
            .context("invocation_open returned null")?;
            Ok(Fired::Invocation(InvocationId::from(id)))
        }

        SeedAction::InvocationClose {
            invocation,
            disposition,
            outcome,
            originating: _,
            emitter,
        } => {
            let payload = payloads::InvocationClosed {
                invocation_id: invocation,
                disposition,
                outcome,
            };
            sqlx::query_scalar!(
                "SELECT invocation_close($1,$2)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?;
            Ok(Fired::Invocation(invocation))
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
                lens: LensId::from(Uuid::nil()),
                watermark: EventId::from(Uuid::nil()),
                membership_fingerprint: "",
                region_ids: &[],
                emitter,
            }
            .event_type()
            .as_canonical_name(),
            "region_materialized"
        );
        assert_eq!(
            SeedAction::RelationshipFold {
                edge: crate::ids::EdgeId::from(Uuid::nil()),
                reason: None,
                emitter,
            }
            .event_type()
            .as_canonical_name(),
            "relationship_folded"
        );
        let chunks: Vec<PreparedChunk> = Vec::new();
        assert_eq!(
            SeedAction::BlockMutate {
                block: BlockId::from(Uuid::nil()),
                chunks: &chunks,
                emitter,
            }
            .event_type()
            .as_canonical_name(),
            "block_mutated"
        );
    }
}
