-- Migration: index selected open_meta fields into the FTS vector (issue #359).
--
-- Importers enrich resources with open_meta (section descriptors, keyword lists), but none of it was
-- searchable: open_meta lands in kb_properties (one row per key, property_key = the open_meta key) and
-- the only search consumer was the doc_type filter. Two observed consequences on a chunked corpus:
--   1. Chunk titles get truncated by importers, with the full section descriptor stored in open_meta
--      where search can't see it — the searchable title loses exactly the discriminating words.
--   2. Deliberately-attached keywords provided zero ranking boost, contrary to the user's mental model.
--
-- Fix (the FTS-vector-indexing route, NOT a unified_search score term — keeps #359 independent of the
-- unified_search DROP+CREATE churn in #357/#358): fold selected open_meta keys into the stored
-- kb_resource_search_index vector at *supplementary* weights inside _rebuild_resource_search_vector, so
-- ts_rank picks up the extra lexemes for free with no query-side change.
--
-- ── open_meta search-indexing convention v1 ──────────────────────────────────────────────────────
--   keywords    → weight C   a JSON array of strings (space-joined), or a bare JSON string.
--   descriptor  → weight D   a JSON string (the full section descriptor truncated out of the title).
-- Both sit below title(A)/body(B): importer metadata breaks ties and adds matches the title/body
-- missed, without overpowering genuine primary-content matches. The set is versioned here (the
-- migration IS the schema version); adding a field is a new additive migration bumping the version.
-- See docs/search-open-meta-indexing.md.
--
-- Three beats (all additive: CREATE OR REPLACE, same signatures + a targeted backfill):
--   1. Extend _rebuild_resource_search_vector to append keywords@C + descriptor@D.
--   2. Rebuild on property_set for an indexed key — at create time the block projection rebuilds the
--      vector (title+body) BEFORE the property_set events fire, so this hook is what gets keywords/
--      descriptor into the index, for both create and runtime open_meta updates.
--   3. Backfill: re-run the rebuild for every active resource that already carries one of these keys.

-- ── Beat 1: keywords@C + descriptor@D on top of the live-block title(A)/body(B) vector ───────────
CREATE OR REPLACE FUNCTION _rebuild_resource_search_vector(p_resource uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_config varchar(64); v_title text; v_body text;
        v_keywords text; v_descriptor text;
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
              || setweight(to_tsvector(v_config::regconfig, COALESCE(v_descriptor,'')), 'D'),
            v_config, now())
    ON CONFLICT (resource_id) DO UPDATE
        SET search_vector = EXCLUDED.search_vector, updated = now();
END;
$$;

-- ── Beat 2: re-fold the vector when an indexed open_meta key changes ──────────────────────────────
-- Base definition: migrations/20260624000002_canonical_functions.sql:1136 (fold-then-insert). The only
-- addition is the gated rebuild at the tail. Gated on the exact indexed key set so unrelated
-- property_sets (provenance, doc_type, clustering facets) do not pay a rebuild.
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
    IF v_owner_tbl = 'kb_resources' AND v_key IN ('keywords', 'descriptor') THEN
        PERFORM _rebuild_resource_search_vector(v_owner);
    END IF;
    RETURN v_prop;
END;
$$;

-- ── Beat 3: backfill the affected population ─────────────────────────────────────────────────────
-- Only resources carrying an indexed key can have a changed vector; rebuild exactly those.
DO $$
DECLARE r uuid;
BEGIN
    FOR r IN
        SELECT DISTINCT p.owner_id
          FROM kb_properties p
          JOIN kb_resources res ON res.id = p.owner_id AND res.is_active
         WHERE p.owner_table = 'kb_resources'
           AND p.property_key IN ('keywords', 'descriptor')
           AND NOT p.is_folded
    LOOP
        PERFORM _rebuild_resource_search_vector(r);
    END LOOP;
END;
$$;
