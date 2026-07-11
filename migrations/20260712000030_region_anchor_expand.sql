-- M1 of expand → migrate → contract (spec §3.6). PURELY ADDITIVE: main stays auto-deployable.
--
-- The four region tables are keyed on `cogmap_id NOT NULL`. kb_resource_homes and kb_edges already
-- solved this with a polymorphic (anchor_table, anchor_id) pair; the region tier is the last place
-- that hasn't. This migration adds the pair, backfills it, and leaves cogmap_id in place, dual-written,
-- so the PREVIOUS commit's code keeps working against the NEW schema.
--
-- M3 (drop cogmap_id; rename kb_cogmap_* -> kb_*) is an operator-run cutover, DEFERRED INDEFINITELY.
-- Until then the table names lie: kb_cogmap_regions will hold context regions. The COMMENTs at the
-- bottom of this file carry that honesty. Naming follows confidence, not the other way round.

-- ---------------------------------------------------------------------------
-- 1. The polymorphic anchor pair on the four region tables.
-- ---------------------------------------------------------------------------
ALTER TABLE kb_cogmap_regions
    ADD COLUMN home_anchor_table VARCHAR(64)
        CHECK (home_anchor_table IN ('kb_contexts', 'kb_cogmaps')),
    ADD COLUMN home_anchor_id UUID;

ALTER TABLE kb_cogmap_components
    ADD COLUMN home_anchor_table VARCHAR(64)
        CHECK (home_anchor_table IN ('kb_contexts', 'kb_cogmaps')),
    ADD COLUMN home_anchor_id UUID;

-- On lenses the pair is NULLABLE-as-a-pair: (NULL, NULL) = a global default lens, which is how
-- telos-default and telos-default-propheavy are seeded today (cogmap_id IS NULL).
ALTER TABLE kb_cogmap_lenses
    ADD COLUMN home_anchor_table VARCHAR(64)
        CHECK (home_anchor_table IN ('kb_contexts', 'kb_cogmaps')),
    ADD COLUMN home_anchor_id UUID;

-- A context region has no cogmap. cogmap_id is NOT NULL on both tables today, so the T3 producer's
-- INSERT would fail on every context region without this. Dropping NOT NULL only WIDENS what is
-- accepted — the pre-M2 code path always supplies cogmap_id — so it is safe on auto-deploying main.
--
-- Note what this gives up: cogmap_id carries `REFERENCES kb_cogmaps(id) ON DELETE CASCADE`, which is
-- what reaps a cogmap's regions today. A context region has cogmap_id IS NULL, so it inherits no
-- cascade, and the anchor pair cannot carry an FK because it is polymorphic (the same trade
-- kb_edges and kb_resource_homes already make). Reaping context regions on context delete is the
-- producer's job, not the schema's.
ALTER TABLE kb_cogmap_regions    ALTER COLUMN cogmap_id DROP NOT NULL;
ALTER TABLE kb_cogmap_components ALTER COLUMN cogmap_id DROP NOT NULL;

UPDATE kb_cogmap_regions    SET home_anchor_table = 'kb_cogmaps', home_anchor_id = cogmap_id;
UPDATE kb_cogmap_components SET home_anchor_table = 'kb_cogmaps', home_anchor_id = cogmap_id;
UPDATE kb_cogmap_lenses     SET home_anchor_table = 'kb_cogmaps', home_anchor_id = cogmap_id
    WHERE cogmap_id IS NOT NULL;

-- Live-region and live-component lookups now key on the anchor. The old cogmap_id indexes stay
-- until M3 (the previous commit's code still uses them).
CREATE INDEX idx_kb_cogmap_regions_anchor
    ON kb_cogmap_regions(home_anchor_table, home_anchor_id) WHERE NOT is_folded;
CREATE INDEX idx_kb_cogmap_components_anchor_live
    ON kb_cogmap_components(home_anchor_table, home_anchor_id, lens_id) WHERE NOT is_folded;

-- Region members may now be context resources.
ALTER TABLE kb_cogmap_region_members
    DROP CONSTRAINT kb_cogmap_region_members_member_table_check,
    ADD CONSTRAINT kb_cogmap_region_members_member_table_check
        CHECK (member_table IN ('kb_resources', 'kb_cogmaps', 'kb_contexts'));

-- ---------------------------------------------------------------------------
-- 2. Contexts gain the shape columns.
--
-- shape_materialized_event_id mirrors kb_cogmaps. telos_centroid is NEW on both anchors — a cogmap
-- does not carry one because its telos is a DECLARED resource (kb_cogmaps.telos_resource_id, NOT
-- NULL), whose embedding can be read directly. A context has no declared telos: it is COMPUTED from
-- the liveness-weighted goal census (spec §3.4), so it must be snapshotted to be compared against.
-- ---------------------------------------------------------------------------
ALTER TABLE kb_contexts
    ADD COLUMN shape_materialized_event_id UUID REFERENCES kb_events(id),
    -- The telos snapshot at last materialize. Gate 1 of the two-clock trigger (spec §3.5) compares
    -- the CURRENT liveness-weighted goal centroid against this; drift past epsilon refreshes salience
    -- WITHOUT re-clustering. Also what makes anchor_telos_drift() computable.
    ADD COLUMN telos_centroid vector(768);

-- ---------------------------------------------------------------------------
-- 3. Lens columns for the context regime.
--
-- DEFAULTS ARE THE POINT: every existing lens row gets w_cos = 0.0, which reproduces today's
-- declared-graph-only cogmap behavior BIT-FOR-BIT. Nothing about cogmaps changes.
-- ---------------------------------------------------------------------------
ALTER TABLE kb_cogmap_lenses
    -- Formation: the embedding term (spec §3.1).
    ADD COLUMN w_cos               DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    ADD COLUMN knn_k               INT              NOT NULL DEFAULT 12,
    ADD COLUMN cos_floor           DOUBLE PRECISION NOT NULL DEFAULT 0.55,
    -- Wayfind: the anchor-kind prior (spec §3.7, consumed in T7).
    ADD COLUMN kappa_anchor_prior  DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    -- Telos: goal liveness from the task census (spec §3.4, consumed in T5).
    ADD COLUMN telos_halflife_days DOUBLE PRECISION NOT NULL DEFAULT 30.0,
    ADD COLUMN sw_in_progress      DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    ADD COLUMN sw_backlog          DOUBLE PRECISION NOT NULL DEFAULT 0.35,
    ADD COLUMN sw_done             DOUBLE PRECISION NOT NULL DEFAULT 0.15,
    ADD COLUMN damper_paused       DOUBLE PRECISION NOT NULL DEFAULT 0.3,
    ADD COLUMN damper_completed    DOUBLE PRECISION NOT NULL DEFAULT 0.4;

-- ---------------------------------------------------------------------------
-- 4. Transitional COMMENTs. The names are wrong until M3 and we say so.
-- ---------------------------------------------------------------------------
COMMENT ON TABLE kb_cogmap_regions IS
    'TRANSITIONAL NAME. Holds regions for ANY anchor — contexts as well as cogmaps. Key on '
    '(home_anchor_table, home_anchor_id); `cogmap_id` is VESTIGIAL (dual-written for the pre-M2 code '
    'path, never read by new code). M3 drops cogmap_id and renames this to kb_regions. See '
    'docs/superpowers/specs/2026-07-11-context-regions-and-wayfinding-design.md §3.6.';
COMMENT ON COLUMN kb_cogmap_regions.cogmap_id IS
    'VESTIGIAL. Superseded by (home_anchor_table, home_anchor_id). NULL for context regions — which '
    'also means a context region inherits no ON DELETE CASCADE from kb_cogmaps; reaping those is the '
    'producer''s job. Dropped in M3. Do not read this in new code.';
COMMENT ON TABLE kb_cogmap_components IS
    'TRANSITIONAL NAME — see kb_cogmap_regions. Renamed to kb_components in M3.';
COMMENT ON COLUMN kb_cogmap_components.cogmap_id IS 'VESTIGIAL — see kb_cogmap_regions.cogmap_id.';
COMMENT ON TABLE kb_cogmap_lenses IS
    'TRANSITIONAL NAME — see kb_cogmap_regions. Renamed to kb_lenses in M3. A lens is IMMUTABLE: '
    'editing means asserting a new row. w_cos = 0.0 is the cogmap regime (declared graph only); '
    'w_cos > 0 is the context regime (embedding-primary). See spec §3.1–§3.2.';
COMMENT ON COLUMN kb_cogmap_lenses.w_cos IS
    'Weight on the sparse exact-kNN cosine affinity term. 0.0 = the cogmap regime, byte-identical to '
    'pre-2026-07 behavior. The context lens sets this to 1.0 — in a context the embedding is the '
    'PRIMARY signal of regionality, not a second-order readout.';
COMMENT ON COLUMN kb_cogmap_lenses.w_prop IS
    'Facet-overlap weight. Held at cogmap parity (0.4) in the context lens even though contexts carry '
    'ZERO facets today. A lens weight is meaning-when-present, not a frequency prior: zeroing it would '
    'make the discipline provably unrewarded, and an information system that returns no signal for '
    'signal provided gets routed around. See spec §3.2.';
COMMENT ON COLUMN kb_contexts.telos_centroid IS
    'Snapshot of the liveness-weighted goal centroid at last materialize. Gate 1 of the two-clock '
    'trigger compares the current telos against this; drift past epsilon refreshes salience without '
    're-clustering. See spec §3.5.';
