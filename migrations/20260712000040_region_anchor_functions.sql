-- Producer-side SQL for the anchor-generalized region tier (spec §3.6 M2, §3.9).
--
-- M1 (20260712000030) added the (home_anchor_table, home_anchor_id) pair to the region tables. This
-- migration is the companion to the code that starts WRITING it: the producer now folds, creates and
-- asserts by ANCHOR rather than by cogmap_id, so the event-side functions must anchor the same way.

-- ---------------------------------------------------------------------------
-- 0. CATCH-UP BACKFILL. Not optional, and it must run BEFORE anything else here.
--
-- T2's backfill anchored every row that existed AT MIGRATION TIME, and it has already run in prod.
-- But the producer only starts dual-writing the anchor pair in THIS migration's companion code — so
-- every region and component materialized in the T2 -> T3 window landed with home_anchor_id IS NULL,
-- and T2's one-shot backfill can never catch them.
--
-- That is not cosmetic. The new fold_live_regions / fold_live_components find live rows BY ANCHOR. A
-- NULL-anchor region does not match that predicate, so it is never folded — it survives as a LIVE
-- region alongside the freshly-created ones. Result: duplicate live regions, inflated member counts,
-- and stale rows in every region read.
--
-- Cheap insurance whether or not any rows have actually accumulated.
UPDATE kb_cogmap_regions    SET home_anchor_table = 'kb_cogmaps', home_anchor_id = cogmap_id
    WHERE home_anchor_id IS NULL AND cogmap_id IS NOT NULL;
UPDATE kb_cogmap_components SET home_anchor_table = 'kb_cogmaps', home_anchor_id = cogmap_id
    WHERE home_anchor_id IS NULL AND cogmap_id IS NOT NULL;

-- ---------------------------------------------------------------------------
-- 1. BUG FIX (spec §3.9.2): cogmap_region_centrality counted edges from OUTSIDE the map.
--
-- The function sums kb_edges.weight between member pairs with NO home_anchor filter, so any edge
-- asserted elsewhere between two of this region's members already inflated its centrality — and
-- through centrality, its salience. Under a polymorphic anchor it would additionally MIX context and
-- cogmap edges into one region's score.
--
-- The fix restricts the edge set to edges homed in the region's OWN anchor. Everything else about the
-- body is unchanged (this is the live body, printed with \sf — not a reconstruction).
CREATE OR REPLACE FUNCTION cogmap_region_centrality(p_region uuid)
RETURNS double precision
LANGUAGE sql STABLE AS $$
    WITH reg AS (
        SELECT home_anchor_table, home_anchor_id FROM kb_cogmap_regions WHERE id = p_region
    ),
    mem AS (
        SELECT member_id FROM kb_cogmap_region_members
        WHERE region_id = p_region AND member_table = 'kb_resources'
    ),
    internal AS (
        SELECT coalesce(sum(e.weight), 0) AS mass
        FROM kb_edges e, reg
        WHERE e.source_table='kb_resources' AND e.target_table='kb_resources'
          AND e.source_id IN (SELECT member_id FROM mem)
          AND e.target_id IN (SELECT member_id FROM mem)
          AND NOT e.is_folded
          -- THE FIX: the edge must be homed in the SAME anchor as the region.
          AND e.home_anchor_table = reg.home_anchor_table
          AND e.home_anchor_id    = reg.home_anchor_id
    )
    SELECT internal.mass * (SELECT count(*) FROM mem) FROM internal;
$$;

COMMENT ON FUNCTION cogmap_region_centrality(uuid) IS
    'Internal declared-affinity density x size. Edges are home-filtered to the region''s own anchor '
    '(added 2026-07-12, spec 3.9.2) — previously UNFILTERED, which counted edges asserted outside the '
    'map and inflated salience.';

-- ---------------------------------------------------------------------------
-- 2. Anchor the materialization event and its projection.
--
-- Both functions READ THE ANCHOR TWO WAYS, and that is deliberate. kb_events is append-only: every
-- region_materialized event written before this migration carries `cogmap_id` and no anchor pair, and
-- those rows are immortal. replay() re-projects historical events through _project_region_materialized
-- (replay.rs), so a function that only understood the new key would RAISE on every pre-T3 act and
-- break ledger replay outright. Falling back to cogmap_id costs nothing and keeps the old acts
-- readable forever. The Rust payload dual-writes both keys for the same reason.
CREATE OR REPLACE FUNCTION _project_region_materialized(p_event uuid, p_payload jsonb)
RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    v_table text := coalesce(p_payload->>'home_anchor_table', 'kb_cogmaps');
    v_id    uuid := coalesce((p_payload->>'home_anchor_id')::uuid, (p_payload->>'cogmap_id')::uuid);
BEGIN
    IF v_table = 'kb_cogmaps' THEN
        UPDATE kb_cogmaps  SET shape_materialized_event_id = p_event WHERE id = v_id;
    ELSIF v_table = 'kb_contexts' THEN
        UPDATE kb_contexts SET shape_materialized_event_id = p_event WHERE id = v_id;
    ELSE
        RAISE EXCEPTION 'region_materialized: unknown home_anchor_table %', v_table;
    END IF;
END;
$$;

CREATE OR REPLACE FUNCTION region_materialize(p_payload jsonb, p_emitter uuid)
RETURNS uuid
LANGUAGE plpgsql
AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('region_materialized', p_emitter,
                          coalesce(p_payload->>'home_anchor_table', 'kb_cogmaps'),
                          coalesce((p_payload->>'home_anchor_id')::uuid,
                                   (p_payload->>'cogmap_id')::uuid),
                          p_payload);
    PERFORM _project_region_materialized(v_ev, p_payload);
    RETURN v_ev;
END;
$$;

-- ---------------------------------------------------------------------------
-- 3. The published payload contract.
--
-- Carried forward verbatim from the schemars-generated snapshot
-- (crates/temper-substrate/tests/fixtures/payloads/region_materialized.v1.schema.json) — the boot-seed
-- stamps that file into kb_event_types for fresh databases; this UPDATE is what keeps an ALREADY-SEEDED
-- database (prod) honest. Repo == registry == Rust types.
--
-- `home_anchor_table` + `home_anchor_id` are now REQUIRED; `cogmap_id` stays as an OPTIONAL property,
-- dual-written for cogmap acts. Note _event_append does NOT validate payloads against this schema — it
-- is a published contract, not an enforced one — so this is a documentation fix, not a gate. Old events
-- carrying only cogmap_id remain valid and readable.
UPDATE kb_event_types SET payload_schema = $schema${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "RegionMaterialized",
  "type": "object",
  "properties": {
    "home_anchor_table": {
      "description": "The anchor the regions were formed over — a context OR a cognitive map (spec §3.6 M2).\nSupersedes `cogmap_id`.",
      "$ref": "#/$defs/AnchorTable"
    },
    "home_anchor_id": {
      "type": "string",
      "format": "uuid"
    },
    "cogmap_id": {
      "description": "VESTIGIAL, dual-written through the expand window. `kb_events` is APPEND-ONLY: every\n`region_materialized` event written before T3 carries this key and no anchor pair, and those\nrows are immortal. Keeping it written (and OPTIONAL, so a context act can omit it) is what lets\nthe ledger probe in `replay::last_materialize_event` read old and new acts with one query.\n`None` for a context anchor. Do not read this in new code.",
      "anyOf": [
        {
          "$ref": "#/$defs/CogmapId"
        },
        {
          "type": "null"
        }
      ]
    },
    "lens_id": {
      "$ref": "#/$defs/LensId"
    },
    "watermark_event_id": {
      "description": "Max event id over the substrate at load time — the point-in-time the projection saw.",
      "$ref": "#/$defs/EventId"
    },
    "membership_fingerprint": {
      "description": "The per-lens membership signature (sorted member-uuid join). Doubles as the drift-detection\ndecision's persisted fingerprint artifact.",
      "type": "string"
    },
    "region_ids": {
      "type": "array",
      "items": {
        "$ref": "#/$defs/RegionId"
      }
    }
  },
  "required": [
    "home_anchor_table",
    "home_anchor_id",
    "lens_id",
    "watermark_event_id",
    "membership_fingerprint",
    "region_ids"
  ],
  "$defs": {
    "AnchorTable": {
      "description": "A polymorphic anchor/endpoint reference. Serializes table names exactly as the DDL spells them.",
      "type": "string",
      "enum": [
        "kb_contexts",
        "kb_cogmaps",
        "kb_resources",
        "kb_edges",
        "kb_content_blocks",
        "kb_teams",
        "kb_profiles"
      ]
    },
    "CogmapId": {
      "description": "A `kb_cogmaps.id` value — a cognitive map.",
      "type": "string",
      "format": "uuid"
    },
    "LensId": {
      "description": "A `kb_cogmap_lenses.id` value.",
      "type": "string",
      "format": "uuid"
    },
    "EventId": {
      "description": "A `kb_events.id` value. Always UUIDv7 (time-sortable).",
      "type": "string",
      "format": "uuid"
    },
    "RegionId": {
      "description": "A `kb_cogmap_regions.id` value — one materialized region.",
      "type": "string",
      "format": "uuid"
    }
  }
}$schema$::jsonb
WHERE name = 'region_materialized';

-- ---------------------------------------------------------------------------
-- 4. The member-affinity column finally has a writer (spec §3.9.1).
--
-- kb_cogmap_region_members.affinity has existed since the region tier shipped and was NEVER written,
-- yet four readers order by it (graph_region_members, graph_region_territories,
-- graph_cogmap_territories, atlas_search — all `ORDER BY m.affinity DESC NULLS LAST`). Every "top
-- member" and every derived region label in the product has therefore been arbitrary. write.rs now
-- persists it as the member's average-link affinity to the rest of its component. No DDL needed —
-- the column was always there. This COMMENT is the definition it never had.
COMMENT ON COLUMN kb_cogmap_region_members.affinity IS
    'How CORE this member is to its region: its average-link affinity to the region''s other members '
    '(same linkage the clustering uses, so the score is coherent with why the member landed here). '
    'A singleton region scores 0.0. Written by the region producer as of 2026-07-12 — before that it '
    'was NULL on every row, which made the four `ORDER BY affinity DESC` readers arbitrary.';
