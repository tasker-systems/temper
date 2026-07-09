-- Migration: kb_resource_workflow_props — one pivot view for the workflow property keys.
--
-- Code-quality audit 2026-06-26 (docs/code-reviews/2026-06-26-code-quality-audit.md,
-- chunk 17, CQ-13 duplicated-workflow-prop-joins). The per-key
-- `LEFT JOIN kb_properties … property_key = 'temper-…' AND NOT is_folded` block was
-- copy-pasted across three read paths (substrate readback `list`/`resource_row`,
-- temper-services `filtered_visible_page`); a renamed or added workflow key had to be
-- edited in sync at every site. This view is the single statement of that pivot; the
-- read paths JOIN it on resource_id.
--
-- Aggregate-FILTER form deliberately (not a LEFT-JOIN chain re-anchored on
-- kb_resources): one indexed scan of kb_properties per resource, no redundant
-- kb_resources self-join, and the planner pushes a joined resource-id qual into the
-- GroupAggregate's index scan (verified: Index Cond on uq_kb_properties_active
-- includes owner_id), so single-resource reads stay index probes.
--
-- `seq` stays text — the one consumer that needs an integer (`resource_row`) parses it
-- Rust-side with a typed error; `filtered_visible_page`'s ORDER BY casts ::bigint.
CREATE VIEW kb_resource_workflow_props AS
SELECT owner_id AS resource_id,
       MAX(property_value #>> '{}') FILTER (WHERE property_key = 'temper-stage')  AS stage,
       MAX(property_value #>> '{}') FILTER (WHERE property_key = 'temper-mode')   AS mode,
       MAX(property_value #>> '{}') FILTER (WHERE property_key = 'temper-effort') AS effort,
       MAX(property_value #>> '{}') FILTER (WHERE property_key = 'temper-seq')    AS seq
  FROM kb_properties
 WHERE owner_table = 'kb_resources'
   AND NOT is_folded
   AND property_key IN ('temper-stage', 'temper-mode', 'temper-effort', 'temper-seq')
 GROUP BY owner_id;
