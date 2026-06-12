-- ============================================================================
-- Temper — Arc-1 destination schema (one-shot artifact, NOT a migration)
-- ----------------------------------------------------------------------------
-- This is the fresh, destination shape the five+one Arc-1 specs describe, written
-- as a *target* so it can be loaded into a separate Postgres namespace alongside
-- the live `public.*` schema and evaluated empirically (seed + scenario queries)
-- before any phased migration is written. Read the real delta vs. `public.*`,
-- then ground the migration work on what the model actually does.
--
-- Namespace: everything lands in `temper_next`. Extensions (`vector`, the
-- `uuid_generate_v7()` generator) live in `public` and are reached via search_path.
--
-- Source specs (docs/superpowers/specs/):
--   2026-06-01-data-model-reconciliation-design.md      (resources/homes/properties/edges)
--   2026-06-02-access-capability-model-design.md        (descriptor, edge-home, teams:RBAC)
--   2026-06-02-map-regions-self-materialized-shape-surface-design.md  (cogmaps, regions)
--   2026-06-02-map-to-map-delegation-dissolution-design.md            (cogmaps_share_a_team)
--   2026-06-03-content-block-primitive-design.md        (blocks/chunks/revisions/provenance)
--   2026-06-04-domain-b-charter-questions-regulation-edge-semantics-design.md  (Domain-B)
--
-- All 17 plan-level leans were promoted to decisions on 2026-06-04 (commit 9335afb);
-- the ones with DDL consequence are marked [LEAN→DECISION] inline below.
--
-- Out of scope for this artifact (operational/sync Domain-A tables not central to
-- evaluating the cognitive-map model): kb_blob_files, kb_ingestion_records,
-- kb_device_sync_state, kb_transfers, kb_team_invitations, kb_join_requests,
-- kb_profile_auth_links, kb_resource_search_index (FTS rebuilt by trigger in prod).
-- ============================================================================

DROP SCHEMA IF EXISTS temper_next CASCADE;
CREATE SCHEMA temper_next;
SET search_path TO temper_next, public;

-- ============================================================================
-- ENUMS
-- ============================================================================

-- Carried unchanged from the built schema -----------------------------------
CREATE TYPE team_role     AS ENUM ('owner', 'maintainer', 'member', 'watcher');
CREATE TYPE edge_polarity AS ENUM ('forward', 'inverse');

-- edge_kind: the four structural classes. edge_kind = structural class;
-- the row's `label` carries the domain relationship (Domain-B §4 carve).
--   express  — abstract prior → local operationalization (skill→cogmap, telos→regulation)
--   contains — hierarchical has-a
--   leads_to — causal
--   near     — non-hierarchical, override-by-construction
CREATE TYPE edge_kind AS ENUM ('express', 'contains', 'leads_to', 'near');

-- NEW: profile-level system-access status (access §6). Virtual root-team
-- membership is *derived* from this — no stored membership row.
--   none     — no system access
--   approved — read-only ceiling on root/system-public content
--   admin    — management tier (may author system-public content)
CREATE TYPE system_access AS ENUM ('none', 'approved', 'admin');

-- NEW: tags a block-provenance contribution's source (content-block §DDL).
CREATE TYPE provenance_source_kind AS ENUM ('event', 'resource');

-- RETIRED (intentionally absent): `access_level` (vault|mutable|immutable) →
-- replaced by four boolean columns on kb_resource_access; `porosity`
-- (access|attention) → dropped, visibility is teams:RBAC.

-- ============================================================================
-- IDENTITY & ACTORS
-- ============================================================================

-- People. system_access gates the virtual root-team membership (access §6).
CREATE TABLE kb_profiles (
    id            UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    handle        TEXT NOT NULL UNIQUE,
    display_name  TEXT NOT NULL,
    system_access system_access NOT NULL DEFAULT 'none',
    created       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- NEW: the actor of an event. "Personas are behavior; the actor is an entity"
-- (Domain-B §6). A launched agent-instance is a runtime entity carrying
-- launch-metadata; a GitHub/Linear integration is another. The event ledger
-- emits via emitter_entity_id, never a bare profile.
--
-- [LEAN→DECISION] launch-metadata home (domain-b PQ-7): an OPEN `metadata jsonb`,
-- NOT a frozen entity_kind enum. An agent-instance populates {model, platform,
-- bound_cogmap, persona, priming_telos}; an integration populates differently;
-- neither is forced into the other's shape. Keeps room for ephemeral-vs-long-lived
-- and future profile-property promotion. The typed-typology cut, if ever earned,
-- is deferred to the event-substrate spec on real evidence.
CREATE TABLE kb_entities (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    profile_id  UUID NOT NULL REFERENCES kb_profiles(id),   -- the human/principal behind the actor
    name        TEXT NOT NULL,
    metadata    JSONB NOT NULL DEFAULT '{}'::jsonb,
    created     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_kb_entities_profile ON kb_entities(profile_id);

-- Navigation home anchor (Domain-A). A resource homes in a context.
CREATE TABLE kb_contexts (
    id       UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    name     TEXT NOT NULL UNIQUE,
    created  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Topic taxonomy (event ledger dimension; carried, minimal).
CREATE TABLE kb_topics (
    id         UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    fqdn       TEXT NOT NULL UNIQUE,
    parent_id  UUID REFERENCES kb_topics(id),
    created    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ============================================================================
-- TEAMS (RBAC) — access §4/§6
-- ============================================================================

CREATE TABLE kb_teams (
    id       UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    slug     TEXT NOT NULL UNIQUE,
    name     TEXT NOT NULL,
    created  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- team_role re-grounded as a (management_tier, member_ceiling) pair (access §6);
-- the enum is unchanged.
CREATE TABLE kb_team_members (
    team_id     UUID NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    profile_id  UUID NOT NULL REFERENCES kb_profiles(id) ON DELETE CASCADE,
    role        team_role NOT NULL,
    created     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (team_id, profile_id)
);
CREATE INDEX idx_kb_team_members_profile ON kb_team_members(profile_id);

-- Teams DAG. vis(T) is DAG-expanded DOWN-only: a descendant inherits an
-- ancestor/umbrella's grants; an ancestor gains no visibility into a descendant's
-- privates (access §4 A2-3). Every team descends from the temper-system root.
CREATE TABLE kb_teams_parents (
    child_id   UUID NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    parent_id  UUID NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    PRIMARY KEY (child_id, parent_id),
    CHECK (child_id <> parent_id)
);

-- ============================================================================
-- RESOURCES, HOMES, COGMAPS
-- ============================================================================

-- Slimmed to identity (data-model §1). Anchor/ownership moved to homes; doctype
-- and slug left entirely (doctype → kb_properties key='doc_type'; slug retired).
CREATE TABLE kb_resources (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    title       TEXT NOT NULL,
    origin_uri  TEXT NOT NULL,                 -- canonical source uri; not unique
    -- [LEAN→DECISION] A1: denormalized sync fingerprint — a merkle over the
    -- resource's ordered non-folded (block_id, block_body_hash) tuples. NOT
    -- identity: a derived content cache, recomputed by the block-mutation write
    -- path. sync_diff_for_device reads this column (one-column source swap).
    body_hash   TEXT,
    is_active   BOOLEAN NOT NULL DEFAULT true,
    created     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Write-envelope for block mutations (audit_id FK target on revisions).
CREATE TABLE kb_resource_audits (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    resource_id  UUID REFERENCES kb_resources(id) ON DELETE SET NULL,
    created      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Cognitive maps (map-regions §0). Was kb_scopes; renamed, porosity dropped,
-- gains the constitutive telos_resource_id FK.
--
-- [LEAN→DECISION] map↔telos creation ordering (map-regions OQ-4, SUPERSEDED by
-- spine #2): cogmap_genesis inserts the telos resource BEFORE this row, so the
-- NOT NULL FK holds at insert in one transaction — no deferred FK.
CREATE TABLE kb_cogmaps (
    id                          UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    name                        TEXT NOT NULL,
    telos_resource_id           UUID NOT NULL REFERENCES kb_resources(id),  -- the charter resource (Domain-B §1)
    -- §6 stored materialization watermark: the event the shape was last
    -- materialized under (distinct from the on-read staleness aggregate, A3-3).
    shape_materialized_event_id UUID,                                       -- FK added after kb_events
    created                     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Joins a cogmap to 0+ teams. Empty-join ⇒ default-CLOSED cogmap (access §4).
CREATE TABLE kb_team_cogmaps (
    cogmap_id  UUID NOT NULL REFERENCES kb_cogmaps(id) ON DELETE CASCADE,
    team_id    UUID NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    PRIMARY KEY (cogmap_id, team_id)
);
CREATE INDEX idx_kb_team_cogmaps_team ON kb_team_cogmaps(team_id);

-- Navigation: where a resource lives. One per resource. Polymorphic anchor —
-- no real FK (can't FK two tables); integrity is the CHECK + the seeding path.
CREATE TABLE kb_resource_homes (
    id                    UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    resource_id           UUID NOT NULL UNIQUE REFERENCES kb_resources(id) ON DELETE CASCADE,
    anchor_table          VARCHAR(64) NOT NULL CHECK (anchor_table IN ('kb_contexts', 'kb_cogmaps')),
    anchor_id             UUID NOT NULL,
    originator_profile_id UUID NOT NULL REFERENCES kb_profiles(id),
    owner_profile_id      UUID NOT NULL REFERENCES kb_profiles(id),
    created               TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_kb_resource_homes_anchor ON kb_resource_homes(anchor_table, anchor_id);

-- Additive grants beyond the home anchor (access §2). Subsumes kb_team_resources.
-- [LEAN→DECISION] B3 descriptor: four boolean capability columns (NOT an
-- access_level enum, NOT a bitmask) — readable directly in SQL, keeping the §6
-- role-mask intersection a plain per-column AND. The verb set is exactly §2's;
-- `contribute` is a role-CEILING tier, not a descriptor column.
-- [LEAN→DECISION] A2 grant-anchor set: ('kb_teams','kb_profiles') only. Cogmaps
-- read via the resources_accessible_to_cogmap intersection (never per-resource
-- grants); a context is a navigation home, not a grantee. A profile-anchored grant
-- is consumer-axis ONLY (leak-safety — never enters a vis(T) or a producer intersection).
CREATE TABLE kb_resource_access (
    id                    UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    resource_id           UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    anchor_table          VARCHAR(64) NOT NULL CHECK (anchor_table IN ('kb_teams', 'kb_profiles')),
    anchor_id             UUID NOT NULL,
    can_read              BOOLEAN NOT NULL DEFAULT false,
    can_write             BOOLEAN NOT NULL DEFAULT false,
    can_delete            BOOLEAN NOT NULL DEFAULT false,
    can_grant             BOOLEAN NOT NULL DEFAULT false,
    granted_by_profile_id UUID NOT NULL REFERENCES kb_profiles(id),
    granted_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (resource_id, anchor_table, anchor_id),
    -- §2 coherence: you cannot mutate or re-share what you cannot read.
    CHECK ((can_write OR can_delete OR can_grant) <= can_read)
);
CREATE INDEX idx_kb_resource_access_anchor   ON kb_resource_access(anchor_table, anchor_id);
CREATE INDEX idx_kb_resource_access_resource ON kb_resource_access(resource_id);

-- ============================================================================
-- EVENT LEDGER — the append-only spine everything projects from
-- ============================================================================

CREATE TABLE kb_event_types (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    name            TEXT NOT NULL UNIQUE,
    -- The published contract: current JSON-Schema for this type's payload (NULL =
    -- unregistered/permissive — foreign/webhook types may stay NULL). Stamped by the
    -- boot-seed from the committed schema-artifact/payloads/*.schema.json snapshots.
    payload_schema  JSONB,
    -- First-class version declaration. Evolution: additive-only within a version;
    -- a breaking change bumps this and registers the new schema.
    schema_version  INT NOT NULL DEFAULT 1,
    created         TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The unified ledger. DELTA vs. public.kb_events: the built ledger emits via
-- (profile_id, device_id); the Arc-1 target emits via emitter_entity_id (spine
-- #2 "actor is an entity"). Legacy payload/references/resource_id/kb_context_id
-- columns collapse into metadata + the producing anchor.
--
-- [PHASE-2 DECISION] producing-anchor (formerly kb_events.scope_id): every homed
-- object (edges/properties/regions) carries its OWN gating (anchor_table,
-- anchor_id), so the event's producing anchor is PROVENANCE, not the gate. Modeled
-- polymorphic + nullable over ('kb_contexts','kb_cogmaps') — faithful to the
-- edge-home set, costs little.
CREATE TABLE kb_events (
    id                     UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    event_type_id          UUID NOT NULL REFERENCES kb_event_types(id),
    emitter_entity_id      UUID NOT NULL REFERENCES kb_entities(id),
    topic_id               UUID REFERENCES kb_topics(id),
    producing_anchor_table VARCHAR(64) CHECK (producing_anchor_table IN ('kb_contexts', 'kb_cogmaps')),
    producing_anchor_id    UUID,
    correlation_id         UUID,                              -- groups a multi-event act (e.g. a block's stream)
    -- Typed, per-event-type, replay-sufficient (payload-first design, 2026-06-09 spec §1/§3).
    -- The projection halves (_project_*) read ONLY this.
    payload                JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- Typed provenance pointers: [{rel: supersedes|derived_from|touches, target:{kind,id}}]
    "references"           JSONB NOT NULL DEFAULT '[]'::jsonb,
    -- Which registered schema version this row's payload conforms to.
    payload_version        INT   NOT NULL DEFAULT 1,
    metadata               JSONB NOT NULL DEFAULT '{}'::jsonb,
    occurred_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    created                TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- both-null-or-both-set for the polymorphic producing anchor
    CHECK ((producing_anchor_table IS NULL) = (producing_anchor_id IS NULL))
);
CREATE INDEX idx_kb_events_emitter     ON kb_events(emitter_entity_id, occurred_at DESC);
CREATE INDEX idx_kb_events_type        ON kb_events(event_type_id);
CREATE INDEX idx_kb_events_correlation ON kb_events(correlation_id);
CREATE INDEX idx_kb_events_references  ON kb_events USING GIN ("references" jsonb_path_ops);

-- Append-only enforcement (parity with production 20260522000001): supersession and correction are
-- themselves events; the ledger row is final. Safe to add now — no mutation function UPDATEs
-- kb_events anymore (identity-as-input made the genesis anchor known up front).
CREATE FUNCTION kb_events_append_only() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'event ledger is append-only';
END;
$$;
CREATE TRIGGER kb_events_append_only
    BEFORE UPDATE OR DELETE ON kb_events
    FOR EACH ROW EXECUTE FUNCTION kb_events_append_only();

-- Deferred FK from kb_cogmaps to the ledger (table created after kb_cogmaps).
ALTER TABLE kb_cogmaps
    ADD CONSTRAINT fk_kb_cogmaps_shape_event
    FOREIGN KEY (shape_materialized_event_id) REFERENCES kb_events(id);

-- ============================================================================
-- CONTENT BLOCKS — resource ⊃ blocks ⊃ chunks (content-block spec)
-- ============================================================================

-- The unit of content. A projection of the block's correlation-keyed event stream.
-- Blocks carry NO prose (text stays emergent from chunks, β overlay).
CREATE TABLE kb_content_blocks (
    id                UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    resource_id       UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    seq               INT  NOT NULL,                  -- flat ordering within the resource
    -- [LEAN→DECISION] block-fold indexing: folding is an act on VISIBILITY (same
    -- category as edge-fold), orthogonal to chunk currency. NOT is_folded is the
    -- availability gate; is_current (on chunks) stays a true statement about the
    -- chunk. Reads filter on both; the vector index is partial on both.
    is_folded         BOOLEAN NOT NULL DEFAULT false,
    genesis_event_id  UUID NOT NULL REFERENCES kb_events(id),  -- correlation root of the block's stream
    last_event_id     UUID NOT NULL REFERENCES kb_events(id),  -- most recent event to change the block
    created           TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (resource_id, seq)
);
CREATE INDEX idx_kb_content_blocks_resource ON kb_content_blocks(resource_id) WHERE NOT is_folded;

-- Chunks gain block_id (lifecycle anchor), keep resource_id (denormalized,
-- immutable = block's resource_id) so resource-scoped reads stay intact.
CREATE TABLE kb_chunks (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    block_id        UUID NOT NULL REFERENCES kb_content_blocks(id) ON DELETE CASCADE,
    resource_id     UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,  -- denormalized, immutable
    chunk_index     INT  NOT NULL,
    version         INT  NOT NULL DEFAULT 1,
    header_path     TEXT,
    heading_depth   SMALLINT,
    content_hash    TEXT NOT NULL,
    embedding       vector(768),
    is_current      BOOLEAN NOT NULL DEFAULT true,   -- latest revision of THIS chunk; orthogonal to block fold
    created         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (block_id, chunk_index, version)           -- dedup at block grain
);
CREATE INDEX idx_kb_chunks_block    ON kb_chunks(block_id);
CREATE INDEX idx_kb_chunks_resource ON kb_chunks(resource_id);
-- [LEAN→DECISION] partial HNSW: is_current AND NOT is_folded (join to the block).
-- A partial index can't reference another table, so the fold predicate rides the
-- query join; the index is partial on the chunk-local is_current. (Production may
-- denormalize is_folded onto the chunk if the join proves hot — measure first.)
CREATE INDEX idx_kb_chunks_embedding ON kb_chunks
    USING hnsw (embedding vector_cosine_ops) WHERE is_current;

-- Chunk prose (TOAST-stored), split so chunk metadata reads stay narrow.
CREATE TABLE kb_chunk_content (
    chunk_id  UUID PRIMARY KEY REFERENCES kb_chunks(id) ON DELETE CASCADE,
    content   TEXT NOT NULL
);

-- Content-version anchor at block grain (replaces resource-grain revisions).
CREATE TABLE kb_block_revisions (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    block_id        UUID NOT NULL REFERENCES kb_content_blocks(id) ON DELETE CASCADE,
    audit_id        UUID REFERENCES kb_resource_audits(id) ON DELETE SET NULL,  -- write envelope
    block_body_hash TEXT NOT NULL,
    chunk_count     INT  NOT NULL,
    created         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_kb_block_revisions_block_created ON kb_block_revisions(block_id, created DESC);

-- Per-block provenance. ACCRETES — append-only-in-spirit, never superseded.
-- [LEAN→DECISION] scar linkage (domain-b PQ-2): a scar-lesson links back to the
-- folded question-block HERE via {kind:block} provenance (source_kind='resource',
-- the regulation resource), not via a graph edge — "the lesson came from scarring."
CREATE TABLE kb_block_provenance (
    id                      UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    block_id                UUID NOT NULL REFERENCES kb_content_blocks(id) ON DELETE CASCADE,
    source_kind             provenance_source_kind NOT NULL,
    source_id               UUID NOT NULL,                              -- the contributing event/resource
    contributed_by_event_id UUID NOT NULL REFERENCES kb_events(id),     -- the block_mutated event that added it
    accretion_seq           INT  NOT NULL,                              -- monotonic order this source shaped the block
    is_corrected            BOOLEAN NOT NULL DEFAULT false,             -- rare: "this source was wrong" (a scar)
    created                 TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (block_id, source_kind, source_id, contributed_by_event_id)
);
CREATE INDEX idx_kb_block_provenance_block  ON kb_block_provenance(block_id) WHERE NOT is_corrected;
CREATE INDEX idx_kb_block_provenance_source ON kb_block_provenance(source_kind, source_id);

-- ============================================================================
-- EDGES & PROPERTIES — polymorphic, event-sourced projections
-- ============================================================================

-- Was kb_resource_edges. Endpoints become polymorphic ('kb_resources','kb_cogmaps').
-- [LEAN→DECISION] edge-home (access OQ-2): the nullable scope_id is SUPERSEDED by a
-- denormalized home (home_anchor_table, home_anchor_id) over ('kb_contexts','kb_cogmaps'),
-- projected write-once from the asserting event. An edge is a first-class
-- access-gated object homed in the same resource-terms as everything else; the
-- home gates "seeing the assertion exists" (access §3). The event stays the
-- normalized source of truth; this column makes the graph read-gate a plain join.
CREATE TABLE kb_edges (
    id                 UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    source_table       VARCHAR(64) NOT NULL CHECK (source_table IN ('kb_resources', 'kb_cogmaps')),
    source_id          UUID NOT NULL,
    target_table       VARCHAR(64) NOT NULL CHECK (target_table IN ('kb_resources', 'kb_cogmaps')),
    target_id          UUID NOT NULL,
    edge_kind          edge_kind NOT NULL,
    polarity           edge_polarity NOT NULL DEFAULT 'forward',
    label              TEXT,                            -- the domain relationship (e.g. 'operationalized_by')
    weight             DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    home_anchor_table  VARCHAR(64) NOT NULL CHECK (home_anchor_table IN ('kb_contexts', 'kb_cogmaps')),
    home_anchor_id     UUID NOT NULL,
    asserted_by_event_id UUID NOT NULL REFERENCES kb_events(id),
    last_event_id        UUID NOT NULL REFERENCES kb_events(id),
    is_folded          BOOLEAN NOT NULL DEFAULT false,
    created            TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX uq_kb_edges_assertion ON kb_edges
    (source_table, source_id, target_table, target_id, edge_kind, COALESCE(label, ''),
     home_anchor_table, home_anchor_id) WHERE NOT is_folded;
CREATE INDEX idx_kb_edges_source ON kb_edges(source_table, source_id) WHERE NOT is_folded;
CREATE INDEX idx_kb_edges_target ON kb_edges(target_table, target_id) WHERE NOT is_folded;
CREATE INDEX idx_kb_edges_home   ON kb_edges(home_anchor_table, home_anchor_id) WHERE NOT is_folded;

-- The canonical structured-meta model (data-model §3). Single shape, non-null key.
-- doctype is a row here (key='doc_type'); relational frontmatter projects to edges.
CREATE TABLE kb_properties (
    id                    UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    owner_table           VARCHAR(64) NOT NULL CHECK (owner_table IN ('kb_resources', 'kb_cogmaps', 'kb_edges', 'kb_content_blocks')),  -- §4a edges carry facets; D3 blocks carry block_role
    owner_id              UUID NOT NULL,
    property_key          TEXT NOT NULL,
    property_value        JSONB NOT NULL,
    weight                DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    asserted_by_event_id  UUID NOT NULL REFERENCES kb_events(id),
    last_event_id         UUID NOT NULL REFERENCES kb_events(id),
    is_folded             BOOLEAN NOT NULL DEFAULT false,
    created               TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (owner_table, owner_id, property_key, property_value)
);
CREATE INDEX idx_kb_properties_owner     ON kb_properties(owner_table, owner_id) WHERE NOT is_folded;
CREATE INDEX idx_kb_properties_value_gin ON kb_properties USING gin (property_value jsonb_path_ops);
CREATE INDEX idx_kb_properties_key       ON kb_properties(property_key) WHERE NOT is_folded;

-- ============================================================================
-- COGNITIVE-MAP SHAPE — self-materialized regions (map-regions spec)
-- ============================================================================

-- The surface tier. Readable by anyone who can read the map (§4). Same
-- assert/fold pattern as edges/properties — no new freshness primitive.
-- Region lenses (spec §3B): a lens IS a declared, stored, IMMUTABLE projection-class
-- instance. Editing = assert a new row; a region's lens_id pins the exact weight-vector
-- it was computed under (the reproducibility anchor). Plurality = more rows; same function.
CREATE TABLE kb_cogmap_lenses (
    id                   UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    cogmap_id            UUID REFERENCES kb_cogmaps(id),  -- NULL = global default; non-null = map-specific
    name                 TEXT NOT NULL,
    selection_kind       TEXT NOT NULL DEFAULT 'homed',   -- 'homed' (this plan); 'team_visible' later
    w_express            DOUBLE PRECISION NOT NULL,
    w_contains           DOUBLE PRECISION NOT NULL,
    w_leads_to           DOUBLE PRECISION NOT NULL,
    w_near               DOUBLE PRECISION NOT NULL,
    w_prop               DOUBLE PRECISION NOT NULL,
    s_telos              DOUBLE PRECISION NOT NULL,
    s_ref                DOUBLE PRECISION NOT NULL,
    s_central            DOUBLE PRECISION NOT NULL,
    resolution           DOUBLE PRECISION NOT NULL,
    asserted_by_event_id UUID NOT NULL REFERENCES kb_events(id),
    created              TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The INPUT-grain grouping a region is computed within (drift decision §3.2/§4). A component is a
-- connected set of the nonzero-affinity graph; regions never span components, so clustering decomposes
-- over them and a change re-clusters only its touched component(s). `fingerprint` is the SHA-256 of the
-- component's membership-determining inputs (members + intra-component edges + member facets + lens
-- affinity weights); `member_ids` is the component's node set (its identity across materializes).
-- Together they are the persisted artifact behind incremental materialization: a current component whose
-- (member_ids, fingerprint) matches a live row is provably unchanged ⇒ its regions are reused untouched.
CREATE TABLE kb_cogmap_components (
    id                   UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    cogmap_id            UUID NOT NULL REFERENCES kb_cogmaps(id) ON DELETE CASCADE,
    lens_id              UUID NOT NULL REFERENCES kb_cogmap_lenses(id),
    fingerprint          TEXT NOT NULL,               -- sha256 of membership-determining inputs
    member_ids           UUID[] NOT NULL,             -- the component's node set (sorted; its identity)
    asserted_by_event_id UUID NOT NULL REFERENCES kb_events(id),
    last_event_id        UUID NOT NULL REFERENCES kb_events(id),
    is_folded            BOOLEAN NOT NULL DEFAULT false,
    created              TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_kb_cogmap_components_live ON kb_cogmap_components(cogmap_id, lens_id) WHERE NOT is_folded;

-- [LEAN→DECISION] centroid pooling (OQ-1): pool-per-concept-then-mean (one vector
-- per member concept first), computed in MaterializeCogmapShape. No HNSW on
-- centroid yet (OQ-2: per-map scan; wanted, deferred "as we go").
CREATE TABLE kb_cogmap_regions (
    id                   UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    cogmap_id            UUID NOT NULL REFERENCES kb_cogmaps(id) ON DELETE CASCADE,
    lens_id              UUID NOT NULL REFERENCES kb_cogmap_lenses(id),  -- the perspective that produced this region (§3B)
    component_id         UUID REFERENCES kb_cogmap_components(id),       -- the input-grain group this region was clustered within (drift §4)
    centroid             vector(768) NOT NULL,
    salience             DOUBLE PRECISION NOT NULL,   -- computed blend, memoized (was agent-assigned; spec §3A)
    telos_alignment      DOUBLE PRECISION,            -- cosine(centroid, telos_resource.embedding)  [salience part]
    reference_standing   DOUBLE PRECISION,            -- aggregate reinforce_count over members        [salience part]
    centrality           DOUBLE PRECISION,            -- internal declared-affinity density × size      [salience part]
    content_cohesion     DOUBLE PRECISION,            -- mean member-to-centroid cosine (surface↔relational, §2c)
    internal_tension     DOUBLE PRECISION,            -- over oppositional-labeled declared edges among members
    label                TEXT,                        -- optional agent-authored region label
    member_count         INT NOT NULL,                -- aggregate; the blur exposed in the surface read
    asserted_by_event_id UUID NOT NULL REFERENCES kb_events(id),
    last_event_id        UUID NOT NULL REFERENCES kb_events(id),
    is_folded            BOOLEAN NOT NULL DEFAULT false,
    created              TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_kb_cogmap_regions_map ON kb_cogmap_regions(cogmap_id) WHERE NOT is_folded;

-- The interior tier. Member identities — NEVER returned wholesale by the surface
-- read; resolved per-member through resources_visible_to (and may be denied).
CREATE TABLE kb_cogmap_region_members (
    region_id     UUID NOT NULL REFERENCES kb_cogmap_regions(id) ON DELETE CASCADE,
    member_table  VARCHAR(64) NOT NULL CHECK (member_table IN ('kb_resources', 'kb_cogmaps')),
    member_id     UUID NOT NULL,
    affinity      DOUBLE PRECISION,            -- nearness to centroid (core vs peripheral)
    PRIMARY KEY (region_id, member_table, member_id)
);
CREATE INDEX idx_kb_cogmap_region_members_member ON kb_cogmap_region_members(member_table, member_id);

-- ============================================================================
-- End of 01_schema.sql. Functions → 02_functions.sql; seed → 03_seed.sql.
-- ============================================================================
