-- W2 PR 1 (#420 set 3): `ingest_state` — the missing projection.
--
-- `resource_finalize()` validates the landed block set and then records a PROJECTION-LESS
-- `resource_finalized` event. Nothing is materialized. So "is this document complete?" is answerable
-- only by scanning `kb_events` — which is exactly why a killed segmented upload stays listed,
-- searchable, and `status: ok` while holding 93% of its content. A silently-truncated document in a
-- knowledge base is worse than a failed ingest.
--
-- This migration projects the event, and gives a segmented ingest an honest birth state.
--
-- ── Why an additive PAYLOAD KEY and not a new `ingest_begun` event ──────────────────────────────────
-- A new event type would be the tidier ledger (two bookends: begun → finalized). It is also a WRITE
-- OUTAGE in the new-app/old-DB skew direction: `_event_append` cannot resolve an event type whose
-- `kb_event_types` row has not been migrated yet, so every segmented begin would 500 until the operator
-- ran the migration. `main` auto-deploys. So the flag rides an additive `segmented` key on the existing
-- `resource_created` payload instead: an old app omits the key and `coalesce(…, false)` reads it as
-- `complete` — exactly today's behavior — and an old projector ignores a key it does not know about.
-- Additive in both directions, by construction.
--
-- ── Why NO backfill ─────────────────────────────────────────────────────────────────────────────────
-- The obvious heuristic — "more than one live block AND no resource_finalized event ⇒ an incomplete
-- upload" — matches exactly FOUR production resources, and all four are telos charters, including the
-- L0 kernel "What Temper Is" (12 blocks). `charter_set` projects a multi-block role-tagged set and never
-- fires `resource_finalized`, because it is not a segmented ingest. A backfill on that heuristic would
-- have hidden every cognitive map's charter from list and search.
--
--   MULTI-BLOCK DOES NOT MEAN SEGMENTED. `charter_set` is the counter-example.
--
-- Verified against prod 2026-07-14: of 2,223 active resources, 2,219 are single-block; the only
-- multi-block rows are those 4 charters (8, 8, 9, 12 blocks). There is no historical signal that
-- reliably identifies an abandoned pre-existing upload. So: backfill NOTHING. Every existing row keeps
-- the `complete` default, which is true for all of them. Only NEW segmented begins are born
-- `in_progress`.

-- ── The column ──────────────────────────────────────────────────────────────────────────────────────
-- ADD COLUMN … NOT NULL DEFAULT is catalog-only on PG11+ — no table rewrite on PG17 (Neon prod) or
-- PG18 (local/CI).
ALTER TABLE kb_resources
    ADD COLUMN ingest_state text NOT NULL DEFAULT 'complete';

ALTER TABLE kb_resources
    ADD CONSTRAINT ck_kb_resources_ingest_state
        CHECK (ingest_state IN ('in_progress', 'complete'));

-- Partial index: the incomplete set is tiny and transient, and this is how a caller enumerates
-- resumable uploads without scanning the corpus. It anchors on `id` because ownership does NOT live on
-- kb_resources — it lives on kb_resource_homes.owner_profile_id, so "my partials" is this index joined
-- to homes. (It carries no weight for the `= 'complete'` predicate in list/search: that is the common
-- case and a seq/existing-index path is correct there.)
CREATE INDEX idx_kb_resources_incomplete
    ON kb_resources (id) WHERE ingest_state = 'in_progress';

-- ── The projection the finalize event never had ─────────────────────────────────────────────────────
CREATE FUNCTION _project_resource_finalized(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    UPDATE kb_resources SET ingest_state = 'complete'
     WHERE id = (p_payload->>'resource_id')::uuid;
END;
$$;

-- ── resource_finalize: project the event it already records ─────────────────────────────────────────
-- Body copied VERBATIM from the live definition (20260708000012_streaming_ingest.sql), with ONE added
-- call. The 4-arg signature is UNCHANGED — CREATE OR REPLACE cannot add a parameter (it would mint a
-- second overload and make old-arity calls ambiguous), and a signature change is a write outage across
-- deploy skew.
--
-- The validation already here is load-bearing for the guarantee: a mismatched block count or body_hash
-- RAISEs, the transaction rolls back, `_project_resource_finalized` never runs, and the resource stays
-- `in_progress` — still resumable, never silently done.
CREATE OR REPLACE FUNCTION resource_finalize(p_payload jsonb, p_emitter uuid,
                                             p_metadata jsonb DEFAULT '{}', p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_resource uuid := (p_payload->>'resource_id')::uuid;
    v_expected_blocks int := (p_payload->>'expected_blocks')::int;
    v_expected_hash text := p_payload->>'expected_body_hash';
    v_actual_blocks int;
    v_actual_hash text;
    v_anchor_tbl text; v_anchor uuid;
    v_ev uuid;
BEGIN
    SELECT count(*) INTO v_actual_blocks FROM kb_content_blocks
        WHERE resource_id = v_resource AND NOT is_folded;
    IF v_actual_blocks <> v_expected_blocks THEN
        RAISE EXCEPTION 'resource_finalize: resource % has % live blocks, expected %',
            v_resource, v_actual_blocks, v_expected_blocks;
    END IF;
    SELECT body_hash INTO v_actual_hash FROM kb_resources WHERE id = v_resource;
    IF v_actual_hash IS DISTINCT FROM v_expected_hash THEN
        RAISE EXCEPTION 'resource_finalize: resource % body_hash % does not match expected %',
            v_resource, v_actual_hash, v_expected_hash;
    END IF;
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table = 'kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'resource_finalize: resource % has no home', v_resource;
    END IF;
    v_ev := _event_append('resource_finalized', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    PERFORM _project_resource_finalized(v_ev, p_payload);   -- ← NEW: the event is no longer projection-less
    RETURN v_ev;
END;
$$;

-- ── _project_resource_created: an honest birth state ────────────────────────────────────────────────
-- Body copied VERBATIM from the live definition (20260624000002_canonical_functions.sql), with ONE edit:
-- the kb_resources INSERT now carries `ingest_state`, read from the payload.
--
-- `coalesce((p_payload->>'segmented')::boolean, false)` is the skew hinge. An old app's payload has no
-- `segmented` key ⇒ false ⇒ `complete`, exactly as today. Only `begin_segmented_ingest` sets it.
CREATE OR REPLACE FUNCTION _project_resource_created(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_owner    uuid := (p_payload->>'owner_profile_id')::uuid;
BEGIN
    INSERT INTO kb_resources (id, title, origin_uri, created, updated, ingest_state)
        VALUES (v_resource, p_payload->>'title', p_payload->>'origin_uri', v_occurred, v_occurred,
                CASE WHEN coalesce((p_payload->>'segmented')::boolean, false)   -- ← NEW
                     THEN 'in_progress' ELSE 'complete' END);
    INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id,
                                   originator_profile_id, owner_profile_id, created)
        VALUES (v_resource, p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid,
                COALESCE((p_payload->>'originator_profile_id')::uuid, v_owner),
                v_owner, v_occurred);
    PERFORM _project_blocks(v_resource, p_event, p_payload->'blocks', p_content);
    IF p_payload->>'doc_type' IS NOT NULL THEN
        INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value,
                                   asserted_by_event_id, last_event_id, created)
            VALUES ('kb_resources', v_resource, 'doc_type', p_payload->'doc_type',
                    p_event, p_event, v_occurred);
    END IF;
    RETURN v_resource;
END;
$$;

-- ── Search excludes partials ────────────────────────────────────────────────────────────────────────
-- THE RULE: `ingest_state = 'complete'` goes exactly where `r.is_active` already goes. Same semantics
-- (a row that exists but must not surface), same placement, same trade-offs. Nothing else moves.
--
-- Both bodies below are copied VERBATIM from their live definitions
-- (20260711000050_search_vector_scope_aware.sql). Re-deriving these by hand is how you silently delete
-- the search index; the ONLY edits are the added predicates, marked ← NEW.

-- search_fts_candidates: the lexical arm. Body copied VERBATIM from its live definition
-- (20260626000002_search_beat2_surface_a.sql); the ONLY edit is the added predicate.
--
-- The `corpus` CTE below would already keep a partial out of the RESULTS, so this is not needed for
-- visibility. It is needed because `blend0` (fts ∪ vec) feeds `seeds`, and `seeds` anchors the graph
-- expansion: an unfinalized partial left in the lexical arm can occupy auto-seed slots and lend
-- graph_score to its neighbours while never surfacing itself. A document that is not here yet should
-- not be shaping the ranking of documents that are.
--
-- Note the asymmetry with the vector arm, which is deliberate: this function has NO top-k, so it
-- carries no starvation risk and the predicate can sit plainly in the WHERE.
CREATE OR REPLACE FUNCTION search_fts_candidates(p_principal uuid, p_query text)
RETURNS TABLE (resource_id uuid, fts_norm real)
LANGUAGE sql STABLE AS $$
  SELECT r.id,
         (ts_rank(si.search_vector, plainto_tsquery('english', p_query), 32))::real
    FROM kb_resource_search_index si
    JOIN kb_resources r                       ON r.id = si.resource_id
    JOIN resources_visible_to(p_principal) v   ON v.resource_id = r.id
   WHERE p_query IS NOT NULL AND p_query <> ''
     AND r.is_active
     AND r.ingest_state = 'complete'                                        -- ← NEW
     AND si.search_vector @@ plainto_tsquery('english', p_query);
$$;

-- search_vector_candidates: the ANTI-STARVATION beat. The `corpus` CTE below is already a sufficient
-- correctness gate (everything scored passes through it), but without this an `in_progress` resource's
-- chunks can still occupy slots in the global top-k ANN and crowd complete ones out of the candidate
-- set — the exact starvation class issue #358 fixed for scope. Note that in the UNSCOPED branch the
-- predicate lands AFTER `LIMIT p_k`, not inside the `ann` CTE: applying it inside would force a
-- seq-scan and defeat idx_kb_chunks_embedding. That is precisely how `is_active` is handled here today.
CREATE OR REPLACE FUNCTION search_vector_candidates(
  p_principal   uuid,
  p_emb         vector,
  p_k           int,
  p_context_id  uuid   DEFAULT NULL,
  p_scope_ids   uuid[] DEFAULT NULL)
RETURNS TABLE (resource_id uuid, vec_norm real)
LANGUAGE plpgsql STABLE AS $$
BEGIN
  IF p_context_id IS NULL AND p_scope_ids IS NULL THEN
    RETURN QUERY
      WITH ann AS (
        SELECT c.resource_id, (c.embedding <=> p_emb) AS dist
          FROM kb_chunks c
         WHERE p_emb IS NOT NULL AND c.is_current
         ORDER BY c.embedding <=> p_emb
         LIMIT p_k
      )
      SELECT a.resource_id, (1.0 - MIN(a.dist) / 2.0)::real
        FROM ann a
        JOIN kb_resources r                      ON r.id = a.resource_id AND r.is_active
                                                AND r.ingest_state = 'complete'   -- ← NEW
        JOIN resources_visible_to(p_principal) v ON v.resource_id = a.resource_id
       GROUP BY a.resource_id;
  ELSE
    RETURN QUERY
      WITH scoped_res AS (
        SELECT v.resource_id AS id
          FROM resources_visible_to(p_principal) v
          JOIN kb_resources r ON r.id = v.resource_id AND r.is_active
                             AND r.ingest_state = 'complete'                       -- ← NEW
         WHERE (p_context_id IS NULL OR EXISTS (
                 SELECT 1 FROM kb_resource_homes h
                  WHERE h.resource_id = v.resource_id
                    AND h.anchor_table = 'kb_contexts' AND h.anchor_id = p_context_id))
           AND (p_scope_ids IS NULL OR v.resource_id = ANY(p_scope_ids))
      ),
      ann AS (
        SELECT c.resource_id, (c.embedding <=> p_emb) AS dist
          FROM kb_chunks c
          JOIN scoped_res s ON s.id = c.resource_id
         WHERE p_emb IS NOT NULL AND c.is_current
      )
      SELECT a.resource_id, (1.0 - MIN(a.dist) / 2.0)::real
        FROM ann a
       GROUP BY a.resource_id;
  END IF;
END;
$$;

-- unified_search: the `corpus` CTE is the SUFFICIENT gate — every candidate from every arm (fts, vec,
-- graph, seed) is funnelled through it before scoring, so one predicate here covers them all.
-- Identical 13-arg signature (no DROP, no deploy-skew absence window).
CREATE OR REPLACE FUNCTION unified_search(
  p_principal uuid, p_query text, p_emb vector, p_seed_ids uuid[], p_depth int,
  p_edge_types text[], p_context_id uuid, p_doc_type text, p_graph_expand boolean,
  p_limit int, p_offset int, p_scope_ids uuid[], p_seed_only boolean DEFAULT false)
RETURNS TABLE (resource_id uuid, fts_score real, vector_score real, graph_score real, combined_score real)
LANGUAGE sql STABLE AS $$
  WITH
  k AS (SELECT 1.0::float8 AS w_fts, 1.0::float8 AS w_vec, 0.5::float8 AS w_graph,
               0.5::float8 AS gamma, 100 AS vector_k, 20 AS auto_seed_n),
  fts AS (SELECT * FROM search_fts_candidates(p_principal, p_query)),
  vec AS (SELECT * FROM search_vector_candidates(
            p_principal, p_emb, (SELECT vector_k FROM k), p_context_id, p_scope_ids)),
  blend0 AS (
    SELECT COALESCE(f.resource_id, v.resource_id) AS id,
           (SELECT w_fts FROM k) * COALESCE(f.fts_norm, 0)
         + (SELECT w_vec FROM k) * COALESCE(v.vec_norm, 0) AS s0
      FROM fts f FULL OUTER JOIN vec v ON f.resource_id = v.resource_id
  ),
  seeds AS (
    SELECT unnest(COALESCE(p_seed_ids, ARRAY[]::uuid[])) AS id
    UNION
    SELECT id FROM (SELECT id, s0 FROM blend0 ORDER BY s0 DESC LIMIT (SELECT auto_seed_n FROM k)) t
     WHERE NOT (COALESCE(p_seed_only, false) AND COALESCE(array_length(p_seed_ids, 1), 0) > 0)
  ),
  graph AS (
    SELECT * FROM search_graph_expand(
      p_principal,
      CASE WHEN p_graph_expand THEN ARRAY(SELECT id FROM seeds) ELSE ARRAY[]::uuid[] END,
      p_depth, p_edge_types, (SELECT gamma FROM k))
  ),
  seed_cand AS (
    SELECT s.id
      FROM unnest(COALESCE(p_seed_ids, ARRAY[]::uuid[])) AS s(id)
      JOIN resources_visible_to(p_principal) v ON v.resource_id = s.id
     WHERE p_graph_expand
  ),
  cand AS (
    SELECT id FROM blend0
    UNION SELECT resource_id FROM graph
    UNION SELECT id FROM seed_cand
  ),
  corpus AS (   -- context/doc_type/scope candidate-corpus filter + the completeness gate
    SELECT c.id FROM cand c
     WHERE (p_context_id IS NULL OR EXISTS (
             SELECT 1 FROM kb_resource_homes h
              WHERE h.resource_id = c.id AND h.anchor_table = 'kb_contexts' AND h.anchor_id = p_context_id))
       AND (p_scope_ids IS NULL OR c.id = ANY(p_scope_ids))
       AND (p_doc_type IS NULL OR EXISTS (
             SELECT 1 FROM kb_properties p
              WHERE p.owner_table = 'kb_resources' AND p.owner_id = c.id
                AND p.property_key = 'doc_type' AND NOT p.is_folded
                AND p.property_value #>> '{}' = p_doc_type))
       -- An interrupted ingest is not a document. It stays addressable via `show`; it never
       -- surfaces in search as if it were whole.                                        ← NEW
       AND EXISTS (
             SELECT 1 FROM kb_resources rr
              WHERE rr.id = c.id AND rr.ingest_state = 'complete')
  ),
  scored AS (
    SELECT co.id,
           COALESCE(f.fts_norm, 0)::real    AS fts_score,
           COALESCE(v.vec_norm, 0)::real    AS vector_score,
           COALESCE(g.graph_score, 0)::real AS graph_score,
           ((SELECT w_fts FROM k)   * COALESCE(f.fts_norm, 0)
          + (SELECT w_vec FROM k)   * COALESCE(v.vec_norm, 0)
          + (SELECT w_graph FROM k) * COALESCE(g.graph_score, 0))::real AS combined_score
      FROM corpus co
      LEFT JOIN fts f   ON f.resource_id = co.id
      LEFT JOIN vec v   ON v.resource_id = co.id
      LEFT JOIN graph g ON g.resource_id = co.id
  )
  SELECT id, fts_score, vector_score, graph_score, combined_score
    FROM scored
   ORDER BY combined_score DESC, id
   LIMIT p_limit OFFSET p_offset;
$$;
