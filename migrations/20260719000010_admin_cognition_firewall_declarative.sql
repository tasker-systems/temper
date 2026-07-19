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
-- kb_events and adding only the CHECK is EVADABLE: label an admin-typed event 'cognition' and the
-- CHECK passes. The FK rejects it because (admin_type_id, 'cognition') is not a real registry pair.
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

-- ── 1. The denormalized column ────────────────────────────────────────────────────────────────
-- DEFAULT 'cognition' matches 20260718000020's reasoning: additive over an existing table, and the
-- permissive-by-default direction is safe here ONLY because the FK below rejects a defaulted row
-- whose type is admin (probe case T4). The default creates no silent hole.
ALTER TABLE kb_events ADD COLUMN category text NOT NULL DEFAULT 'cognition';

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
