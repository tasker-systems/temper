-- The cogmap self-read gate checks that the map exists.
--
-- SUPERSEDES a claim made by 20260823000010, which is already applied in production and therefore
-- may never be amended -- "any change to one already applied is permanent and must be superseded,
-- never amended" (temper-migrate's exit-4 guidance). This migration is that supersession. The
-- function body below is 20260823000010's VERBATIM, plus one conjunct, so a reviewer can diff the
-- two files and see exactly one difference.
--
-- ── WHAT WAS WRONG ──────────────────────────────────────────────────────────────────────────────
--
-- The gate's second disjunct is a tautology over two values the CALLER supplies:
--
--     p_principal_kind = 'cogmap' AND p_anchor_table = 'kb_cogmaps' AND p_principal_id = p_anchor_id
--
-- Setting them equal is free and proves nothing, and nothing checked that a kb_cogmaps row with
-- that id exists. That was harmless for as long as the function returned a bare row set: a
-- fabricated uuid and a real-but-empty map both answered a byte-identical `[]`.
--
-- 20260823000010 made it harmful, by giving the function an envelope to speak with. Reproduced on a
-- local instance carrying that migration:
--
--     anchor_shape('kb_cogmaps', <invented uuid>, 'cogmap', <the same uuid>, NULL)
--       -> population 0 | emptiness never_clustered | materialized_at NULL
--
--     -- the same uuid through the profile arm, which is correct:
--       -> population 0 | emptiness unreadable_or_absent | materialized_at NULL
--
-- `never_clustered` is a fact about an anchor. A caller learns from any uuid they invent whether a
-- cogmap sits behind it, and for a materialized one they would also receive `materialized_at` -- an
-- existence-and-clock oracle, from a gate that verified nothing. The arm is also exempt from the
-- member gate (`p_principal_kind = 'cogmap' OR seen.visible_members > 0`) and takes the STORED
-- member_count, so the same call returns every non-folded region with an ungated count.
--
-- ── WHY IT IS FIXED RATHER THAN FOOTNOTED ───────────────────────────────────────────────────────
--
-- Not reachable from the application: every Rust call site hardcodes 'profile'
-- (temper-substrate/src/readback/mod.rs), and no 'cogmap' literal reaches this function anywhere in
-- crates/. So this closes direct-SQL residue rather than a live leak, and on its own that would
-- argue for a note.
--
-- What argues against a note is that 20260823000010 claims, without qualification, in both its
-- COMMENT and its ledger reason, "no existence oracle". A safety claim true of one arm and false of
-- another is the kind of prose that outlives the person who knew the difference -- and a ledger is
-- append-only, so the claim cannot be edited where it was made. Making the claim TRUE is the only
-- way to reconcile it.
--
-- ── TWO RATIONALES IN 20260823000010 THAT ARE WRONG, RECORDED HERE BECAUSE THEY CANNOT BE EDITED ─
--
-- 1. Its ledger reason says a lagging binary "selects columns that no longer exist in that order".
--    False. The prior caller named all six columns explicitly and sqlx indexes its own select list,
--    so ordering is irrelevant and all six still exist. The real and sufficient mechanism is the
--    second one that reason gives: the guaranteed sentinel row makes region_id NULL under a
--    `"region_id!"` override, which is a decode error. The verdict `shape-breaking` was right; half
--    its reasoning was not. DEPLOYING.md records the 2026-07-30 misclassification as wrong in its
--    reasoning rather than in its vocabulary; this is the same species, caught before it misled.
--
-- 2. Its `DO NOT ADD AN ARM HERE` comment justifies merging `nothing_visible`'s two causes by "it
--    would tell a caller how many regions they cannot read". That rule is real (20260713000050:137)
--    but the state it protects is one a PROFILE principal cannot reach: region members are written
--    by the materialize projection from the anchor's own homes, and resources_visible_to admits
--    every resource homed in a readable anchor (20260807000010:192-222). So "regions holding members
--    another tenant hid from you" is not a row the writer can produce. The merge is still correct --
--    what it actually protects is STALE membership, the ghost regions measured on prod where dead
--    -but-homed resources remain region members -- but by a different route than the one written
--    down. DO NOT take this as licence to add the fifth arm.
--
-- ── CLASS ───────────────────────────────────────────────────────────────────────────────────────
--
-- ADDITIVE, and by the definition rather than by hope: "does a binary that does not carry this
-- migration still work?" (DEPLOYING.md). Same signature, same RETURNS TABLE, same nine columns in
-- the same order; the only behavioural difference is on an arm no shipped binary can reach. A
-- lagging binary is unaffected, so this needs no operator cutover and deploys with the additive set.

CREATE OR REPLACE FUNCTION anchor_shape(
    p_anchor_table   text,
    p_anchor_id      uuid,
    p_principal_kind text,
    p_principal_id   uuid,
    p_lens           uuid DEFAULT NULL
)
RETURNS TABLE(
    population       integer,
    emptiness        text,
    materialized_at  timestamptz,
    region_id        uuid,
    lens_id          uuid,
    salience         double precision,
    content_cohesion double precision,
    label            text,
    member_count     integer
)
LANGUAGE sql STABLE AS $$
    WITH vis AS MATERIALIZED (
        -- Computed ONCE for both the rows and the population. Empty for a non-profile principal.
        SELECT v.resource_id FROM resources_visible_to(p_principal_id) v
    ),
    gate AS (
        -- Always exactly one row (no FROM), which is what keeps `env` non-empty for an anchor that
        -- is unreadable OR does not exist. Disjunction carried over from 20260713000050:126-132.
        SELECT (
            (p_principal_kind = 'profile'
                 AND anchor_readable_by_profile(p_principal_id, p_anchor_table, p_anchor_id))
            OR (p_principal_kind = 'cogmap'
                 AND p_anchor_table = 'kb_cogmaps'
                 AND p_principal_id = p_anchor_id
                 -- THE ONE CHANGE IN THIS MIGRATION. See the header.
                 AND EXISTS (SELECT 1 FROM kb_cogmaps m WHERE m.id = p_anchor_id))
        ) AS readable
    ),
    regs AS (
        SELECT reg.id AS region_id, reg.lens_id, reg.salience, reg.content_cohesion,
               COALESCE(reg.label, seen.rep_title) AS label,
               CASE
                   WHEN p_principal_kind = 'cogmap' THEN reg.member_count
                   ELSE seen.visible_members
               END AS member_count
        FROM kb_cogmap_regions reg
        CROSS JOIN LATERAL (
            SELECT count(*)::int AS visible_members,
                   (array_agg(r.title ORDER BY m.affinity DESC NULLS LAST))[1] AS rep_title
            FROM kb_cogmap_region_members m
            JOIN vis v ON v.resource_id = m.member_id
            JOIN kb_resources r ON r.id = m.member_id AND r.is_active
            WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
        ) seen
        WHERE reg.home_anchor_table = p_anchor_table
          AND reg.home_anchor_id    = p_anchor_id
          AND NOT reg.is_folded
          -- A region you can see nothing in is not a region you can see. (Cogmap arm exempt.)
          AND (p_principal_kind = 'cogmap' OR seen.visible_members > 0)
          AND (SELECT readable FROM gate)
        -- DELIBERATELY no p_lens predicate: `regs` is the ALL-LENS set. The lens narrows the ROWS
        -- returned, below; it must not narrow the denominator.
    ),
    clock AS (
        SELECT a.eid, ev.occurred_at AS materialized_at
        FROM (
            SELECT c.shape_materialized_event_id AS eid FROM kb_contexts c
             WHERE p_anchor_table = 'kb_contexts' AND c.id = p_anchor_id
            UNION ALL
            SELECT m.shape_materialized_event_id FROM kb_cogmaps m
             WHERE p_anchor_table = 'kb_cogmaps' AND m.id = p_anchor_id
        ) a
        LEFT JOIN kb_events ev ON ev.id = a.eid
    ),
    env AS (
        SELECT
            CASE WHEN g.readable THEN (SELECT count(*)::int FROM regs) ELSE 0 END AS population,
            CASE WHEN g.readable THEN (SELECT k.materialized_at FROM clock k) ELSE NULL END
                AS materialized_at,
            -- Precedence is load-bearing at EVERY step. Read the arms as one cascade, widest
            -- suppression first, finest distinction last.
            --
            -- ARM 1 (rows returned) guards the field's own contract: `emptiness` explains an EMPTY
            -- row set and nothing else. Without it, a readable anchor that holds visible regions but
            -- was never materialized returns rows AND 'never_clustered' -- a named cause attached to
            -- a non-empty answer, which contradicts the column's documented meaning. That fact is
            -- not lost by suppressing it here: `materialized_at` is NULL for exactly that anchor,
            -- which is the field that is actually about the clock.
            --
            -- ARM 2 (deny) must outrank everything below it, because every arm below names a fact
            -- about an anchor -- and a caller who fails the gate has been told nothing about this
            -- one, not even that it exists. (Such a caller never reaches arm 1: `regs` is gated too,
            -- so they have no rows.)
            --
            -- ARM 3 (population > 0) must outrank the CLOCK arm below it. The clock arm used to sit
            -- here, and it was wrong: it fired for any readable anchor with a NULL watermark, WITHOUT
            -- regard to whether the anchor was empty for this caller. A readable anchor holding
            -- visible regions, never materialized, read under a lens matching none of them answered
            -- `population: 1` alongside 'never_clustered' -- self-contradictory on its face, and it
            -- sent the caller to `context materialize` when the fix was to drop the lens. Whenever
            -- `population > 0` the anchor is NOT empty for this caller; they are looking at a
            -- narrowed view, which is precisely what 'lens_narrowed' means, and the lens is then the
            -- only cause arm 1 can have failed for. The clock fact is not lost here either, by the
            -- same argument as arm 1: `materialized_at` is NULL for that anchor.
            --
            -- ARMS 4-5 are reached only when the anchor is GENUINELY empty for this caller. (There,
            -- `count(*) FROM regs` = `population`: arm 2 has already established `readable`.) Within
            -- that, 'never_clustered' MUST precede 'nothing_visible', or a never-clustered anchor
            -- reports 'nothing_visible' and the distinction this function exists to draw is lost.
            CASE
                WHEN (SELECT count(*) FROM regs rr
                       WHERE p_lens IS NULL OR rr.lens_id = p_lens) > 0 THEN NULL
                -- `IS NOT TRUE`, not `NOT`: this must fail CLOSED on a NULL. `readable` is an
                -- expression over a different function, so its two-valuedness is that function's
                -- property, not this one's; under plain `NOT g.readable` a NULL makes the arm NULL,
                -- the arm does not fire, and the caller falls through to an arm naming a fact about
                -- an anchor they may not read. The three other consumers of `g.readable` already
                -- fail closed on NULL -- `regs`' gate (a NULL WHERE rejects the row) and both
                -- `CASE WHEN g.readable THEN ... ELSE` above (a NULL takes the ELSE). This one now
                -- matches them, so the safety no longer rests on a property of another function.
                WHEN g.readable IS NOT TRUE                THEN 'unreadable_or_absent'
                WHEN (SELECT count(*) FROM regs) > 0       THEN 'lens_narrowed'
                WHEN (SELECT k.eid FROM clock k) IS NULL   THEN 'never_clustered'
                -- 'nothing_visible' DELIBERATELY does not distinguish "this anchor formed zero
                -- regions" from "it formed regions and you can see into none of them". That is not
                -- an oversight to be repaired with a fifth arm: separating them would tell a caller
                -- how many regions they cannot read, which is exactly what the member gate carried
                -- over above (20260713000050:125) exists to forbid -- "a caller is never told how
                -- many resources they cannot read" (20260713000050:137). DO NOT ADD AN ARM HERE.
                ELSE 'nothing_visible'
            END AS emptiness
        FROM gate g
    )
    SELECT env.population, env.emptiness, env.materialized_at,
           r.region_id, r.lens_id, r.salience, r.content_cohesion, r.label, r.member_count
    FROM env
    LEFT JOIN (SELECT * FROM regs rr WHERE p_lens IS NULL OR rr.lens_id = p_lens) r ON true
    ORDER BY r.salience DESC NULLS LAST, r.region_id;
$$;

COMMENT ON FUNCTION anchor_shape(text, uuid, text, uuid, uuid) IS
'Surface-tier read of an anchor''s materialized regions plus an anchor-level envelope, for EITHER anchor kind. Returns AT LEAST ONE ROW always: an empty or unreadable anchor yields a single row with region_id NULL, carrying the envelope. `population` is the member-gated region count across ALL lenses (a real denominator under a lens filter); `emptiness` names why the row set is empty (unreadable_or_absent / never_clustered / nothing_visible / lens_narrowed, NULL when non-empty); `materialized_at` is the shape watermark, NULL when never clustered. Deny and absent collapse into ONE arm and disclose neither population nor clock -- no existence oracle, and since 20260824000010 that holds on BOTH gate arms: the cogmap self-read disjunct carries an EXISTS on kb_cogmaps, without which a caller naming itself as the anchor could learn from any invented uuid whether a materialized map sat behind it, and when. The gate is inside the SQL. The member gate, label fallback and cogmap self-read exemption are carried unchanged from 20260713000050.';

SELECT declare_migration(
    20260824000010,
    'additive',
    'The cogmap self-read gate checks that the map exists. Supersedes an unqualified claim in 20260823000010, which is applied and therefore may not be amended. That migration''s gate admitted the cogmap arm on p_principal_id = p_anchor_id alone -- two caller-supplied values with no existence check -- which was inert while the function returned a bare row set and became an existence-and-clock oracle once it returned an envelope: a fabricated uuid answered `never_clustered` where the profile arm correctly answered `unreadable_or_absent`, and a materialized map would also have disclosed materialized_at. Not reachable from the application (every Rust call site hardcodes ''profile''), so this closes direct-SQL residue rather than a live leak; it is fixed rather than footnoted because 20260823000010 claims "no existence oracle" without qualification in both its COMMENT and its ledger reason, and an append-only ledger cannot be corrected where the claim was made. ADDITIVE by the definition: same signature, same RETURNS TABLE, same nine columns in the same order, and the only behavioural difference is on an arm no shipped binary reaches -- a lagging binary is unaffected. This migration''s header also records two rationales in 20260823000010 that are wrong (its ledger reason''s column-order claim, and the threat model cited for the nothing_visible merge); both verdicts stand, the reasoning does not, and neither can be edited in place.'
);
