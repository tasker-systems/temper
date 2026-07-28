-- Migration: add `status` to kb_resource_workflow_props.
--
-- `temper resource list --status <x>` accepted any value — including values outside the
-- goal-status enum — and filtered NOTHING, returning every row. The flag was plumbed
-- through the CLI but `ResourceListParams` carried no `status` field, so the filter never
-- reached SQL. Worse than a missing filter: under it, PRESENCE is the lie. A row returned
-- under `--status active` was no evidence whatsoever that the row was active, and a
-- status-truth pass over the goal list on 2026-07-27 reported "43 goals active" on its
-- strength when three were already `completed`.
--
-- The pivot view is where the fix belongs, because `--stage` — the filter that DOES work —
-- reads `wp.stage` from here. Adding a sixth key makes the status filter structurally
-- identical to the stage filter rather than a parallel mechanism that can drift from it.
--
-- CREATE OR REPLACE (not DROP + CREATE): a replace may APPEND columns but may not reorder
-- or retype existing ones, so `status` goes last even though it reads better beside
-- `stage`. That constraint is also what makes this safe to auto-deploy on `main` —
-- every existing consumer of the view sees an unchanged leading column set.
--
-- Aggregate-FILTER form and the `seq`-stays-text rationale are unchanged from
-- 20260709000002; see that migration's header for why this is not a LEFT-JOIN chain.
CREATE OR REPLACE VIEW kb_resource_workflow_props AS
SELECT owner_id AS resource_id,
       MAX(property_value #>> '{}') FILTER (WHERE property_key = 'temper-stage')  AS stage,
       MAX(property_value #>> '{}') FILTER (WHERE property_key = 'temper-mode')   AS mode,
       MAX(property_value #>> '{}') FILTER (WHERE property_key = 'temper-effort') AS effort,
       MAX(property_value #>> '{}') FILTER (WHERE property_key = 'temper-seq')    AS seq,
       MAX(property_value #>> '{}') FILTER (WHERE property_key = 'temper-status') AS status
  FROM kb_properties
 WHERE owner_table = 'kb_resources'
   AND NOT is_folded
   AND property_key IN ('temper-stage', 'temper-mode', 'temper-effort', 'temper-seq',
                        'temper-status')
 GROUP BY owner_id;
