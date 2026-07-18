-- The element-trail admin firewall, made RUNTIME rather than conventional (plan item 5b.5).
--
-- `element_trail_node`/`element_trail_edge` match events by payload KEY SHAPE — `resource_id`,
-- `owner.{table,id}`, `block_id`, `edge_id` — with no event-type filter, and are gated only by
-- `resources_visible_to` / `anchor_readable_by_profile`. Both are far weaker gates than the admin
-- ledger's per-act dispatch (`admin_ledger_service::readable_event_types`). So an admin event whose
-- payload happened to spell one of those keys would leak an authority record to any reader of the
-- subject.
--
-- Until now the invariant held *only* because admin payloads spell `subject_table`/`subject_id`
-- instead (20260717000010, guarded by `no_admin_payload_spells_a_trail_matched_key`). That is a
-- naming convention with a test behind it, not a filter. This migration replaces it with two
-- independent runtime filters, either of which alone is sufficient:
--
--   A. ANCHOR NULLITY (the primary, semantically-exact signal). Admin events are born with a
--      both-NULL `producing_anchor` — the cognition firewall (spec 2026-07-16 §4). "No cognition
--      home" is precisely the element trail's inclusion criterion inverted, so a both-NULL event
--      has no business in a trail regardless of what its payload spells. This matches the shape
--      every other anchor-scoped reader already uses (`event_service::latest_event_id_for_context`
--      scopes `producing_anchor_table = 'kb_contexts'`; the region producers and
--      `steward_ingest_delta` do the same) — the trail functions were the outlier.
--
--      Verified against the full prod corpus 2026-07-18 and reproduced on local dev: the ONLY
--      both-NULL-anchored types are `lens_created` (3 rows) and `admin_ledger_opened` (1 row), and
--      neither spells a trail-matched key. This filter therefore drops zero existing trail entries.
--
--   B. REGISTRY CLASSIFICATION (the visible belt-and-braces). Nullity is exact but invisible at the
--      call site — a reviewer reading `element_trail_node` cannot tell that "both-NULL" means
--      "admin". `kb_event_types.category` says it in words, and the trail's `et.category =
--      'cognition'` reads as what it is: a cognition trail returns cognition events.
--
-- Why `category text` and not `is_admin boolean`: the axis is not binary. `lens_created` is
-- both-NULL-anchored and is NOT admin — it is system configuration — so a boolean would force it
-- into a bucket that misdescribes it the moment anyone wants to classify it. A CHECK-bounded text
-- category leaves room for that third value without another migration, and matches how this schema
-- already spells bounded sets (`kb_events_producing_anchor_table_check`).
--
-- Why the trail filter is an ALLOWLIST (`= 'cognition'`) and not a denylist (`<> 'admin'`): a
-- future category added without touching this function must be EXCLUDED from cognition reads by
-- default, not included. The permissive direction here is the leaking direction.
--
-- The DEFAULT is 'cognition' because the column must be additive over an existing registry — so a
-- newly-registered admin type that nobody stamps still rides the nullity filter (A). That is the
-- whole reason both halves ship: neither default is safe alone.

ALTER TABLE kb_event_types
  ADD COLUMN category text NOT NULL DEFAULT 'cognition';

ALTER TABLE kb_event_types
  ADD CONSTRAINT kb_event_types_category_check
  CHECK (category IN ('cognition', 'admin'));

-- Stamp the three admin types. UPDATE-by-name, never INSERT: `kb_event_types` is mid-reconciliation
-- in prod (13 of 18 typed names carry a NULL payload_schema — cutover scar, task 019f7509), so
-- prod's row set does NOT match a fresh local migrate. An UPDATE is correct whether a row is
-- present or absent, and is idempotent on re-run.
UPDATE kb_event_types
   SET category = 'admin'
 WHERE name IN ('admin_ledger_opened', 'grant_created', 'grant_revoked');

-- Does NOT interact with `bootseed_publishes_payload_schemas`: that gate counts
-- `payload_schema IS NOT NULL` against `TYPED_EVENT_NAMES`, and this migration touches neither
-- `payload_schema` nor the row set. `replay`'s verbatim table round-trip
-- (`jsonb_populate_recordset(NULL::kb_event_types, …)`, replay.rs:329) carries the new column
-- unchanged, so ledger-replay equivalence is unaffected.

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
    WHERE et.category = 'cognition'
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
      AND et.category = 'cognition'
      AND anchor_readable_by_profile(p_profile, edg.home_anchor_table, edg.home_anchor_id)
      AND endpoint_readable_by_profile(p_profile, edg.source_table, edg.source_id)
      AND endpoint_readable_by_profile(p_profile, edg.target_table, edg.target_id)
    ORDER BY ev.id;
$$;
