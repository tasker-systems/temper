-- ═══════════════════════════════════════════════════════════════════════════════════════════════
-- Selection is an act: `find-resources-with` gets its mechanic.
--
-- Task 01a0003c-7468-7b31-b6b3-81913b78d150, beat 2. The contract half landed in beat 1.
--
-- ── What this is FOR ────────────────────────────────────────────────────────────────────────────
-- Narrowing by what a resource IS was a MODIFIER on every find act — `ActInvocation.resource_filter`
-- — and of its seven fields exactly one (`doc_type`) was ever applied; the other six are refused at
-- `validate` as `filter_not_applicable`. A modifier is what composition was supposed to replace, so
-- the narrowing becomes an act that produces an id SET, and the find acts already consume one
-- through `p_bound_ids uuid[]` (20260808000030).
--
-- The rule, so the next case does not re-argue this one `[decided — 2026-08-14, Pete]`:
--
--     A narrowing that can be expressed as a set must be an act. A narrowing that cannot be a set
--     belongs to the act whose semantics it constrains.
--
-- Resource properties pass. An edge predicate on a walk fails — it constrains which HOPS are
-- permitted, not which nodes are admitted — so `edge_filter` stays on `follow-from`.
--
-- ── ADDITIVE, and that is load-bearing rather than incidental ───────────────────────────────────
-- Two CREATE FUNCTION and nothing else. No DROP, no signature change, no `CREATE OR REPLACE` on
-- anything shipped. `vercel.json`'s build runs `temper-migrate --additive-only` and halts the deploy
-- at the first shape-breaking member, so a new PARAMETER on the existing find fragments — which is
-- what applying these seven as modifiers would have needed — would have required an operator-run
-- cutover per target. A new function needs none.
--
-- ── Ordering: THERE IS NONE, and no LIMIT either ────────────────────────────────────────────────
-- This act declares `orders_by: None` and `accepts_bound_terms: []`. "Carries `doc_type = task`" is
-- not more or less true of one resource than another, so there is no quantity to rank on, and
-- ranking would mean inventing a preference the caller never expressed.
--
-- **The absent LIMIT follows from that and is not an oversight.** A selection that truncates is a
-- SAMPLE, and a sample piped into a find act bounds that act to an arbitrary subset while looking
-- like a narrowing — the silent substitution this whole surface exists against. Whatever ceiling
-- applies belongs on the act that RETURNS rows, where the response discloses it. The consequence is
-- real and accepted: a broad selection over a large corpus materializes every matching id.
--
-- ── Anchors, not resource bounds ────────────────────────────────────────────────────────────────
-- There is deliberately no `p_bound_ids`. Narrowing a selection by an upstream id set IS set
-- intersection, `CombineOp::Intersect` already expresses it, and two selections piped together are
-- one selection carrying both predicates. A parameter nothing binds is the shape that produced this
-- migration's own subject — `p_doc_type` sat hardcoded to NULL for a release, accepted and ignored.
--
-- The anchor pair is a different question and cannot ride in an id set: nothing produces "the
-- resources homed in this context" as a set, so without `p_anchor_*` the act cannot answer *"every
-- task in @me/temper"*, which is most of why it exists.
--
-- ── Gated wrapper over ungated core, same as the find family ────────────────────────────────────
-- `query_plan.rs` hoists `resources_visible_to` into ONE materialized CTE per statement and hands
-- it to every core, because the planner does not dedupe that call across sites and an N-stage
-- composition would otherwise pay N recursive team closures. So this act splits the same way, and
-- the `__temper_ungated_` prefix carries the same meaning it carries there: SOURCE DISCIPLINE
-- (`audit-ungated-fragments.sh` plus a single compiler emitter), never a database permission — the
-- application connects as the owning role.
--
-- `p_anchor_reader` is NOT a visibility gate here either. Row visibility comes exclusively from
-- `p_visible_ids`; this parameter is read only by `anchor_readable_by_profile`, `COALESCE`d to
-- false so a NULL reader denies rather than yielding a NULL that neither admits nor refuses.
--
-- ── Every predicate CONFORMS to an incumbent; one is authored, and says so ──────────────────────
-- `filtered_visible_page` (temper-services/src/backend/substrate_read.rs:211) has run six of these
-- seven against real storage since the list endpoint shipped, and this reuses its idioms and the
-- views it reads rather than restating them:
--
--   doc_type        -> `kb_resource_doc_type`, the named view  (widened here to a SET; see below)
--   stage / status  -> `kb_resource_workflow_props`, the pivot view
--   owner           -> `kb_resource_homes.owner_profile_id` OR `kb_profiles.handle` — two spellings
--   title_contains  -> `r.title ILIKE '%' || $ || '%'`
--   tags            -> case-folded `text[] @> text[]` containment over the JSONB array
--   is_active +
--   ingest_state    -> `kb_resources_live`, the named view — NOT restated as two predicates
--
-- **`facets` is the exception and is AUTHORED, not reached.** `ResourceListParams` carries no facet
-- filter and `FacetPredicate` appears nowhere outside the query contract's own types, so there is no
-- incumbent read to conform to. What IS settled is the storage: `20260730000010_facet_inner_key_grain`
-- keys a facet at the inner-key grain, one `kb_properties` row per inner key holding a single-key
-- object, so `{key,value}` is `property_value @> jsonb_build_object(key, value)`. Stated here
-- because a predicate with no incumbent is the class most likely to drift from what the rest of the
-- system means by the same word.
--
-- ── doc_type takes a SET, which the modifier could not ──────────────────────────────────────────
-- `p_doc_types text[]`, not `text`. The find fragments' `p_doc_type` is a single `text`, which is
-- why `validate` refuses a multi-value doc-type filter as inexpressible rather than merely
-- unimplemented (`capability.rs`: *"this door's doc-type narrowing holds exactly one value"*).
-- Within the field the semantics are OR, matching `EdgeFilter.labels` and the modifier's own
-- `Vec<String>`; ACROSS fields they are AND, matching every other filter here.
--
-- ── The tag case fold lives in THIS function ────────────────────────────────────────────────────
-- `filtered_visible_page` folds the bind side in Rust and the row side with `lower(t)`, and its
-- comment explains why the fold must have exactly one home: production carries six tag pairs that
-- differ only by case (authn/AuthN, ci/CI, cli/CLI, authz/AuthZ, vercel/Vercel, pr-482/PR-482), so
-- whether `CI` and `ci` are one tag is a live distinction and not a hypothetical.
--
-- Here BOTH sides fold in SQL, which is that principle applied one level lower rather than a
-- divergence from it: this function has two callers (the compiler, and tests calling the wrapper
-- directly), and a fold that lives in one of them is a fold the other can get wrong silently.
-- ═══════════════════════════════════════════════════════════════════════════════════════════════

-- ── 1. The ungated core ─────────────────────────────────────────────────────────────────────────
--
-- Every parameter after `p_visible_ids` is an AND-composed narrowing whose NULL narrows NOTHING.
-- That is the opposite polarity from `p_visible_ids`, whose NULL admits nothing — the same
-- confusable pairing 20260808000030's header warns about, and the reason the compiler supplies the
-- visible set from a fixed constant rather than from an argument any caller could fill.
CREATE FUNCTION __temper_ungated_find_resources_with(
    p_visible_ids    uuid[],
    p_doc_types      text[]  DEFAULT NULL,
    p_tags           text[]  DEFAULT NULL,
    p_facets         jsonb   DEFAULT NULL,
    p_stage          text    DEFAULT NULL,
    p_status         text    DEFAULT NULL,
    p_owner_profile  uuid    DEFAULT NULL,
    p_owner_handle   text    DEFAULT NULL,
    p_title_contains text    DEFAULT NULL,
    p_anchor_table   varchar DEFAULT NULL,
    p_anchor_id      uuid    DEFAULT NULL,
    p_anchor_reader  uuid    DEFAULT NULL)
RETURNS TABLE (resource_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT r.id
      FROM kb_resources r
      -- `is_active AND ingest_state = 'complete'` has a name; join it rather than restate it. An
      -- interrupted segmented ingest is not a document, and a second copy of that rule here would
      -- be free to drift from the one every other read uses.
      JOIN kb_resources_live lv                    ON lv.id = r.id
      -- The verdict, handed in. NOT `= ANY` — see 20260808000030's header.
      JOIN unnest(p_visible_ids) AS v(resource_id) ON v.resource_id = r.id
      -- Every resource has exactly one home, so this is an INNER join and cannot drop a row; it
      -- carries the owner and the anchor, which two predicates below read.
      JOIN kb_resource_homes h                     ON h.resource_id = r.id
     WHERE (p_doc_types IS NULL OR EXISTS (
             SELECT 1 FROM kb_resource_doc_type dt
              WHERE dt.resource_id = r.id AND dt.doc_type = ANY(p_doc_types)))
       -- `stage` and `status` are the pivot's, not properties in their own right. Read through the
       -- view for the same reason `doc_type` is: the key names (`temper-stage`, `temper-status`)
       -- live in ONE place and this is not it.
       AND (p_stage IS NULL OR EXISTS (
             SELECT 1 FROM kb_resource_workflow_props wp
              WHERE wp.resource_id = r.id AND wp.stage = p_stage))
       AND (p_status IS NULL OR EXISTS (
             SELECT 1 FROM kb_resource_workflow_props wp
              WHERE wp.resource_id = r.id AND wp.status = p_status))
       -- Owner in two spellings, exactly as `filtered_visible_page` binds them: a resolved profile
       -- id (what `@me` becomes) and a handle. Independent slots rather than one polymorphic
       -- parameter, so neither has to be parsed to know which it is.
       AND (p_owner_profile IS NULL OR h.owner_profile_id = p_owner_profile)
       AND (p_owner_handle IS NULL OR EXISTS (
             SELECT 1 FROM kb_profiles p
              WHERE p.id = h.owner_profile_id AND p.handle = p_owner_handle))
       AND (p_title_contains IS NULL OR r.title ILIKE '%' || p_title_contains || '%')
       -- Tags: AND-containment, both sides folded. A resource with no `tags` property aggregates to
       -- NULL and `NULL @> $` is NULL, so it is correctly excluded; three production resources carry
       -- `tags: []` and are likewise excluded for any non-empty filter.
       AND (p_tags IS NULL OR (
             SELECT coalesce(array_agg(DISTINCT lower(t)), '{}')
               FROM kb_properties tp
               CROSS JOIN LATERAL jsonb_array_elements_text(tp.property_value) AS t
              WHERE tp.owner_table = 'kb_resources' AND tp.owner_id = r.id
                AND tp.property_key = 'tags' AND NOT tp.is_folded
           ) @> ARRAY(SELECT lower(x) FROM unnest(p_tags) AS x))
       -- Facets: `[{"key":…,"value":…}, …]`, AND across the list. Spelled as "no listed predicate
       -- FAILS to match", which is the direct rendering of universal quantification: it short-
       -- circuits on the first unmatched predicate and computes no length.
       --
       -- `[measured — 2026-08-14]` The count-comparison alternative
       -- (`count(*) FILTER (matched) = jsonb_array_length(p_facets)`) is **equivalent**, including
       -- on the duplicate-key case both forms must call unsatisfiable. An earlier draft of this
       -- comment claimed it "silently becomes OR" there; that was an unverified rationale attached
       -- to a correct implementation, and both forms return false. The reason to prefer this one is
       -- that it says what it means, not that the other is wrong.
       AND (p_facets IS NULL OR NOT EXISTS (
             SELECT 1 FROM jsonb_array_elements(p_facets) AS f
              WHERE NOT EXISTS (
                SELECT 1 FROM kb_properties fp
                 WHERE fp.owner_table = 'kb_resources' AND fp.owner_id = r.id
                   AND fp.property_key = 'facet' AND NOT fp.is_folded
                   AND fp.property_value @> jsonb_build_object(f->>'key', f->>'value'))))
       -- The anchor guard, BOTH kinds (ADJ-1), byte-for-byte the find cores' arm.
       -- `anchor_readable_by_profile` dispatches on the table and fails closed on an unknown or NULL
       -- one; `COALESCE(…, false)` so a NULL reader denies. An unreadable anchor renders as an empty
       -- stage, never an error — the existence-oracle rule.
       AND (p_anchor_id IS NULL OR (
             COALESCE(anchor_readable_by_profile(p_anchor_reader, p_anchor_table, p_anchor_id),
                      false)
             AND h.anchor_table = p_anchor_table
             AND h.anchor_id = p_anchor_id));
$$;

-- ── 2. The gated wrapper ────────────────────────────────────────────────────────────────────────
--
-- Computes the caller's visible set once and hands it down; the principal continues as
-- `p_anchor_reader`, the only thing the core still asks a principal for.
--
-- **No `CASE` guard, unlike `query_find_exact`'s.** That guard exists because a text-only
-- `/api/search` calls the wide arm with `p_emb NULL`, which is guaranteed-empty, and evaluating the
-- gate for it costs a full recursive team closure (~3.9 ms of ~4.1 ms, measured). This act has no
-- guaranteed-empty input: every parameter is optional, and a call with all of them NULL is the
-- legitimate question *"everything I can see"*, which must expand the gate to answer.
CREATE FUNCTION query_find_resources_with(
    p_principal      uuid,
    p_doc_types      text[]  DEFAULT NULL,
    p_tags           text[]  DEFAULT NULL,
    p_facets         jsonb   DEFAULT NULL,
    p_stage          text    DEFAULT NULL,
    p_status         text    DEFAULT NULL,
    p_owner_profile  uuid    DEFAULT NULL,
    p_owner_handle   text    DEFAULT NULL,
    p_title_contains text    DEFAULT NULL,
    p_anchor_table   varchar DEFAULT NULL,
    p_anchor_id      uuid    DEFAULT NULL)
RETURNS TABLE (resource_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id
      FROM __temper_ungated_find_resources_with(
             ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v),
             p_doc_types, p_tags, p_facets, p_stage, p_status,
             p_owner_profile, p_owner_handle, p_title_contains,
             p_anchor_table, p_anchor_id, p_principal) c;
$$;

COMMENT ON FUNCTION __temper_ungated_find_resources_with(
        uuid[], text[], text[], jsonb, text, text, uuid, text, text, varchar, uuid, uuid) IS
    'Selection core for the find-resources-with act: returns the ids of resources matching an AND-composed set of property/workflow/identity narrowings, ordered by nothing and truncated by nothing. Takes its visibility verdict as p_visible_ids (NULL admits nothing) rather than computing it, so a multi-stage /api/query composition pays resources_visible_to once; the __temper_ungated_ prefix is source discipline enforced by audit-ungated-fragments.sh and a single compiler emitter, NOT a database permission. Every narrowing parameter is optional and a NULL narrows nothing — the OPPOSITE polarity from p_visible_ids. doc_type takes a SET (OR within the field, AND across fields), which the p_doc_type modifier on the find fragments could not express. Tags fold case on BOTH sides here rather than relying on a caller to fold the bind. Facets read the inner-key grain established by 20260730000010. Reads kb_resources_live, kb_resource_doc_type and kb_resource_workflow_props rather than restating their predicates. There is deliberately no p_bound_ids: narrowing a selection by an upstream set is CombineOp::Intersect, and a parameter nothing binds is how p_doc_type came to sit hardcoded to NULL for a release.';

COMMENT ON FUNCTION query_find_resources_with(
        uuid, text[], text[], jsonb, text, text, uuid, text, text, varchar, uuid) IS
    'Gated wrapper over __temper_ungated_find_resources_with: computes resources_visible_to(p_principal) once and passes the principal down as p_anchor_reader, which is read only by anchor_readable_by_profile and is not a visibility gate. Unlike query_find_exact it carries no CASE guard, because this act has no guaranteed-empty input — a call with every narrowing NULL is the legitimate question "everything I can see" and must expand the gate to answer it. Serves the find-resources-with act, whose output is a set to pipe into a find act as p_bound_ids, never rows to return: a stage running it is refused in `returns` as stage_not_returnable, since an act that orders nothing has no quantity to score its rows.';

SELECT declare_migration(
    20260814000010,
    'additive',
    'Gives the find-resources-with act its mechanic (task 01a0003c-7468-7b31-b6b3-81913b78d150, beat 2): __temper_ungated_find_resources_with plus the gated wrapper query_find_resources_with, both new. TWO CREATE FUNCTION and nothing else — no DROP, no signature change, no CREATE OR REPLACE on anything shipped — which is the point rather than a happy accident: applying these seven narrowings as MODIFIERS would have added parameters to the existing find fragments, and a new parameter makes a new function, so --additive-only would have halted the deploy and required an operator-run cutover per target. Selection returns ids ordered by nothing and truncated by nothing: the act declares orders_by None and accepts_bound_terms [], and a selection that truncates is a sample, which piped into a find act bounds it to an arbitrary subset while looking like a narrowing. There is deliberately no p_bound_ids — narrowing a selection by an upstream id set IS CombineOp::Intersect — but the anchor pair IS taken, because nothing produces "the resources homed in this context" as a set and without it "every task in @me/temper" is inexpressible. Six of the seven predicates conform to filtered_visible_page''s incumbents and read the named views (kb_resources_live, kb_resource_doc_type, kb_resource_workflow_props) rather than restating their predicates; facets is the seventh and is AUTHORED, since no read path anywhere filters by facet, against the inner-key storage grain 20260730000010 established. doc_type takes a SET, which the find fragments'' single-text p_doc_type could not express. The tag case fold lives on BOTH sides inside the SQL rather than half in a caller, because this function has two callers and a fold in one of them is a fold the other can get wrong silently.'
);
