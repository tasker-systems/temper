-- T8 — context orientation reads: the region-level view of everything in a context.
--
-- Spec §3.7. The arc's whole point was to make a *context* form regions (T4/T5/T6 did), and then to
-- let someone LOOK at them. Nothing could: the orientation trio (`cogmap_shape`,
-- `cogmap_region_metrics`) is keyed on `kb_cogmap_regions.cogmap_id`, and a context region cannot
-- carry one — `cogmap_id` is a FK to `kb_cogmaps`. Measured on prod before writing this migration:
--
--     home_anchor_table | regions | cogmap_id_null
--     ------------------+---------+----------------
--     kb_cogmaps        |    2460 |              0
--     kb_contexts       |     297 |            297   <-- every one, necessarily
--
-- So the trio is not merely *unaware* of context regions, it is **structurally blind** to them: no
-- argument you can pass makes `WHERE reg.cogmap_id = p_cogmap` match a row whose `cogmap_id` IS NULL.
-- That is the gap this migration closes.
--
-- ── The shape of the fix: one body, not two families ────────────────────────────────────────────
--
-- The region table's real key has been the anchor pair (`home_anchor_table`, `home_anchor_id`) since
-- T2/M1 — `cogmap_id` is vestigial-but-populated, and there is already an index on the pair
-- (`idx_kb_cogmap_regions_anchor`). Spec §3.6 M2 states the target directly: *"Producer, readbacks,
-- and wayfind read and write only the anchor pair."* T7 already did this to wayfind.
--
-- So rather than cloning each function body into a `context_*` twin — two parallel families keyed on
-- different columns, guaranteed to drift — each read is written ONCE, anchor-generic, and the
-- existing `cogmap_*` names are re-pointed at it as thin wrappers. Same names, same signatures, same
-- result columns: every existing caller (temper-substrate readback, the API handlers, the MCP tools,
-- the CLI) keeps working untouched, and there is exactly one body to fix when a read changes.
--
-- ── Why the gate is free ───────────────────────────────────────────────────────────────────────
--
-- `anchor_readable_by_profile(p_profile, p_anchor_table, p_anchor_id)` already exists and is already
-- anchor-generic. It is a literal CASE that DELEGATES:
--
--     WHEN 'kb_cogmaps'  THEN cogmap_readable_by_profile(...)
--     WHEN 'kb_contexts' THEN context_readable_by_profile(...)   -- T1's predicate
--
-- Two consequences, both load-bearing:
--   1. The cogmap arm is **equivalent by construction** — the wrapper cannot change cogmap
--      authorization, because the generic gate calls the very predicate the old body called.
--      (Verified differentially against prod anyway: all 24 real (profile, cogmap) pairs, zero
--      disagreements.)
--   2. The context arm is `context_readable_by_profile` — precisely the T1 predicate the task says to
--      gate on, the one that honors `kb_access_grants` rather than the inline EXISTS that ignores
--      them. So "a context read-grant grants the orientation read" is satisfied by construction, not
--      by a second hand-rolled check that could drift from T1's.
--
-- ── The NULL cousin of T7's NaN trap ───────────────────────────────────────────────────────────
--
-- T7 found that a region whose members carry no embedding (a bodyless resource ⇒ zero chunks) has a
-- **zero-vector centroid**, that pgvector's `<=>` against it is `NaN`, and that Postgres sorts `NaN`
-- ABOVE every real value on `ORDER BY … DESC` — so un-guarded, those contentless regions win every
-- query. It guarded that at the consumer.
--
-- These reads return **stored scalars**, not a query cosine, so they do NOT inherit the NaN trap.
-- Measured on prod: zero NaN in any stored region column. What they DO inherit is its cousin — those
-- same regions store `content_cohesion IS NULL` (11 of them), and **Postgres sorts NULL FIRST on
-- ORDER BY … DESC** by default, for exactly the same reason. Hence `NULLS LAST` on every DESC sort
-- below. Unlike NaN, `NULLS LAST` does guard NULL.
--
-- The ORDER BY is new (the old bodies were unordered). It is additive: same rows, now in a
-- deterministic, useful order — most salient first, which is what an orientation read is FOR.

-- ─────────────────────────────────────────────────────────────────────────────
-- 1. `anchor_shape` — the surface tier, for any anchor.
--
-- Surface tier only: member identities are never returned wholesale (that is `graph_*`'s business,
-- under its own gate). The access gate lives INSIDE the SQL — a principal who cannot read the anchor
-- gets zero rows, never an error, so the read is leak-safe by construction.
--
-- The `p_principal_kind = 'cogmap'` arm is the map self-read (an agent invoked AS a cogmap reading
-- its own shape). It is preserved exactly, and deliberately does not generalize: a cogmap principal
-- is the anchor only when the anchor IS that cogmap, so a cogmap principal reads no context's
-- regions. That is today's behavior for cogmaps, unchanged, and the safe default for contexts.
CREATE OR REPLACE FUNCTION anchor_shape(
    p_anchor_table  text,
    p_anchor_id     uuid,
    p_principal_kind text,
    p_principal_id  uuid,
    p_lens          uuid DEFAULT NULL
)
RETURNS TABLE(
    region_id        uuid,
    lens_id          uuid,
    salience         double precision,
    content_cohesion double precision,
    label            text,
    member_count     integer
)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.lens_id, reg.salience, reg.content_cohesion, reg.label, reg.member_count
    FROM kb_cogmap_regions reg
    WHERE reg.home_anchor_table = p_anchor_table
      AND reg.home_anchor_id    = p_anchor_id
      AND NOT reg.is_folded
      AND (p_lens IS NULL OR reg.lens_id = p_lens)   -- default = all lenses
      AND (
        (p_principal_kind = 'profile'
             AND anchor_readable_by_profile(p_principal_id, p_anchor_table, p_anchor_id))
        OR (p_principal_kind = 'cogmap'
             AND p_anchor_table = 'kb_cogmaps'
             AND p_principal_id = p_anchor_id)
      )
    ORDER BY reg.salience DESC NULLS LAST, reg.id;   -- NULLS LAST: see the header's NULL-cousin note
$$;

COMMENT ON FUNCTION anchor_shape(text, uuid, text, uuid, uuid) IS
'Surface-tier read of an anchor''s materialized regions, for EITHER anchor kind (spec §3.7, T8). Keyed on the anchor pair (home_anchor_table, home_anchor_id) — the region table''s real key since M1 — not on the vestigial cogmap_id, which is NULL for every context region and therefore made the old cogmap_shape structurally blind to them. Gate is inside the SQL (deny => zero rows, never an error). cogmap_shape is now a wrapper over this.';

-- ─────────────────────────────────────────────────────────────────────────────
-- 2. `anchor_region_metrics` — the per-region analytics tier, for any anchor.
--
-- Same gate, same leak-safety. The metrics are the stored readouts the region producer wrote at
-- materialize time; `telos_alignment` is meaningful only under a lens with a telos term (a context
-- under `workflow-default` has w_cos = 1 and no facets, so it will commonly be NULL — that is
-- honest, not missing data).
CREATE OR REPLACE FUNCTION anchor_region_metrics(
    p_anchor_table  text,
    p_anchor_id     uuid,
    p_principal_kind text,
    p_principal_id  uuid,
    p_lens          uuid DEFAULT NULL
)
RETURNS TABLE(
    region_id          uuid,
    lens_id            uuid,
    centrality         double precision,
    content_cohesion   double precision,
    internal_tension   double precision,
    reference_standing double precision,
    telos_alignment    double precision
)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.lens_id, reg.centrality, reg.content_cohesion,
           reg.internal_tension, reg.reference_standing, reg.telos_alignment
    FROM kb_cogmap_regions reg
    WHERE reg.home_anchor_table = p_anchor_table
      AND reg.home_anchor_id    = p_anchor_id
      AND NOT reg.is_folded
      AND (p_lens IS NULL OR reg.lens_id = p_lens)
      AND (
        (p_principal_kind = 'profile'
             AND anchor_readable_by_profile(p_principal_id, p_anchor_table, p_anchor_id))
        OR (p_principal_kind = 'cogmap'
             AND p_anchor_table = 'kb_cogmaps'
             AND p_principal_id = p_anchor_id)
      )
    ORDER BY reg.centrality DESC NULLS LAST, reg.id;  -- centrality IS nullable: NULLS LAST is load-bearing here
$$;

COMMENT ON FUNCTION anchor_region_metrics(text, uuid, text, uuid, uuid) IS
'Per-region analytics-tier read for EITHER anchor kind (spec §3.7, T8). Sibling to anchor_shape''s surface tier; member identities are still never returned. cogmap_region_metrics is now a wrapper over this.';

-- ─────────────────────────────────────────────────────────────────────────────
-- 3. Re-point the cogmap trio at the generic bodies.
--
-- `CREATE OR REPLACE` keeps the name, the argument list, and the result columns byte-identical, so
-- this is invisible to every caller — temper-substrate's readback still runs
-- `SELECT … FROM cogmap_shape($1,'profile',$2,$3)` and gets exactly what it got before.
--
-- The one behavioral difference is the WHERE key: `home_anchor_table='kb_cogmaps' AND
-- home_anchor_id = p_cogmap` instead of `cogmap_id = p_cogmap`. For a cogmap region those two select
-- the same rows — every one of the 2460 cogmap regions on prod carries BOTH a cogmap_id and the
-- matching anchor pair (0 NULLs) — and the anchor pair is the key that survives M3, when `cogmap_id`
-- is dropped. Moving to it now means M3 does not have to rewrite these functions again.
CREATE OR REPLACE FUNCTION cogmap_shape(
    p_cogmap uuid,
    p_principal_kind text,
    p_principal_id uuid,
    p_lens uuid DEFAULT NULL
)
RETURNS TABLE(
    region_id        uuid,
    lens_id          uuid,
    salience         double precision,
    content_cohesion double precision,
    label            text,
    member_count     integer
)
LANGUAGE sql STABLE AS $$
    SELECT * FROM anchor_shape('kb_cogmaps', p_cogmap, p_principal_kind, p_principal_id, p_lens);
$$;

COMMENT ON FUNCTION cogmap_shape(uuid, text, uuid, uuid) IS
'Cogmap-addressed surface-tier region read. Now a thin wrapper over anchor_shape (T8) — kept as-is so existing callers need no change. Prefer anchor_shape for new work; this name goes away at M3 with the rest of the cogmap_* naming.';

CREATE OR REPLACE FUNCTION cogmap_region_metrics(
    p_cogmap uuid,
    p_principal_kind text,
    p_principal_id uuid,
    p_lens uuid DEFAULT NULL
)
RETURNS TABLE(
    region_id          uuid,
    lens_id            uuid,
    centrality         double precision,
    content_cohesion   double precision,
    internal_tension   double precision,
    reference_standing double precision,
    telos_alignment    double precision
)
LANGUAGE sql STABLE AS $$
    SELECT * FROM anchor_region_metrics('kb_cogmaps', p_cogmap, p_principal_kind, p_principal_id, p_lens);
$$;

COMMENT ON FUNCTION cogmap_region_metrics(uuid, text, uuid, uuid) IS
'Cogmap-addressed analytics-tier region read. Now a thin wrapper over anchor_region_metrics (T8). Prefer anchor_region_metrics for new work.';
