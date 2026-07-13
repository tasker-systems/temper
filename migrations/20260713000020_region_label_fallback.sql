-- T8 follow-up — make the orientation read LEGIBLE.
--
-- T8 (20260713000010) shipped `anchor_shape`, and dogfooding it against prod immediately showed the
-- problem: it works, and it cannot be read.
--
--     $ temper context shape @me/temper
--     [276]{region_id,lens_id,salience,content_cohesion,label,member_count}:
--       "019f5733-8edb-…", "019f53a8-…", 69.55, 0.944, null, 12
--       "019f5885-d153-…", "019f53a8-…", 26.83, 0.967, null, 15
--       …274 more, every label null
--
-- 276 anonymous UUIDs. The read whose entire purpose is to answer *"what is this context about"*
-- returned no answer. Measured cause: **`kb_cogmap_regions.label` is NULL for 100% of live regions —
-- 0 of 276 context regions AND 0 of 251 cogmap regions.** The producer never writes it, for either
-- anchor kind. Nothing was broken; the column has simply never been populated.
--
-- ── Why a read-side fallback, and not "fix the producer" ────────────────────────────────────────
--
-- Naming a region is a judgment (a steward's authored act, or a summarization pass) — it is not
-- something the deterministic region producer can invent at materialize time. That work may land one
-- day, and when it does `reg.label` starts winning: the COALESCE below prefers it and this fallback
-- quietly stops mattering. Until then, "the title of the region's most-affine member" is a real,
-- honest, human-legible stand-in — and it is precisely what the graph/Atlas read
-- (`graph_cogmap_territories`, 20260706130000) has always used for exactly this reason. This brings
-- the orientation reads to parity with it rather than inventing a new convention.
--
-- Measured on prod, this is the difference between a UUID and an answer:
--
--     sal   n   label
--     69.55 12  2026-04-01-r9-sveltekit-ui-design
--     26.83 15  Context regions T7 — anchor-agnostic wayfind
--     16.53  8  Deployment & Release Workflow
--     12.82  7  Temper Cloud
--     12.57  8  Trunk-change awareness: a SCIP code graph teams and stewards can watch
--
-- ── The visibility gate is LOAD-BEARING, not decoration ─────────────────────────────────────────
--
-- The caller is gated on reading the ANCHOR, but each member resource carries its OWN visibility. A
-- region can legitimately contain a resource the caller cannot read — and surfacing that resource's
-- title as the region's label would leak it, through a read whose own gate says nothing about members.
-- So the representative title is taken only from `resources_visible_to(p_principal_id)`, exactly as
-- `graph_cogmap_territories` does. A region whose only visible members are none simply keeps a NULL
-- label; it does not borrow a title the caller may not see.
--
-- `WITH … AS MATERIALIZED` is deliberate: it computes the visible set ONCE instead of re-deriving it
-- inside the LATERAL for every region. Measured on prod (@me/temper, 276 regions): **100ms → 58ms**.
--
-- ── The `cogmap` principal degrades safely, by construction ─────────────────────────────────────
--
-- `anchor_shape` also serves `p_principal_kind = 'cogmap'` (the map self-read), and
-- `resources_visible_to` takes a PROFILE. Passing a cogmap id yields the empty set, so `rep.title` is
-- NULL and the label falls back to `reg.label` — i.e. exactly today's behavior, with no leak and no
-- invented semantics. This needs no CASE: it is safe by construction. (Every Rust caller passes
-- 'profile' today; the arm exists for the agent-invocation design.)
--
-- Additive: `label` was already `text` and already nullable, so a consumer that saw NULL now sees a
-- name. No wire change, no signature change. `cogmap_shape` is a wrapper over this, so the cogmap
-- orientation read becomes legible for free.

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
    WITH vis AS MATERIALIZED (
        -- Computed once, not once per region. Empty for a non-profile principal, which is what makes
        -- the 'cogmap' self-read degrade to a NULL label rather than leak one.
        SELECT v.resource_id FROM resources_visible_to(p_principal_id) v
    )
    SELECT reg.id, reg.lens_id, reg.salience, reg.content_cohesion,
           COALESCE(reg.label, rep.title) AS label,
           reg.member_count
    FROM kb_cogmap_regions reg
    LEFT JOIN LATERAL (
        -- The most-affine VISIBLE member's title: a stand-in name until something authors a real one.
        SELECT r.title
        FROM kb_cogmap_region_members m
        JOIN vis v ON v.resource_id = m.member_id
        JOIN kb_resources r ON r.id = m.member_id AND r.is_active
        WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
        ORDER BY m.affinity DESC NULLS LAST
        LIMIT 1
    ) rep ON true
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
    ORDER BY reg.salience DESC NULLS LAST, reg.id;
$$;

COMMENT ON FUNCTION anchor_shape(text, uuid, text, uuid, uuid) IS
'Surface-tier read of an anchor''s materialized regions, for EITHER anchor kind (spec §3.7, T8). Keyed on the anchor pair (home_anchor_table, home_anchor_id), not the vestigial cogmap_id. Gate is inside the SQL (deny => zero rows, never an error). `label` falls back to the most-affine VISIBLE member''s title when the region carries no authored label — which today is every region (0 of 527 live regions have one), and without which the read returns anonymous UUIDs. The fallback is visibility-gated: a member the caller cannot read can never become the region''s name. cogmap_shape is a wrapper over this.';
