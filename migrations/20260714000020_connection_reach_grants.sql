-- S1 chunk B2 — connection reach grants: admit 'kb_connections' as a grantable subject.
--
-- This migration does exactly ONE thing: it widens the kb_access_grants.subject_table CHECK
-- (introduced inline+unnamed in 20260630000001_access_grants_seam.sql:26, auto-named by Postgres
-- `kb_access_grants_subject_table_check`) to add 'kb_connections' to the admitted set. The single
-- atomic ALTER carries DROP+ADD together, so there is never a window in which the column is
-- unconstrained (precedent: 20260712000030_region_anchor_expand.sql:56-60).
--
-- There is deliberately NO SQL-function diff in this migration. Three grounding facts explain why —
-- and they, not the one-line ALTER, are the real review surface:
--
-- 1. profile_explicit_grant IS ALREADY POLYMORPHIC on its p_subject_table text parameter
--    (20260630000001_access_grants_seam.sql:50-70): it filters `g.subject_table = p_subject_table`
--    with no hardcoded subject set, and can() passes p_subject_table straight through (…:108). So a
--    'kb_connections' grant row becomes LIVE and queryable through can() the instant this CHECK
--    admits it — with ZERO function edit. That absence of a function diff is the expected shape of
--    this change, not an omission.
--
-- 2. THE kb_resource_homes.anchor_table COUPLING TRAP (live, unguarded by any test).
--    resources_visible_to's explicit-container-grant arm and resources_in_team_scope's
--    explicit-container-grant arm (20260712000010_context_read_predicates.sql, arms ~254-260 and
--    ~397-402) both join `g.subject_table = h.anchor_table` against kb_resource_homes. Those arms are
--    inert under THIS widening for two independent reasons: each also carries an explicit
--    `h.anchor_table = 'kb_cogmaps'` / `h.anchor_table IN ('kb_cogmaps','kb_contexts')` filter, AND
--    kb_resource_homes.anchor_table is itself CHECK-bounded to ('kb_contexts','kb_cogmaps')
--    (20260624000001_canonical_schema.sql:279, kb_resource_homes_anchor_table_check). A connection
--    grant therefore has no home row to join to. The trap: if anyone later widens the
--    kb_resource_homes anchor_table CHECK to include 'kb_connections' AND relaxes those explicit
--    per-arm filters, those two arms would silently begin admitting connection grants as RESOURCE
--    visibility. Nothing tests that coupling today. The next person who touches the resource_homes
--    CHECK should see this note.
--
-- 3. derived_access_profile ENDS `ELSE false` (20260630000001_access_grants_seam.sql:75-91): a subject
--    it does not recognize (now including 'kb_connections') gets NO derived floor. There is no
--    bootstrap self-grant fabricated for a widened subject — the first grant on a connection is minted
--    by the admin/owner gate (a later beat), NOT by a derived floor. We deliberately do NOT copy the
--    cogmap creator-self-grant-at-genesis pattern here.
--
-- Relaxing a CHECK is additive-only-safe: it invalidates no existing row (every current subject_table
-- value is still in the admitted set), so `main` stays auto-deployable.

ALTER TABLE kb_access_grants
    DROP CONSTRAINT kb_access_grants_subject_table_check,
    ADD CONSTRAINT kb_access_grants_subject_table_check
        CHECK (subject_table IN ('kb_resources','kb_contexts','kb_cogmaps','kb_connections'));
