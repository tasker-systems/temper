-- Migration: fold open_meta `tags` into the FTS vector at weight C (open_meta convention v1 → v2).
--
-- #359 (migration 20260711000040) indexed `keywords`@C + `descriptor`@D, but production evidence
-- (Neon kb_properties, 2026-07-11) showed the corpus overwhelmingly uses `tags` (54 resources), not
-- `keywords` (0). #359's ranking win was therefore unreachable by real data. This migration closes that
-- gap: `tags` is the everyday topical-tag key and ranks identically to `keywords` (both weight C).
--
-- ── open_meta search-indexing convention v2 ──────────────────────────────────────────────────────
--   keywords    → weight C   a JSON array of strings (space-joined), or a bare JSON string.   (v1)
--   descriptor  → weight D   a JSON string (the full section descriptor truncated out of title). (v1)
--   tags        → weight C   a JSON array of strings (space-joined). Synonymous with keywords.   (v2)
-- The set is versioned by migration (the migration IS the schema version); this additive migration
-- bumps v1 → v2. Do NOT edit 20260711000040 (checksum-locked). See docs/search-open-meta-indexing.md
-- and crates/temper-workflow/schemas/open_meta.schema.json.
--
-- Three beats (all additive: CREATE OR REPLACE, same signatures + a targeted backfill), mirroring
-- 20260711000040 exactly:
--   1. Extend _rebuild_resource_search_vector to also append tags@C.
--   2. Extend the _project_property_set rebuild gate to include 'tags'.
--   3. Backfill: re-run the rebuild for every active resource that already carries `tags`
--      (keywords/descriptor were already rebuilt by 20260711000040 — only `tags` is newly affected).

-- ── Beat 1: keywords@C + tags@C + descriptor@D on top of the live-block title(A)/body(B) vector ────
CREATE OR REPLACE FUNCTION _rebuild_resource_search_vector(p_resource uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_config varchar(64); v_title text; v_body text;
        v_keywords text; v_tags text; v_descriptor text;
BEGIN
    SELECT COALESCE((SELECT search_config FROM kb_resource_search_index WHERE resource_id = p_resource),
                    'english') INTO v_config;
    SELECT title INTO v_title FROM kb_resources WHERE id = p_resource;
    IF v_title IS NULL THEN RETURN; END IF;
    SELECT COALESCE(string_agg(cc.content, ' ' ORDER BY b.seq, c.chunk_index), '')
      INTO v_body
      FROM kb_chunks c
      JOIN kb_content_blocks b ON b.id = c.block_id AND NOT b.is_folded
      JOIN kb_chunk_content cc ON cc.chunk_id = c.id
     WHERE c.resource_id = p_resource AND c.is_current;

    -- open_meta convention v1: keywords (array of strings, space-joined, or a bare string) at weight C.
    SELECT COALESCE(
        (SELECT CASE jsonb_typeof(property_value)
                  WHEN 'array'  THEN (SELECT string_agg(e, ' ')
                                        FROM jsonb_array_elements_text(property_value) e)
                  WHEN 'string' THEN property_value #>> '{}'
                  ELSE '' END
           FROM kb_properties
          WHERE owner_table = 'kb_resources' AND owner_id = p_resource
            AND property_key = 'keywords' AND NOT is_folded
          LIMIT 1),
        '') INTO v_keywords;
    -- open_meta convention v2: tags (array of strings, space-joined, or a bare string) at weight C.
    SELECT COALESCE(
        (SELECT CASE jsonb_typeof(property_value)
                  WHEN 'array'  THEN (SELECT string_agg(e, ' ')
                                        FROM jsonb_array_elements_text(property_value) e)
                  WHEN 'string' THEN property_value #>> '{}'
                  ELSE '' END
           FROM kb_properties
          WHERE owner_table = 'kb_resources' AND owner_id = p_resource
            AND property_key = 'tags' AND NOT is_folded
          LIMIT 1),
        '') INTO v_tags;
    -- open_meta convention v1: descriptor (a JSON string) at weight D.
    SELECT COALESCE(
        (SELECT property_value #>> '{}'
           FROM kb_properties
          WHERE owner_table = 'kb_resources' AND owner_id = p_resource
            AND property_key = 'descriptor' AND NOT is_folded
            AND jsonb_typeof(property_value) = 'string'
          LIMIT 1),
        '') INTO v_descriptor;

    INSERT INTO kb_resource_search_index (resource_id, search_vector, search_config, updated)
    VALUES (p_resource,
            setweight(to_tsvector(v_config::regconfig, COALESCE(v_title,'')), 'A')
              || setweight(to_tsvector(v_config::regconfig, v_body), 'B')
              || setweight(to_tsvector(v_config::regconfig, COALESCE(v_keywords,'')), 'C')
              || setweight(to_tsvector(v_config::regconfig, COALESCE(v_tags,'')), 'C')
              || setweight(to_tsvector(v_config::regconfig, COALESCE(v_descriptor,'')), 'D'),
            v_config, now())
    ON CONFLICT (resource_id) DO UPDATE
        SET search_vector = EXCLUDED.search_vector, updated = now();
END;
$$;

-- ── Beat 2: re-fold the vector when an indexed open_meta key changes ──────────────────────────────
-- Base definition: migrations/20260624000002_canonical_functions.sql:1136 (fold-then-insert), extended
-- by 20260711000040. The only change here is adding 'tags' to the gated rebuild key set.
CREATE OR REPLACE FUNCTION _project_property_set(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_prop uuid := (p_payload->>'property_id')::uuid;
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_owner_tbl text := p_payload#>>'{owner,table}';
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
        v_key text := p_payload->>'property_key';
BEGIN
    UPDATE kb_properties SET is_folded = true, last_event_id = p_event
        WHERE owner_table = v_owner_tbl AND owner_id = v_owner
          AND property_key = v_key AND NOT is_folded;
    INSERT INTO kb_properties (id, owner_table, owner_id, property_key, property_value, weight,
                               asserted_by_event_id, last_event_id, created)
    VALUES (v_prop, v_owner_tbl, v_owner, v_key, p_payload->'value',
            (p_payload->>'weight')::double precision, p_event, p_event, v_occurred);
    IF v_owner_tbl = 'kb_resources' AND v_key IN ('keywords', 'descriptor', 'tags') THEN
        PERFORM _rebuild_resource_search_vector(v_owner);
    END IF;
    RETURN v_prop;
END;
$$;

-- ── Beat 3: backfill the newly-affected population ───────────────────────────────────────────────
-- Only resources carrying `tags` gain a changed vector under v2 (keywords/descriptor were rebuilt by
-- 20260711000040); rebuild exactly those.
DO $$
DECLARE r uuid;
BEGIN
    FOR r IN
        SELECT DISTINCT p.owner_id
          FROM kb_properties p
          JOIN kb_resources res ON res.id = p.owner_id AND res.is_active
         WHERE p.owner_table = 'kb_resources'
           AND p.property_key = 'tags'
           AND NOT p.is_folded
    LOOP
        PERFORM _rebuild_resource_search_vector(r);
    END LOOP;
END;
$$;
