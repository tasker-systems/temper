-- Make the admin/cognition firewall DECLARATIVE rather than conventional.
--
-- Until now "an admin event never carries a producing anchor" held by chokepoint discipline:
-- `_event_append` is the sole SQL writer (20260624000002_canonical_functions.sql:778) and the admin
-- grant functions pass literal NULL,NULL (20260718000010:75,108). Nothing in the DATABASE forbade
-- an anchored admin event — a direct `INSERT INTO kb_events` could still mint one, and the whole
-- cognition firewall (spec 2026-07-16 §4) rests on that anchor being absent.
--
-- The published docs deliberately said "firewalled by intent" rather than "by construction" because
-- of exactly this gap (PR #489). This migration closes it, and that PR's successor flips the wording.
--
-- WHY NOT A PLAIN CHECK. The two facts live on different tables: `producing_anchor_*` on kb_events,
-- `category` on kb_event_types (20260718000020:47-53). A CHECK cannot reference another table.
--
-- WHY NOT A TRIGGER. A BEFORE INSERT trigger would be the FIRST insert trigger on kb_events. The
-- existing `kb_events_append_only` trigger does no lookup at all — it ignores NEW/OLD and raises —
-- so the ledger's hot path currently pays zero per-row lookup cost. A trigger doing a category
-- lookup per row would be new cost on every `_event_append` AND on every row of replay's bulk
-- restore (replay.rs:320-334). Declarative constraints are cheaper and are checked by the planner.
--
-- WHY THE COMPOSITE FK IS LOAD-BEARING, not belt-and-braces. Denormalizing `category` onto
-- kb_events and adding only the CHECK is EVADABLE: label an admin-typed event 'domain' and the
-- CHECK passes. The FK rejects it because (admin_type_id, 'domain') is not a real registry pair.
-- The FK is what makes the CHECK unevadable, and it is also what stops the denormalized copy from
-- drifting from the registry. Verified empirically 2026-07-19 (probe case T3).
--
-- WHY `ON UPDATE RESTRICT` AND NOT CASCADE. CASCADE would be a trap with a misleading failure:
-- reclassifying a type would cascade into UPDATEing kb_events rows, and `kb_events_append_only`
-- fires BEFORE UPDATE and raises — so the cascade dies inside a trigger whose message
-- ("event ledger is append-only") names neither constraint. RESTRICT is also correct on the merits:
-- an event's category is part of what that event WAS, and history should not be retroactively
-- reclassifiable.
--
--   CONSEQUENCE, and it binds future work: a type's category is FIXED once any event of that type
--   exists. Stamp `category` at REGISTRATION time for every new event type. The three current admin
--   types were stamped by 20260718000020 before any grant events existed, so this binds only new
--   ones.
--
-- WHY THE CHECK IS ONE-DIRECTIONAL. `admin => unanchored`, NOT the converse. `lens_created` is
-- unanchored and is NOT admin — it is system configuration — and 20260718000020 argues explicitly
-- for leaving room for that third value. A bidirectional constraint would misclassify it.


-- ── 0. Retax the category vocabulary BEFORE anything references it ─────────────────────────────
-- `20260718000020` named the non-admin bucket 'cognition'. That is a misnomer, and it misleads in
-- practice: the bucket holds `resource_created`, `resource_deleted`, `block_mutated`,
-- `property_set`, `relationship_asserted` — ordinary substrate mutations, not agent reasoning.
-- "Cognition" reads as the steward/agent-workflow story, which is a different thing entirely.
--
-- The misnomer already produced a misclassification, by that migration's OWN reasoning. Its header
-- rejected an `is_admin boolean` because "`lens_created` is both-NULL-anchored and is NOT admin —
-- it is system configuration — so a boolean would force it into a bucket that misdescribes it."
-- It then stamped `lens_created` as 'cognition' — exactly the misdescription it was avoiding. The
-- third value it wanted room for was never used. This migration uses it.
--
--   domain — ordinary knowledge-graph mutations (the trail's subject matter)
--   admin  — authority acts (unchanged; the name was always right)
--   system — configuration, e.g. `lens_created`
--
-- THIS IS A PURE RENAME. `lens_created` moving to 'system' is behaviourally inert, but NOT for the
-- reason that first suggests itself: `lens_create` DOES conditionally anchor — pass it a `cogmap_id`
-- and it writes `producing_anchor_table = 'kb_cogmaps'` (reproducible; the shipped seeds happen to
-- pass `cogmap: None`, but the scenario DSL can author a cogmap-scoped lens by hand). The reason no
-- trail ever returns it is the payload: `LensCreated` carries none of `resource_id`, `owner`,
-- `block_id`, or `edge_id`, which are the only keys either trail function joins on. Do not simplify
-- this to "it is unanchored" — that is false, and it would justify a wrong change later.
--
-- Nothing else keys on the literal (verified: only 20260718000020's two filter sites and one stale
-- comment in bootseed.rs).
--
-- THE DEFAULT IS DROPPED, and that is not incidental. `20260718000020` gave the column
-- `DEFAULT 'cognition'`; retaxing the vocabulary without touching the DEFAULT would leave a NOT NULL
-- column whose default value its own CHECK forbids — so the next migration to register an event type
-- with the idiom all eight prior ones use (`INSERT INTO kb_event_types (name, payload_schema,
-- schema_version) VALUES (...)`) would abort at apply time, naming a literal that appears nowhere in
-- the vocabulary. Rather than restore a working-but-permissive default, drop it: a type registered
-- without an explicit category now fails with a plain NOT NULL error naming the column, instead of
-- silently joining the trail allowlist. That is what the CONSEQUENCE paragraph above already demands
-- ("stamp `category` at REGISTRATION time"), so the schema now enforces what the prose asks for.
--
--   Every future event-type registration must spell `category` explicitly.
--
-- ORDERING IS LOAD-BEARING: this runs BEFORE kb_events gains its `category` column and the FK below.
-- Once that FK exists with ON UPDATE RESTRICT, this very UPDATE is refused (a type with events
-- cannot be reclassified), and the rename would need the FK dropped and the append-only trigger
-- disabled. Renaming first costs three statements; renaming later costs six and a trigger window.
ALTER TABLE kb_event_types DROP CONSTRAINT kb_event_types_category_check;

UPDATE kb_event_types SET category = 'domain' WHERE category = 'cognition';
UPDATE kb_event_types SET category = 'system' WHERE name = 'lens_created';

ALTER TABLE kb_event_types
  ADD CONSTRAINT kb_event_types_category_check
  CHECK (category IN ('domain', 'admin', 'system'));

-- Must accompany the CHECK above, in the same migration. The inherited DEFAULT 'cognition' is not a
-- member of the new vocabulary, so leaving it would make every category-omitting INSERT fail.
ALTER TABLE kb_event_types ALTER COLUMN category DROP DEFAULT;

-- The two trail readers, re-emitted from `20260718000020` VERBATIM with only the category literal
-- changed (two sites; the rewrite was diffed to prove nothing else moved). The allowlist stays an
-- allowlist for the reason that migration gives: a future category added without touching these
-- functions must be EXCLUDED from trail reads by default — the permissive direction is the
-- leaking direction.
CREATE OR REPLACE FUNCTION element_trail_node(
    p_profile uuid,
    p_resource uuid
) RETURNS TABLE (
    event_id uuid,
    kind text,
    actor_entity_id uuid,
    occurred_at timestamptz,
    metadata jsonb,
    payload jsonb,
    actor_name text
) LANGUAGE sql STABLE AS $$
    WITH ev_ids AS (
        -- `producing_anchor_table IS NOT NULL` on every arm, not once at the end: it prunes inside
        -- the index scans rather than after the UNION.
        SELECT ev.id FROM kb_events ev
         WHERE (ev.payload ->> 'resource_id')::uuid = p_resource
           AND ev.producing_anchor_table IS NOT NULL
        UNION
        SELECT ev.id FROM kb_events ev
         WHERE ev.payload -> 'owner' ->> 'table' = 'kb_resources'
           AND (ev.payload -> 'owner' ->> 'id')::uuid = p_resource
           AND ev.producing_anchor_table IS NOT NULL
        UNION
        SELECT ev.id FROM kb_events ev
         JOIN kb_content_blocks b ON b.id = (ev.payload ->> 'block_id')::uuid
        WHERE b.resource_id = p_resource
          AND ev.producing_anchor_table IS NOT NULL
    )
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata, ev.payload, en.name
    FROM ev_ids
    JOIN kb_events ev ON ev.id = ev_ids.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    JOIN kb_entities en ON en.id = ev.emitter_entity_id
    WHERE et.category = 'domain'
      AND EXISTS (
        SELECT 1 FROM resources_visible_to(p_profile) v WHERE v.resource_id = p_resource
    )
    ORDER BY ev.id;
$$;

CREATE OR REPLACE FUNCTION element_trail_edge(
    p_profile uuid,
    p_edge uuid
) RETURNS TABLE (
    event_id uuid,
    kind text,
    actor_entity_id uuid,
    occurred_at timestamptz,
    metadata jsonb,
    payload jsonb,
    actor_name text
) LANGUAGE sql STABLE AS $$
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata, ev.payload, en.name
    FROM kb_edges edg
    JOIN kb_events ev ON (ev.payload ->> 'edge_id')::uuid = edg.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    JOIN kb_entities en ON en.id = ev.emitter_entity_id
    WHERE edg.id = p_edge
      AND ev.producing_anchor_table IS NOT NULL
      AND et.category = 'domain'
      AND anchor_readable_by_profile(p_profile, edg.home_anchor_table, edg.home_anchor_id)
      AND endpoint_readable_by_profile(p_profile, edg.source_table, edg.source_id)
      AND endpoint_readable_by_profile(p_profile, edg.target_table, edg.target_id)
    ORDER BY ev.id;
$$;

-- ── 1. The denormalized column ────────────────────────────────────────────────────────────────
-- DEFAULT 'domain' is the accurate value for the overwhelming majority of writes, and is safe
-- ONLY because the FK below rejects a defaulted row whose type is admin or system (probe case T4).
-- The default creates no silent hole: a mis-defaulted row fails rather than being quietly admitted.
ALTER TABLE kb_events ADD COLUMN category text NOT NULL DEFAULT 'domain';

-- ── 2. Backfill ───────────────────────────────────────────────────────────────────────────────
-- The append-only trigger fires BEFORE UPDATE OR DELETE and raises unconditionally, so the backfill
-- CANNOT run against a live trigger and there is no DELETE+reinsert alternative either. Disabling
-- it for the duration is the only available path. This is a one-shot, in-transaction DDL window: no
-- application code runs inside it, and the trigger is re-enabled before the migration commits.
ALTER TABLE kb_events DISABLE TRIGGER kb_events_append_only;

UPDATE kb_events e
   SET category = et.category
  FROM kb_event_types et
 WHERE et.id = e.event_type_id
   AND e.category IS DISTINCT FROM et.category;

ALTER TABLE kb_events ENABLE TRIGGER kb_events_append_only;

-- ── 3. The constraints ────────────────────────────────────────────────────────────────────────
-- The FK needs a unique key on the referenced pair. `id` is already the PK so this UNIQUE is
-- logically redundant, but Postgres requires a matching unique constraint to point a composite FK at.
ALTER TABLE kb_event_types
  ADD CONSTRAINT kb_event_types_id_category_key UNIQUE (id, category);

-- NOT VALID + VALIDATE: takes a weaker lock and lets the row scan happen without blocking writes.
-- Both statements ship together here (the corpus is small and this is a single migration), but the
-- split is kept so the shape is right if a target ever needs to run them apart.
ALTER TABLE kb_events
  ADD CONSTRAINT kb_events_category_matches_type
  FOREIGN KEY (event_type_id, category) REFERENCES kb_event_types (id, category)
  ON UPDATE RESTRICT
  NOT VALID;
ALTER TABLE kb_events VALIDATE CONSTRAINT kb_events_category_matches_type;

ALTER TABLE kb_events
  ADD CONSTRAINT kb_events_admin_is_unanchored
  CHECK (category <> 'admin' OR producing_anchor_table IS NULL)
  NOT VALID;
ALTER TABLE kb_events VALIDATE CONSTRAINT kb_events_admin_is_unanchored;

-- ── 4. Carry the category through the one event writer ────────────────────────────────────────
-- `_event_append` ALREADY looks up the type row (`SELECT id INTO v_et ... WHERE name = p_type_name`),
-- so widening that SELECT to fetch `category` costs ZERO additional lookups. The category is derived
-- from the registry rather than accepted as a parameter: callers must not be able to declare an
-- event's category, or the FK's guarantee becomes advisory again.
CREATE OR REPLACE FUNCTION _event_append(
    p_type_name text, p_emitter uuid, p_anchor_table text, p_anchor_id uuid,
    p_payload jsonb,
    p_references jsonb DEFAULT '[]'::jsonb,
    p_correlation uuid DEFAULT NULL,
    p_payload_version int DEFAULT 1,
    p_metadata jsonb DEFAULT '{}'::jsonb,
    p_invocation uuid DEFAULT NULL
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_et uuid; v_cat text; v_ev uuid := uuid_generate_v7();
BEGIN
    SELECT id, category INTO v_et, v_cat FROM kb_event_types WHERE name = p_type_name;
    IF v_et IS NULL THEN RAISE EXCEPTION 'event_type % not seeded', p_type_name; END IF;
    INSERT INTO kb_events (id, event_type_id, emitter_entity_id,
                           producing_anchor_table, producing_anchor_id,
                           payload, "references", payload_version, correlation_id,
                           metadata, invocation_id, category)
    VALUES (v_ev, v_et, p_emitter, p_anchor_table, p_anchor_id,
            p_payload, p_references, p_payload_version, COALESCE(p_correlation, v_ev),
            p_metadata, p_invocation, v_cat);
    RETURN v_ev;
END;
$$;
