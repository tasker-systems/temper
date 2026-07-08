-- Migration: index pack for the append-only spine tables.
--
-- SQL function audit 2026-07-08 (docs/code-reviews/2026-07-08-sql-function-audit.md,
-- SQLA-1 findings: missing-owner-home-index, event-payload-scan, missing-anchor-index,
-- missing-status-index). CREATE INDEX only — no function bodies change, no .sqlx impact.
--
-- 1. kb_resource_homes owner/originator — resources_visible_to (the hottest access
--    function) opens with `WHERE h.owner_profile_id = p OR h.originator_profile_id = p`,
--    previously a full scan of homes (one row per resource). The OR resolves to a
--    BitmapOr over the two btrees.
CREATE INDEX idx_kb_resource_homes_owner_profile
    ON kb_resource_homes (owner_profile_id);
CREATE INDEX idx_kb_resource_homes_originator_profile
    ON kb_resource_homes (originator_profile_id);

-- 2. kb_events payload expression indexes — element_trail_edge / element_trail_node
--    probe the unbounded event log by payload key ((payload->>'…')::uuid), previously a
--    seq scan per trail read. One expression index per probed key, matching the exact
--    function expressions (block_id serves element_trail_node's third leg, same shape).
--    These are the tactical fix; migrating trails to the indexed `references` column is
--    a possible future refactor, deliberately out of scope here.
--    Note: the ::uuid casts are evaluated for every existing row at build time — payloads
--    are produced exclusively by the substrate's own functions, so these keys are always
--    uuid-or-absent (absent → NULL, indexed as NULL and never matched).
CREATE INDEX idx_kb_events_payload_edge_id
    ON kb_events (((payload ->> 'edge_id')::uuid));
CREATE INDEX idx_kb_events_payload_resource_id
    ON kb_events (((payload ->> 'resource_id')::uuid));
CREATE INDEX idx_kb_events_payload_owner_id
    ON kb_events (((payload -> 'owner' ->> 'id')::uuid));
CREATE INDEX idx_kb_events_payload_block_id
    ON kb_events (((payload ->> 'block_id')::uuid));

-- 3. kb_events producing anchor — steward_ingest_delta counts events per cogmap via
--    `producing_anchor_table = 'kb_contexts' AND producing_anchor_id IN (…) AND id > watermark`;
--    the trailing `id` column serves the watermark range without a second sort/filter pass.
CREATE INDEX idx_kb_events_producing_anchor
    ON kb_events (producing_anchor_table, producing_anchor_id, id);

-- 4. kb_workflow_jobs dead-status partial — workflow_job_redrive_resource selects
--    `DISTINCT resource_id … WHERE persona=… AND dispatch_type=… AND status='dead'
--    ORDER BY resource_id LIMIT n`; the existing partials only cover live statuses.
CREATE INDEX idx_workflow_jobs_dead_resource
    ON kb_workflow_jobs (persona, dispatch_type, resource_id)
    WHERE status = 'dead';
