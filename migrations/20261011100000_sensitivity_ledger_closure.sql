-- The sensitivity sweep: a ledger finding closes once a resource erasure redacts its path.
--
-- Sweep spec D2 (as amended: place signals only), Q38, and *Carried to erasure cut 2*: "specify how
-- a redaction path, which may carry concrete array indices, maps to a collapsed Q26 path for
-- closure". Task 01a11ddd-9653-7640-9adc-815eb43fb1d1, rulings 1 and 2 (2026-10-08).
--
-- Until now place_closure answered NULL on kb_events.payload and kb_events.metadata, so a ledger
-- finding read open for good: the sweep reads the ledger once per detector version and never
-- revisits a row. Since erasure cut 2 the act rewrites each allowlisted path of its trail's events
-- to a sentinel and records one kb_event_field_redactions row per (event, path). That row is the
-- place's own record that it holds a sentinel, so it is a place signal.
--
--   1. sensitivity.ledger_path_covers(dotted, surface, path): one conversion of a redaction or
--      allowlist path (dotted, `[*]` for array elements, no index ever recorded) to the sweep's
--      collapsed Q26 form, matched by prefix. ledger_remediability reads it, no longer its own copy.
--   2. place_closure takes the finding's path. A ledger finding closes as `sentinel` when a
--      redaction row on its event covers its path, whichever erasure wrote the row (ruling 1): an
--      edge event in two resources' trails holds a sentinel after either is erased.
--   3. Every caller passes the path (ruling 2: one definition of closed): the finding_closure view,
--      expire_erased_fingerprints, close_cardless_card_findings, and erased_place_findings, which
--      gains the ledger arm 20261004000010 reserved for cut 2, so a redacted ledger finding's
--      digests expire on the window like any other erased place's. Each body is its latest
--      definition verbatim except for the path argument and that arm.

-- ---------------------------------------------------------------------------
-- Section 1. One conversion, one prefix rule.
-- ---------------------------------------------------------------------------
-- A finding falls under a path by prefix: property_*'s value is a whole subtree, written `/value/?`
-- below a user-map key. Both forms collapse array elements, so `[*]` and `/*` meet without indices.
CREATE FUNCTION sensitivity.ledger_path_covers(p_dotted text, p_surface text, p_path text)
RETURNS boolean
LANGUAGE sql IMMUTABLE AS $$
    SELECT w.surface = p_surface
       AND (p_path = w.path OR left(p_path, length(w.path) + 1) = w.path || '/')
      FROM (SELECT CASE WHEN p_dotted LIKE 'metadata.%' THEN 'kb_events.metadata'
                        ELSE 'kb_events.payload' END AS surface,
                   '/' || replace(replace(regexp_replace(p_dotted, '^metadata\.', ''), '[*]', '/*'), '.', '/')
                       AS path) w;
$$;

COMMENT ON FUNCTION sensitivity.ledger_path_covers(text, text, text) IS
'Whether a ledger finding at (p_surface, p_path), in the sweep''s collapsed form (Q26), lies at or under p_dotted, a path in the dotted form of _erasure_redact_paths and kb_event_field_redactions (`metadata.<key>` on kb_events.metadata, `[*]` for array elements). The one conversion between the two forms: ledger_remediability and place_closure both read it.';

-- 20261010100000 Section 5's body, its inline conversion replaced by ledger_path_covers.
CREATE OR REPLACE FUNCTION sensitivity.ledger_remediability(p_surface text, p_event_type text, p_path text, p_resource uuid)
RETURNS text
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT CASE WHEN p_resource IS NULL OR NOT EXISTS (
               SELECT 1
                 FROM _erasure_redact_paths() l
                WHERE l.class <> 'keep'
                  AND (l.event_type = p_event_type OR l.event_type IS NULL)
                  AND sensitivity.ledger_path_covers(l.path, p_surface, p_path))
                THEN 'unremediable'
                WHEN EXISTS (SELECT 1 FROM kb_cogmaps c WHERE c.telos_resource_id = p_resource)
                THEN 'blocked:map-grain'
                ELSE 'remediable' END;
$$;

-- ---------------------------------------------------------------------------
-- Section 2. place_closure reads a ledger place at the finding's path.
-- ---------------------------------------------------------------------------
-- 20261003150000's body, with the ledger arm first. A ledger place never reads row_missing: the
-- ledger deletes nothing. A finding on a path no row covers stays open, on a redacted event too:
-- the act rewrote only what it lists. The signature changes, so the view that reads it is dropped
-- with it and recreated below; the three functions that call it are replaced after it.
DROP VIEW sensitivity.finding_closure;
DROP FUNCTION sensitivity.place_closure(text, uuid, text);

CREATE FUNCTION sensitivity.place_closure(p_surface text, p_target uuid, p_hash text, p_path text) RETURNS text
LANGUAGE sql STABLE AS $$
    SELECT CASE
        WHEN p_surface IN ('kb_events.payload', 'kb_events.metadata') THEN
            CASE WHEN EXISTS (SELECT 1 FROM kb_event_field_redactions r
                               WHERE r.event_id = p_target
                                 AND sensitivity.ledger_path_covers(r.path, p_surface, p_path))
                 THEN 'sentinel' END
        WHEN p_surface NOT IN ('kb_block_content.content', 'kb_chunk_content.content', 'kb_chunks.header_path',
                               'kb_resources.title', 'kb_resources.origin_uri', 'kb_properties.property_key',
                               'kb_edges.label', 'kb_citation_audits.reason', 'kb_remote_sources.uri',
                               'kb_properties.property_value') THEN NULL
        WHEN x.present IS NULL THEN 'row_missing'
        -- The resource erasure act's D4 sentinels (20260929040730, steps 9a, 9b, 9d).
        WHEN (p_surface = 'kb_resources.title'           AND x.unit = 'erased-' || p_target::text)
          OR (p_surface = 'kb_resources.origin_uri'      AND x.unit = 'erased:' || p_target::text)
          OR (p_surface = 'kb_properties.property_key'   AND x.unit ~ '^erased-key-[0-9]+$')
          OR (p_surface = 'kb_properties.property_value' AND x.unit = '"erased"') THEN 'sentinel'
        WHEN coalesce(x.unit, '') = ''
          OR (p_surface = 'kb_properties.property_value' AND x.unit IN ('{}', '[]', '""', 'null')) THEN 'content_empty'
        WHEN o.content_hash <> p_hash THEN 'changed'
    END
      FROM (SELECT 1) one
      LEFT JOIN LATERAL (
          SELECT true, bc.content FROM kb_block_content bc
           WHERE p_surface = 'kb_block_content.content' AND bc.block_revision_id = p_target
          UNION ALL
          SELECT true, cc.content FROM kb_chunk_content cc
           WHERE p_surface = 'kb_chunk_content.content' AND cc.chunk_id = p_target
          UNION ALL
          SELECT true, c.header_path FROM kb_chunks c
           WHERE p_surface = 'kb_chunks.header_path' AND c.id = p_target
          UNION ALL
          SELECT true, r.title FROM kb_resources r
           WHERE p_surface = 'kb_resources.title' AND r.id = p_target
          UNION ALL
          SELECT true, r.origin_uri FROM kb_resources r
           WHERE p_surface = 'kb_resources.origin_uri' AND r.id = p_target
          UNION ALL
          SELECT true, p.property_key FROM kb_properties p
           WHERE p_surface = 'kb_properties.property_key' AND p.id = p_target
          UNION ALL
          SELECT true, p.property_value::text FROM kb_properties p
           WHERE p_surface = 'kb_properties.property_value' AND p.id = p_target
          UNION ALL
          SELECT true, e.label FROM kb_edges e
           WHERE p_surface = 'kb_edges.label' AND e.id = p_target
          UNION ALL
          SELECT true, a.reason FROM kb_citation_audits a
           WHERE p_surface = 'kb_citation_audits.reason' AND a.id = p_target
          UNION ALL
          SELECT true, s.uri FROM kb_remote_sources s
           WHERE p_surface = 'kb_remote_sources.uri' AND s.id = p_target
      ) x (present, unit) ON true
      LEFT JOIN sensitivity.place_observations o ON o.surface = p_surface AND o.target_id = p_target;
$$;

COMMENT ON FUNCTION sensitivity.place_closure(text, uuid, text, text) IS
'How a finding''s place closes, from what it holds now (sweep D2 as amended): row_missing, sentinel, content_empty, or on a mutable surface changed by the sweep''s own later observation; NULL while it is open, and on a surface this function does not know. A ledger place (kb_events.payload, kb_events.metadata) closes as sentinel when a kb_event_field_redactions row on its event covers p_path, whichever erasure wrote it, and never otherwise. p_path is read only on the ledger.';

-- A finding is closed when its place says so. An open finding has no row here.
CREATE VIEW sensitivity.finding_closure AS
    SELECT c.finding_id, c.closed_by
      FROM (SELECT f.id AS finding_id, sensitivity.place_closure(f.surface, f.target_id, f.content_hash, f.path) AS closed_by
              FROM sensitivity.findings f) c
     WHERE c.closed_by IS NOT NULL;

-- ---------------------------------------------------------------------------
-- Section 3. The callers pass the path.
-- ---------------------------------------------------------------------------
-- 20261004000010's body, with the ledger arm it reserved for cut 2 last. Its first arm already
-- holds a ledger finding attributed to an erased resource; this one holds a redacted path wherever
-- the finding is attributed (ruling 1), and only a redacted one, so an unredacted path's digests
-- stay with the finding that still describes the place.
CREATE OR REPLACE FUNCTION sensitivity.erased_place_findings() RETURNS TABLE (finding_id uuid)
LANGUAGE sql STABLE AS $$
    WITH f AS (
        SELECT f.* FROM sensitivity.findings f
         WHERE f.fingerprint_state <> 'expired'
           AND NOT EXISTS (SELECT 1 FROM sensitivity.erased_closures c WHERE c.finding_id = f.id)
    )
    -- A place the scan attributes to an erased resource: its title, origin_uri, blocks, chunks,
    -- citation reasons, and its own and its blocks' properties.
    SELECT f.id FROM f
      JOIN kb_resources r ON r.id = f.resource_id
     WHERE r.erased_at IS NOT NULL
    UNION
    -- An edge's label, with an erased resource at either end (act step 9c).
    SELECT f.id FROM f
      JOIN kb_edges e ON e.id = f.target_id
     WHERE f.surface = 'kb_edges.label'
       AND EXISTS (SELECT 1 FROM kb_resources r
                    WHERE r.erased_at IS NOT NULL
                      AND ((e.source_table = 'kb_resources' AND r.id = e.source_id)
                        OR (e.target_table = 'kb_resources' AND r.id = e.target_id)))
    UNION
    -- An edge-owned property's key or value, on such an edge (act step 9d).
    SELECT f.id FROM f
      JOIN kb_properties p ON p.id = f.target_id AND p.owner_table = 'kb_edges'
      JOIN kb_edges e ON e.id = p.owner_id
     WHERE f.surface IN ('kb_properties.property_key', 'kb_properties.property_value')
       AND EXISTS (SELECT 1 FROM kb_resources r
                    WHERE r.erased_at IS NOT NULL
                      AND ((e.source_table = 'kb_resources' AND r.id = e.source_id)
                        OR (e.target_table = 'kb_resources' AND r.id = e.target_id)))
    UNION
    -- Block and chunk content an erasure act emptied while the resource lives on: the scrub's
    -- prior revisions and the principal act's governed content (content_empty, never row_missing).
    SELECT f.id FROM f
     WHERE f.surface IN ('kb_block_content.content', 'kb_chunk_content.content', 'kb_chunks.header_path')
       AND sensitivity.place_closure(f.surface, f.target_id, f.content_hash, f.path) = 'content_empty'
    UNION
    -- A remote source the act orphaned and deleted (act step 9e, the only deleter).
    SELECT f.id FROM f
     WHERE f.surface = 'kb_remote_sources.uri'
       AND NOT EXISTS (SELECT 1 FROM kb_remote_sources s WHERE s.id = f.target_id)
    UNION
    -- A ledger path a resource erasure rewrote to its sentinel (kb_event_field_redactions).
    SELECT f.id FROM f
     WHERE f.surface IN ('kb_events.payload', 'kb_events.metadata')
       AND sensitivity.place_closure(f.surface, f.target_id, f.content_hash, f.path) = 'sentinel';
$$;

-- 20261007000010's body, verbatim except for the path argument.
CREATE OR REPLACE FUNCTION sensitivity.expire_erased_fingerprints(p_window interval DEFAULT '30 days')
RETURNS int LANGUAGE plpgsql AS $$
DECLARE
    v_n int;
BEGIN
    INSERT INTO sensitivity.erased_closures (finding_id)
    SELECT f.id
      FROM sensitivity.erased_place_findings() e
      JOIN sensitivity.findings f ON f.id = e.finding_id
     WHERE sensitivity.place_closure(f.surface, f.target_id, f.content_hash, f.path) IS NOT NULL
    ON CONFLICT (finding_id) DO NOTHING;

    -- Locked in id order, so two ticks expiring the same findings cannot deadlock.
    WITH due AS (
        SELECT f.id, f.content_hash FROM sensitivity.erased_closures c
          JOIN sensitivity.findings f ON f.id = c.finding_id
         WHERE c.closed_seen_at <= now() - p_window
           AND f.fingerprint_state <> 'expired'
           AND sensitivity.place_closure(f.surface, f.target_id, f.content_hash, f.path) IS NOT NULL
         ORDER BY f.id
           FOR UPDATE OF f
    ), memo_gone AS (
        DELETE FROM sensitivity.memo m USING due WHERE m.content_hash = due.content_hash
    ), gone AS (
        DELETE FROM sensitivity.finding_fingerprints ff USING due WHERE ff.finding_id = due.id
    )
    UPDATE sensitivity.findings f
       SET fingerprint_state = 'expired',
           content_hash = encode(sha256(convert_to('expired:' || f.id::text, 'UTF8')), 'hex')
      FROM due
     WHERE f.id = due.id;
    GET DIAGNOSTICS v_n = ROW_COUNT;

    -- Last, and never waiting: each skips rows another call holds, so neither waits on one, and
    -- the finding steps above have taken their locks before these take any.
    -- A clean unit's hash lives as long as the window, whatever place it came from; oldest first.
    DELETE FROM sensitivity.memo m
     WHERE (m.content_hash, m.detector_id, m.detector_version) IN (
            SELECT d.content_hash, d.detector_id, d.detector_version FROM sensitivity.memo d
             WHERE d.memoized_at <= now() - p_window
             ORDER BY d.memoized_at
             LIMIT 50000
               FOR UPDATE SKIP LOCKED);

    -- An erased resource's place observations, the husk's included.
    DELETE FROM sensitivity.place_observations o
     WHERE (o.surface, o.target_id) IN (
            SELECT po.surface, po.target_id FROM sensitivity.place_observations po
              JOIN kb_resources r ON r.id = po.target_id
             WHERE po.surface IN ('kb_resources.title', 'kb_resources.origin_uri')
               AND r.erased_at IS NOT NULL
               FOR UPDATE OF po SKIP LOCKED);

    RETURN v_n;
END;
$$;

-- 20261004120000's body, verbatim except for the path argument.
CREATE OR REPLACE FUNCTION sensitivity.close_cardless_card_findings() RETURNS int
LANGUAGE sql AS $$
    WITH current AS (
        SELECT version FROM sensitivity.detectors WHERE id = 'payment_card'
    ), candidates AS (
        SELECT f.id, f.surface, f.target_id, f.path, c.version
          FROM sensitivity.findings f, current c
         WHERE f.detector_id = 'payment_card' AND f.detector_version < c.version
           AND NOT EXISTS (SELECT 1 FROM sensitivity.dispositions x WHERE x.finding_id = f.id)
           AND sensitivity.place_closure(f.surface, f.target_id, f.content_hash, f.path) IS NULL
    ), units AS (
        SELECT k.id, u.unit,
               EXISTS (SELECT 1 FROM sensitivity.detector_matches('payment_card', k.version, u.unit)) AS card
          FROM candidates k
          CROSS JOIN LATERAL (
              SELECT s.unit FROM sensitivity.src_kb_block_content__content s
               WHERE k.surface = 'kb_block_content.content' AND s.target_id = k.target_id
              UNION ALL SELECT s.unit FROM sensitivity.src_kb_chunk_content__content s
               WHERE k.surface = 'kb_chunk_content.content' AND s.target_id = k.target_id
              UNION ALL SELECT s.unit FROM sensitivity.src_kb_chunks__header_path s
               WHERE k.surface = 'kb_chunks.header_path' AND s.target_id = k.target_id
              UNION ALL SELECT s.unit FROM sensitivity.src_kb_resources__title s
               WHERE k.surface = 'kb_resources.title' AND s.target_id = k.target_id
              UNION ALL SELECT s.unit FROM sensitivity.src_kb_resources__origin_uri s
               WHERE k.surface = 'kb_resources.origin_uri' AND s.target_id = k.target_id
              UNION ALL SELECT s.unit FROM sensitivity.src_kb_properties__property_key s
               WHERE k.surface = 'kb_properties.property_key' AND s.target_id = k.target_id
              UNION ALL SELECT s.unit FROM sensitivity.src_kb_edges__label s
               WHERE k.surface = 'kb_edges.label' AND s.target_id = k.target_id
              UNION ALL SELECT s.unit FROM sensitivity.src_kb_citation_audits__reason s
               WHERE k.surface = 'kb_citation_audits.reason' AND s.target_id = k.target_id
              UNION ALL SELECT s.unit FROM sensitivity.src_kb_remote_sources__uri s
               WHERE k.surface = 'kb_remote_sources.uri' AND s.target_id = k.target_id
              UNION ALL SELECT j.unit FROM sensitivity.src_kb_events__payload s,
                                           sensitivity.jsonb_units(s.doc, s.roots) j
               WHERE k.surface = 'kb_events.payload' AND s.target_id = k.target_id AND j.path = k.path
              UNION ALL SELECT j.unit FROM sensitivity.src_kb_events__metadata s,
                                           sensitivity.jsonb_units(s.doc, s.roots) j
               WHERE k.surface = 'kb_events.metadata' AND s.target_id = k.target_id AND j.path = k.path
              UNION ALL SELECT j.unit FROM sensitivity.src_kb_properties__property_value s,
                                           sensitivity.jsonb_units(s.doc, s.roots) j
               WHERE k.surface = 'kb_properties.property_value' AND s.target_id = k.target_id
                 AND j.path = k.path
          ) u
    ), closed AS (
        INSERT INTO sensitivity.dispositions (finding_id, state)
        SELECT id, 'false_positive'
          FROM units
         GROUP BY id
        HAVING bool_and(unit IS NOT NULL) AND NOT bool_or(card)
        RETURNING 1
    )
    SELECT count(*)::int FROM closed;
$$;

SELECT declare_migration(
    20261011100000,
    'additive',
    'New: the function sensitivity.ledger_path_covers. sensitivity.place_closure is dropped and recreated with a fourth argument, the finding''s path (sensitivity.finding_closure, the view that reads it, is dropped and recreated around it with the same columns); on kb_events.payload and kb_events.metadata it now answers sentinel where a kb_event_field_redactions row on the event covers the path, and NULL as before otherwise. CREATE OR REPLACE, with unchanged signatures and return types, of sensitivity.ledger_remediability (the same answers, its path conversion now ledger_path_covers), sensitivity.expire_erased_fingerprints and sensitivity.close_cardless_card_findings (each passes the path), and sensitivity.erased_place_findings (passes it, and gains an arm for a ledger finding whose path was redacted). Additive: no deployed binary names the sensitivity schema (the grep gate holds it), and the door reads the same int.'
);
