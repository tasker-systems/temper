-- The staleness clock gains the gate its two sibling doors already carry, and the context surface
-- gains the analytics read that was declared missing.
--
-- Design: internal/superpowers/specs/2026-08-25-staleness-member-gate-and-context-analytics-design.md
-- Task: 01a03636-8077-7ee2-a070-f6766658a41e, under goal 019f5c66-755e-7fc1-bd87-ee2de8e4cd3f.
--
-- ── 0. THIS MIGRATION CORRECTS A CLAIM MADE ONE MIGRATION EARLIER ────────────────────────────────
--
-- `20260823000020:50-52` says, in the header of the function this migration replaces:
--
--     "No gate here, matching the incumbent: staleness is a clock reading, and the gate lives in the
--      composers (cogmap_analytics, 20260628000001:77-78). Stale reads are allowed and LEGIBLE --
--      this reports staleness, it never blocks on it."
--
-- and its COMMENT ON FUNCTION at `:87` restates it as "Ungated by design; the gate lives in the
-- composers." **Both are now false, and this migration says so rather than leaving two migrations in
-- an append-only ledger disagreeing.** The correction, stated precisely, because only half of that
-- sentence was ever wrong:
--
--   * "Stale reads are allowed and LEGIBLE -- this reports staleness, it never blocks on it" REMAINS
--     TRUE and is untouched. Staleness is still reported, never enforced.
--   * "The gate lives in the composers" was true of a function that took no principal -- it had
--     nothing to gate WITH. It is false of this one. The composer's gate is on the ANCHOR
--     (`20260628000001:38-39`, a trailing WHERE on `cogmap_readable_by_profile`); it decides whether
--     the single row appears and cannot reach the clock's INPUTS. The tell was internal to that one
--     function: at `20260628000001:35` the sibling sub-read `cogmap_regulation` IS handed the
--     principal, and at `:37` `cogmap_staleness` is not, because its signature took none.
--
--   * A citation repair while we are here: `20260823000020:51` cites the composer gate as
--     `20260628000001:77-78`. That file is 40 lines long. The gate is at `20260628000001:38-39`.
--
-- ── 1. THE DEFECT ────────────────────────────────────────────────────────────────────────────────
--
-- The `touch` CTE at `20260823000020:68-80` takes `max(occurred_at)` over a UNION of a regions arm
-- and an edges arm and applies NO READABILITY PREDICATE TO EITHER. Both enumeration doors onto the
-- same anchor refuse to name a region this clock reports on:
--
--   * `anchor_shape`         -- `20260823000010:87-88`  `p_principal_kind = 'cogmap' OR seen.visible_members > 0`
--   * `anchor_region_metrics` -- `20260713000050:260-268` same rule, same words, as an EXISTS
--
-- The edges arm was broader still: no predicate at all, where the incumbent traversal read-set
-- `edges_visible_to` (`20260712000010:295-309`) requires both endpoints visible and
-- `element_trail_edge` (`20260719000010:166-168`) requires `anchor_readable_by_profile` AND
-- `endpoint_readable_by_profile` on both endpoints.
--
-- HONEST BOUNDS, and do not read more into this than it says. `latest_touch` is a `max()`: it leaks
-- at most one bit per distinct timestamp -- "something under this anchor moved at time T" -- and
-- never names the row. `is_stale` collapses that to one bit. **No differential has been measured.**
-- As with the member-count fix (`20260713000050:41-45`), this is a structural fix about what the
-- read is WILLING to say, not a fix to something it has been observed to say.
--
-- ── 2. WHAT DOES NOT CHANGE: BOTH ARMS STAY FOLD-INCLUSIVE ───────────────────────────────────────
--
-- **`is_folded` is absent from both arms deliberately and MUST STAY ABSENT.** Narrowing either arm
-- to live rows is a silent defect dressed as a tightening: a fold advances the row's `last_event_id`
-- and staleness must keep reporting it, so a fold-narrowed clock makes a STALE anchor read FRESH.
-- Stated at `20260823000020:30-32`, restated independently at that migration's COMMENT (`:87`) and
-- ledger reason (`:113`), and carried upstream by the covering index built for exactly this scan
-- (`20260708000008:10-15`: "a fold COUNTS as a staleness touch ... the function's folded-inclusive
-- predicate is correct as written, and the fix is this index rather than adding NOT is_folded").
--
-- The distinction that makes fold-inclusive-and-member-gated coherent rather than merely
-- inconsistent, from internal/agents/key-patterns.md: `resources_visible_to` and
-- `kb_resources.is_active` are AUTHORIZATION predicates (who may read); `is_folded` / `is_current`
-- are CURRENCY predicates (what is in force). Only the first class is a disclosure question. This
-- migration adds authorization and touches currency nowhere.
--
-- Note the asymmetry with `anchor_shape` and `anchor_region_metrics`, which DO carry
-- `NOT reg.is_folded` (`20260823000010:86`, `20260713000050:258`). That is not an inconsistency to
-- reconcile: those two enumerate what is IN FORCE on the map, this one measures WHEN IT LAST MOVED.
-- Same authorization rule, different currency question, on purpose.
--
-- ── 3. THE GATE IS THE FULL GATE, NOT JUST THE MEMBER HALF ───────────────────────────────────────
--
-- Gating only the member/endpoint half leaves an existence-and-clock oracle. With the arms gated but
-- no anchor gate, a denied caller gets `latest_touch = NULL` (both arms contribute nothing), the
-- COALESCE at `20260823000020:82` collapses `is_stale` to `mat.materialized_at IS NULL`, and the
-- `mat` CTE reads the anchor watermark UNCONDITIONALLY -- so a caller who cannot read a REAL anchor
-- still gets a row carrying that anchor's watermark, and an `is_stale` that reports whether it has
-- ever been materialized.
--
--     STATED PRECISELY, BECAUSE THE FIRST DRAFT OF THIS PARAGRAPH OVERSTATED IT. The design spec
--     said a denied caller learns this "for ANY uuid they invent". That is FALSE, and measured
--     false: `mat` selects from kb_contexts/kb_cogmaps WHERE id = p_anchor_id, so a uuid naming no
--     anchor produces no `mat` row, and the final `FROM mat, touch` cross join yields ZERO ROWS.
--     Verified against the incumbent on a live database -- an invented uuid returns 0 rows there
--     today. The disclosure is therefore narrower than first written, and sharper: it is about a
--     REAL anchor the caller may not read, which is the case where "this uuid names something that
--     exists" is itself the fact being handed over.
--
-- The full gate closes it by making DENY and ABSENT the same answer -- zero rows either way, so the
-- two are indistinguishable. That is the same property `anchor_shape` states as "deny and absent
-- collapse into ONE arm and disclose neither population nor clock" (`20260823000010:186`), reached
-- there by the EXISTS conjunct at `:65` and argued at `:43-58`. Handing this function a principal
-- puts it under the same obligation, so it adopts the same gate.
--
-- So the gate CTE below is carried VERBATIM from `20260823000010:59-66`, EXISTS conjunct included,
-- and the outer WHERE yields ZERO ROWS on deny -- which is what `cogmap_analytics` already produced
-- on deny (`20260628000001:38-39`), so that composer's observable behaviour does not move.
--
-- THE FAIL-OPEN SHAPE THAT IS FORBIDDEN. The gate is NOT expressed as optional principal parameters
-- where NULL means ungated. There is no ungated variant of this function, by ruling (design §0,
-- taken on a three-row caller inventory). An empty scope aggregating to NULL and falling open has
-- already bitten this schema; "ungated", if it ever returns, must be a NAME you cannot type by
-- accident, not a NULL you can pass by accident.
--
-- ── 4. WHY BOTH OLD SIGNATURES ARE DROPPED AND NOT REPLACED ──────────────────────────────────────
--
-- In Postgres, adding a parameter CREATES AN OVERLOAD. A `CREATE OR REPLACE` at the longer argument
-- list would leave `anchor_staleness(text, uuid)` and `cogmap_staleness(uuid)` STANDING and UNGATED:
-- same name, same column names, same `boolean` type. Nothing would error, no test would fail, and
-- every existing caller would keep resolving to the ungated function. That is misrouting, not drift,
-- and it is silent in both directions.
--
-- This is not a hypothetical property of Postgres -- this schema already carries live proof of it:
-- `__temper_ungated_follow_from` exists under THREE signatures at once (8, 9 and 10 arguments), each
-- a generation that was added rather than replaced. Overloads accumulate here in practice.
--
-- Both old signatures are therefore explicitly DROPped below. They are the complete set: the live
-- catalog before this migration holds exactly `anchor_staleness(text,uuid)` and
-- `cogmap_staleness(uuid)`, one signature each.
--
-- `IF EXISTS` is used so a re-run against a database where the drop already landed is not a hard
-- error, matching `20260823000010:207`.

DROP FUNCTION IF EXISTS anchor_staleness(text, uuid);

CREATE FUNCTION anchor_staleness(
    p_anchor_table   text,
    p_anchor_id      uuid,
    p_principal_kind text,
    p_principal_id   uuid
)
RETURNS TABLE(materialized_at timestamptz, latest_touch timestamptz, is_stale boolean)
LANGUAGE sql STABLE AS $$
    WITH vis AS MATERIALIZED (
        -- Computed ONCE for the regions arm. Empty for a non-profile principal, which is why the
        -- cogmap self-read arms below are exemptions rather than redundancies. Carried from
        -- 20260823000010:36-38 / 20260713000050:250-252.
        SELECT v.resource_id FROM resources_visible_to(p_principal_id) v
    ),
    gate AS (
        -- VERBATIM from anchor_shape's gate CTE (20260823000010:59-66), EXISTS conjunct included.
        -- Always exactly one row (no FROM).
        --
        -- THE EXISTS CONJUNCT IS CARRIED FOR SAMENESS, NOT BECAUSE IT IS LOAD-BEARING HERE, and the
        -- difference is worth stating so nobody later "simplifies" the sibling by analogy with this
        -- one. In anchor_shape the EXISTS is load-bearing: that function forces a row via
        -- `LEFT JOIN ... ON true` (20260823000010:181) to carry its envelope, so a tautological
        -- cogmap self-read arm would hand back `emptiness` and `materialized_at` for an invented
        -- uuid -- the oracle argued at :43-58. THIS function ends in `FROM mat, touch`, and `mat` is
        -- empty for a uuid naming no anchor, so the cross join yields ZERO ROWS however the gate
        -- answers. Measured, not reasoned: the INCUMBENT has no gate at all and already returns 0
        -- rows for an invented uuid.
        --
        -- So it is kept because one rule in two places must read identically to stay in sync, not
        -- because removing it would open a hole here. Claiming otherwise would be a safety claim
        -- that is true of one function and false of another -- exactly the kind of prose
        -- 20260823000010:55-58 exists to stop outliving the person who knew the difference.
        SELECT (
            (p_principal_kind = 'profile'
                 AND anchor_readable_by_profile(p_principal_id, p_anchor_table, p_anchor_id))
            OR (p_principal_kind = 'cogmap'
                 AND p_anchor_table = 'kb_cogmaps'
                 AND p_principal_id = p_anchor_id
                 AND EXISTS (SELECT 1 FROM kb_cogmaps m WHERE m.id = p_anchor_id))
        ) AS readable
    ),
    mat AS (
        -- UNCHANGED from 20260823000020:57-67. Its inner UNION ALL subquery is byte-identical to the
        -- one in anchor_shape's `clock` CTE (20260823000010:95-101) and must stay so: the two read
        -- the SAME column off the SAME two tables, and a divergence between them would be a
        -- divergence between what a shape read calls "materialized" and what a staleness read
        -- compares against. (The two CTEs are not byte-identical in full: `clock` additionally
        -- projects `a.eid`, which its `never_clustered` arm needs and this function has no use for.)
        --
        -- Deliberately NOT gated in place: the deny path is the outer WHERE, which yields zero rows,
        -- so the watermark is never projected to a caller who fails the gate. Gating here as well
        -- would put the same predicate in two places for one function to disagree with itself later.
        SELECT ev.occurred_at AS materialized_at
        FROM (
            SELECT c.shape_materialized_event_id AS eid FROM kb_contexts c
             WHERE p_anchor_table = 'kb_contexts' AND c.id = p_anchor_id
            UNION ALL
            SELECT m.shape_materialized_event_id FROM kb_cogmaps m
             WHERE p_anchor_table = 'kb_cogmaps' AND m.id = p_anchor_id
        ) a
        LEFT JOIN kb_events ev ON ev.id = a.eid
    ),
    touch AS (
        SELECT max(occurred_at) AS latest_touch FROM (
            SELECT ev.occurred_at FROM kb_cogmap_regions reg
              JOIN kb_events ev ON ev.id = reg.last_event_id
             WHERE reg.home_anchor_table = p_anchor_table
               AND reg.home_anchor_id    = p_anchor_id
               -- NO `NOT reg.is_folded` HERE, AND THAT IS CORRECT. See header §2. A fold advances
               -- last_event_id and IS a touch; narrowing this arm makes a stale anchor read fresh,
               -- silently. Do not "tighten" it to match anchor_shape's region select.
               --
               -- The member rule, same words and same reason as both incumbent doors
               -- (20260823000010:87-88 as a count, 20260713000050:262-268 as this EXISTS): a region
               -- you can see nothing in is not a region you can see, so its clock is not your clock.
               -- The EXISTS form is taken over the CROSS JOIN LATERAL count because nothing here
               -- needs the cardinality -- only whether the region is enumerable at all.
               -- The cogmap self-read arm is exempt for the same reason it is exempt there.
               AND (p_principal_kind = 'cogmap' OR EXISTS (
                     SELECT 1
                     FROM kb_cogmap_region_members m
                     JOIN vis v ON v.resource_id = m.member_id
                     JOIN kb_resources r ON r.id = m.member_id AND r.is_active
                     WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
               ))
            UNION ALL
            SELECT ev.occurred_at FROM kb_edges e
              JOIN kb_events ev ON ev.id = e.last_event_id
             WHERE e.home_anchor_table = p_anchor_table
               AND e.home_anchor_id    = p_anchor_id
               -- Again NO `NOT e.is_folded`, deliberately, and the covering index built for this
               -- exact folded-inclusive scan is idx_kb_edges_home_all (20260708000008:21-22).
               --
               -- WHY HALF OF AN INCUMBENT AND NOT THE INCUMBENT. The obvious call here is
               -- `edges_visible_to` (20260712000010:295-309), and it is the WRONG call: it mixes the
               -- two predicate classes in one expression -- `NOT e.is_folded` at :297 (CURRENCY)
               -- alongside endpoint visibility at :302-309 (AUTHORIZATION). Calling it would import
               -- the fold narrowing this function must not have, through the back door and without a
               -- line of code to notice. What is needed is only its authorization half, and the
               -- schema already exposes that half under its own name --
               -- `endpoint_readable_by_profile` (20260624000002:292) -- which is also exactly what
               -- element_trail_edge composes for the same purpose (20260719000010:167-168). So:
               -- both endpoints, independently, and the home half is already handled by `gate`.
               --
               -- THIS ARM CANNOT BE TOO TIGHT, AND THAT IS A CONSTRAINT ARGUMENT, NOT A ROW COUNT.
               -- `endpoint_readable_by_profile` falls closed on an unrecognised table via its
               -- `ELSE false` (20260624000002:297). That branch is UNREACHABLE for a kb_edges
               -- endpoint: `kb_edges_source_table_check` and `kb_edges_target_table_check` constrain
               -- both columns to exactly ARRAY['kb_resources','kb_cogmaps'], which is precisely the
               -- domain the CASE dispatches on. So no edge can be dropped from the clock for a
               -- reason unrelated to visibility. Being drawn from a CHECK constraint rather than
               -- from present data, that holds for rows that do not exist yet.
               AND (p_principal_kind = 'cogmap' OR (
                     endpoint_readable_by_profile(p_principal_id, e.source_table, e.source_id)
                 AND endpoint_readable_by_profile(p_principal_id, e.target_table, e.target_id)
               ))
        ) t
    )
    SELECT mat.materialized_at, touch.latest_touch,
           COALESCE(touch.latest_touch > mat.materialized_at, mat.materialized_at IS NULL)
    FROM mat, touch
    -- Deny yields ZERO ROWS, never a NULL row and never an error -- the "no view from nowhere"
    -- pattern (20260628000001:1-3), and the same thing this function already does for an anchor that
    -- does not exist (`mat` contributes no row). A NULL `readable` rejects the row, so this fails
    -- CLOSED: `readable` is an expression over another function, and its two-valuedness is that
    -- function's property, not this one's (20260823000010:135-141 makes the same argument).
    WHERE (SELECT readable FROM gate);
$$;

COMMENT ON FUNCTION anchor_staleness(text, uuid, text, uuid) IS
'ON-READ staleness for EITHER anchor kind (kb_contexts or kb_cogmaps): compares the anchor''s stored shape_materialized_event_id watermark against the latest event touching the regions and edges homed on it that THIS PRINCIPAL MAY READ. Keyed on the anchor pair (home_anchor_table, home_anchor_id) -- the same key anchor_shape uses -- NOT on the vestigial kb_cogmap_regions.cogmap_id, which is NULL for every context region. GATED, in two parts, matching its two sibling doors exactly: the anchor disjunction carried verbatim from anchor_shape (20260823000010:59-66, EXISTS conjunct included -- carried so one rule reads identically in both places, NOT because it is load-bearing here: this function ends in a cross join against an empty mat CTE, so an invented uuid yields zero rows however the gate answers), and the member/endpoint rule -- a region with no visible member does not move this clock, an edge with an unreadable endpoint does not move this clock. Supersedes the "ungated by design, the gate lives in the composers" claim of 20260823000020: the composer gate is on the ANCHOR and cannot reach the clock''s inputs. Folded regions and edges remain deliberately INCLUDED: a fold advances last_event_id and is a touch, and narrowing either arm to live rows would make a stale anchor read fresh (20260708000008). Yields exactly one row for an anchor that exists and is readable; ZERO rows on deny or absence, which are indistinguishable to the caller by design. Staleness is LEGIBLE -- reported, never blocking.';

-- ── The two wrappers ─────────────────────────────────────────────────────────────────────────────
--
-- The cogmap name stays, delegating, exactly as it did at 20260823000020:103-108 -- the pattern the
-- goal endorses and which the task confirmed was NOT the source of the "one rule in four places"
-- drift. What moves is the argument list, and therefore the signature, and therefore this must be a
-- DROP: see header §4. The compile-time-checked caller at
-- crates/temper-substrate/src/scenario/runner.rs:486 is pinned to the old signature and is updated in
-- Beat B of the design; it passes the scenario owner, who never denies.

DROP FUNCTION IF EXISTS cogmap_staleness(uuid);

CREATE FUNCTION cogmap_staleness(p_cogmap uuid, p_principal_kind text, p_principal_id uuid)
RETURNS TABLE(materialized_at timestamptz, latest_touch timestamptz, is_stale boolean)
LANGUAGE sql STABLE AS $$
    SELECT s.materialized_at, s.latest_touch, s.is_stale
      FROM anchor_staleness('kb_cogmaps', p_cogmap, p_principal_kind, p_principal_id) s;
$$;

COMMENT ON FUNCTION cogmap_staleness(uuid, text, uuid) IS
'Cogmap-keyed wrapper over anchor_staleness(''kb_cogmaps'', ...). Carries no gate of its own -- it inherits the full two-part gate from the core, and deny is zero rows. Replaces cogmap_staleness(uuid), which was DROPPED rather than replaced: adding parameters in Postgres creates an overload, and the old ungated signature would otherwise have stood under the same name with the same column set, silently absorbing every existing caller.';

-- `cogmap_analytics` composes this one gated core with two other reads. One line changes: the
-- principal is passed through to the wrapper at 20260628000001:37, which is the asymmetry the whole
-- task turns on -- the sibling sub-read at :35 was ALREADY handed the principal.
--
-- Return type does not move, so CREATE OR REPLACE is legal and correct here; the argument list does
-- not move either, so there is no overload to strand.
--
-- ITS TRAILING WHERE IS DELIBERATELY KEPT, not made redundant by the core's gate. It is the only
-- thing gating `cogmap_telos(p_cogmap)`, which carries no gate of its own. It is co-extensive with
-- the core's gate on the profile arm (anchor_readable_by_profile dispatches straight to
-- cogmap_readable_by_profile for kb_cogmaps, 20260624000002:277) and slightly looser on the cogmap
-- arm (no EXISTS); the core is the tighter of the two, so the composed behaviour is the core's.
-- Observable behaviour on deny is unchanged either way: zero rows before, zero rows after.
CREATE OR REPLACE FUNCTION cogmap_analytics(p_cogmap uuid, p_principal_kind text, p_principal_id uuid)
RETURNS TABLE(telos_resource_id uuid, materialized_at timestamptz,
              latest_touch timestamptz, is_stale boolean, regulation jsonb)
LANGUAGE sql STABLE AS $$
    SELECT cogmap_telos(p_cogmap),
           s.materialized_at, s.latest_touch, s.is_stale,
           COALESCE(
             (SELECT json_agg(r) FROM cogmap_regulation(p_cogmap, p_principal_kind, p_principal_id) r),
             '[]'::json)::jsonb
    FROM cogmap_staleness(p_cogmap, p_principal_kind, p_principal_id) s
    WHERE (p_principal_kind = 'profile' AND cogmap_readable_by_profile(p_principal_id, p_cogmap))
       OR (p_principal_kind = 'cogmap' AND p_principal_id = p_cogmap);
$$;

COMMENT ON FUNCTION cogmap_analytics(uuid, text, uuid) IS
'Map-level analytics: telos charter id + staleness + the regulation set, composed from the canonical reads in one gated row. Its principal now reaches ALL THREE composed sub-reads: cogmap_regulation had it from the start, cogmap_staleness gained it (20260825000010), and cogmap_telos is gated by the trailing WHERE, which is why that WHERE is kept rather than dropped as redundant. Deny is zero rows, unchanged. regulation defaults to [] (never SQL-null).';

-- ── context_analytics: THREE columns, and the two it does not have are not oversights ────────────
--
-- The peer read `cogmap_analytics` returns five columns (20260628000001:29-30). A context has
-- nothing to put in two of them -- not "no value", but no such thing -- and the UI already says so
-- in as many words. packages/temper-ui/src/lib/graph/analysis.ts:353-362 (the NUL-byte file; read it
-- with sed/tr, never grep) documents that a context has a telos_centroid and neither a charter
-- resource nor a regulation set, and the string it guards, CONTEXT_HAS_NO_MAP_READOUT, says a
-- charter and the concepts that regulate it belong to a cognitive map, "so there is NOTHING HERE TO
-- REPORT rather than NOTHING FOUND".
--
-- That distinction decides the return type. Returning `telos_resource_id NULL, regulation '[]'`
-- would say "nothing found" about two things that cannot exist -- the precise failure that UI
-- constant was written to avoid, and what its own docstring calls being "faked as a peer field". So
-- this returns the staleness triple and nothing else, and the shape difference from its peer is the
-- answer, not a gap in it.
--
-- Like cogmap_staleness above, it carries NO gate of its own: all three columns come from the one
-- gated core, and a second copy of the predicate is a second thing to drift. A 'cogmap' principal
-- gets zero rows here by construction -- the core's self-read arm requires p_anchor_table =
-- 'kb_cogmaps' -- which is correct: a map is not a member of a context.

CREATE FUNCTION context_analytics(p_context uuid, p_principal_kind text, p_principal_id uuid)
RETURNS TABLE(materialized_at timestamptz, latest_touch timestamptz, is_stale boolean)
LANGUAGE sql STABLE AS $$
    SELECT s.materialized_at, s.latest_touch, s.is_stale
      FROM anchor_staleness('kb_contexts', p_context, p_principal_kind, p_principal_id) s;
$$;

COMMENT ON FUNCTION context_analytics(uuid, text, uuid) IS
'Context-level analytics, closing the one asymmetric row of the anchor read surface (shape, region_metrics, materialize_delta and materialize are already symmetric across the two anchor kinds; analytics was cogmap-only). Returns THREE columns -- materialized_at, latest_touch, is_stale -- and NOT the five its cogmap peer returns: a context has no charter resource and no regulation set, so telos_resource_id and regulation would be null peer fields asserting "nothing found" about two things that cannot exist. Carries no gate of its own; it inherits the full two-part gate from anchor_staleness, and deny is zero rows. A cogmap principal always gets zero rows, since the self-read arm of that gate applies only to a kb_cogmaps anchor.';

SELECT declare_migration(
    20260825000010,
    'shape-breaking',
    'The staleness clock gains a principal and the gate its two sibling doors already carry, and context gains an analytics read (task 01a03636-8077-7ee2-a070-f6766658a41e). THE GATE: anchor_staleness previously took max(occurred_at) over every region and every edge homed on the anchor with no readability predicate on either arm (20260823000020:68-80), while both enumeration doors onto that same anchor refuse to name a region with no visible member (anchor_shape 20260823000010:87-88; anchor_region_metrics 20260713000050:262-268), so the clock reported movement in regions the shape read would not admit exist. It is now gated in TWO parts: the anchor disjunction carried verbatim from anchor_shape (20260823000010:59-66) including the EXISTS conjunct, plus the member rule on the regions arm and endpoint_readable_by_profile on BOTH endpoints of the edges arm. The edges arm deliberately composes endpoint_readable_by_profile (20260624000002:292) rather than calling edges_visible_to (20260712000010:295-309), because that read-set mixes an authorization predicate with a currency one and calling it would silently import a fold narrowing this function must not have. Deny is zero rows, never an error, which is what cogmap_analytics already produced on deny, so that composer does not move. The ANCHOR half of the gate is what makes deny and absence indistinguishable, and stated precisely it closes a narrower hole than the design spec first claimed: before it, a caller who could not read a REAL anchor still received a row carrying that anchor watermark and an is_stale reporting whether it had ever been materialized. It was never an oracle over invented uuids -- mat selects on id = p_anchor_id, so a uuid naming no anchor already yielded zero rows through the cross join, measured against the incumbent on a live database. The spec said "for any uuid they invent"; that was wrong and is corrected here rather than carried forward. THE FOLD ARMS ARE PRESERVED ON PURPOSE, in both the regions arm and the edges arm: a fold advances last_event_id and IS a touch (20260708000008:10-15), so narrowing either to live rows would make a stale anchor read FRESH -- silently, since nothing would error and every value would still be a plausible timestamp. Authorization was added; currency was not touched. CLASSED SHAPE-BREAKING because both prior signatures were explicitly DROPPED rather than replaced, and that matters more than the return shape: adding a parameter in Postgres creates an OVERLOAD, so a CREATE OR REPLACE at the longer argument list would have left anchor_staleness(text, uuid) and cogmap_staleness(uuid) standing and UNGATED under the same names with the same column names and the same boolean type -- no type error, no failing test, every existing caller silently resolving to the ungated function. This schema already carries live proof that overloads accumulate here: __temper_ungated_follow_from exists under three signatures at once. There is deliberately NO ungated variant and the gate is NOT expressed as nullable parameters where NULL falls open; ungated, if it ever returns, must be a name nobody can type by accident. Consequently the compile-time-checked caller at crates/temper-substrate/src/scenario/runner.rs:486 must move in the same change. ALSO ADDED: context_analytics(uuid, text, uuid), returning three columns and not the five of its cogmap peer, because a context has no charter resource and no regulation set and a null peer field would report "nothing found" about something that cannot exist. NO DIFFERENTIAL WAS MEASURED. Unlike 20260713000050, which counted 0 of 546 regions diverging before shipping, nothing here was measured against production or any other data set. This is a structural fix about what the read is WILLING to say, not a response to an observed leak; latest_touch is a max() and discloses at most one bit per distinct timestamp, never a row identity. Anything read as a claim about live exposure would be a claim this migration does not make. THIS MIGRATION CORRECTS 20260823000020:50-52 and its COMMENT at :87, which said "no gate here ... the gate lives in the composers" -- true of a function that took no principal, false of this one; the composer gate is on the ANCHOR (20260628000001:38-39, which that header miscites as :77-78 in a 40-line file) and cannot reach the clock inputs. The other half of that sentence stands untouched: staleness is still reported and never blocking. Design: internal/superpowers/specs/2026-08-25-staleness-member-gate-and-context-analytics-design.md.'
);
