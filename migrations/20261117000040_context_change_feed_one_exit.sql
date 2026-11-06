-- The change feed's last disclosure rulings (owner, 2026-11-06; cross-review
-- tasker-systems/temper-artifacts#203, recommendations 3 to 6).
--
--   1. A RESOURCE LEAVES A CONTEXT ONE WAY. A move wrote resource_departed in the home it left
--      (20261116000020), and a deletion wrote resource_deleted. So the kind told a reader of the
--      old home that a moved resource still existed somewhere it could not see, though every door
--      answers an unreadable resource exactly as one that never existed. A deletion now writes the
--      same departure, through _resource_departure_append, before it projects (the helper needs
--      the resource live). The feed serves the departure and withholds resource_deleted, which
--      _feed_withholds_kind now names, so `kinds` refuses it as an unknown kind. The ledger keeps
--      resource_deleted as it was: replay, the element trail and every other reader see no change.
--      A departure carries only the resource id and the act's own correlation id, so a move's and
--      a deletion's exits are the same entry. Neither act writes anything else into the home it
--      leaves: a move's record anchors on its destination, and a deletion folds nothing.
--
--   2. AN ADMIN ACT DOES NOT MOVE THE CONTEXT CURSOR. latest_event_id_for_context reads every
--      event anchored on the context, so an erasure's edge folds there moved it, and the cursor's
--      uuidv7 dated an erasure the feed never shows. The owner ruled that admin acts keep the
--      confidentiality they were meant to have, over the letter of
--      `the-feed-never-burdens-the-caller-who-ignores-it`. The cursor now skips an admin act's
--      events through the feed's own definition, _event_is_admin_act. That definition now matches
--      a legacy row too: a row with no xact_id (written before 20261116000010) is in an act's span
--      when the act's record has no xact_id either and shares its correlation id and emitter. The
--      feed never serves a legacy row, so nothing it serves changes. No new row can be legacy (the
--      order trigger refuses one outside replay), so the transaction rule's guard against reusing
--      an act's correlation id (20261117000000 item 3) still holds for every row written since.
--
--   3. A BLOCK'S ENTRY NAMES ITS RESOURCE. block_mutated, block_provenance_annotated, block_folded
--      and citation_audited named only a block, and no door re-reads a block by id alone. Their
--      subject is now the block's resource, and the block's id rides beside it as `block`.
--      block_created already named its resource; it gains `block` too. A block row is never
--      deleted (erasure scrubs it in place), so the lookup always finds the resource.
--
--   4. A SUBJECT IS ONE OF THE FEED'S FOUR TABLES. The owner arm took `subject.table` from the
--      payload as raw text. Every stored payload is shaped today, but a permissive payload type
--      added later could have put any string there. An entry is served only when its subject is
--      kb_resources, kb_edges, kb_blobs or kb_contexts with an id; anything else is withheld.
--
-- context_changes_after's signature and result type are unchanged; its entries gain the optional
-- key `block`. resource_delete's signature is unchanged.

-- ── 1. The deletion departs ─────────────────────────────────────────────────
CREATE OR REPLACE FUNCTION _feed_withholds_kind(p_kind text)
RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT p_kind IN ('webhook_received', 'subscription_delivery_disposed', 'resource_deleted')
$$;

COMMENT ON FUNCTION _feed_withholds_kind(text) IS
  'A domain kind the context change feed never serves and refuses in `kinds` (20261117000020, '
  '20261117000040). A connection''s traffic (webhook_received, subscription_delivery_disposed) is '
  'no change to what lives in the context. A deletion is served as its departure, so '
  'resource_deleted is not served as well.';

-- 20260709000050's body, with the departure between the record and the projection. The resource
-- row is locked first, as resource_rehome does, so a deletion and a move of one resource append in
-- the order they hold the lock.
CREATE OR REPLACE FUNCTION resource_delete(p_payload jsonb, p_emitter uuid,
                                           p_metadata jsonb DEFAULT '{}'::jsonb,
                                           p_invocation uuid DEFAULT NULL,
                                           p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_anchor_tbl text; v_anchor uuid;
BEGIN
    PERFORM 1 FROM kb_resources WHERE id = v_resource FOR NO KEY UPDATE;
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table='kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN RAISE EXCEPTION 'resource_delete: resource % has no home', v_resource; END IF;
    v_ev := _event_append('resource_deleted', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    -- No destination: the helper's same-home test never matches, and it departs only a live
    -- resource, so a second deletion departs nothing.
    PERFORM _resource_departure_append(
        v_resource, p_emitter, NULL, NULL,
        (SELECT correlation_id FROM kb_events WHERE id = v_ev),
        p_invocation);
    RETURN _project_resource_deleted(v_ev, p_payload);
END;
$$;

-- The helper's same-home test compares with NULL when there is no destination. Spelled out so it
-- reads as intended: no destination is never the same home.
CREATE OR REPLACE FUNCTION _resource_departure_append(
    p_resource uuid, p_emitter uuid,
    p_to_table text, p_to_id uuid,
    p_correlation uuid, p_invocation uuid DEFAULT NULL
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_table text; v_id uuid;
BEGIN
    PERFORM 1 FROM kb_resources WHERE id = p_resource FOR NO KEY UPDATE;
    SELECT h.anchor_table, h.anchor_id INTO v_table, v_id
      FROM kb_resource_homes h JOIN kb_resources r ON r.id = h.resource_id
     WHERE h.resource_id = p_resource AND r.is_active;
    IF v_id IS NULL
       OR (p_to_id IS NOT NULL AND v_table = p_to_table AND v_id = p_to_id) THEN
        RETURN NULL;
    END IF;
    RETURN _event_append('resource_departed', p_emitter, v_table, v_id,
                         jsonb_build_object('resource_id', p_resource),
                         p_invocation => p_invocation, p_correlation => p_correlation);
END;
$$;

-- ── 2. The admin-act span reaches legacy rows ───────────────────────────────
-- 20261117000000's definition with `=` on xact_id widened to IS NOT DISTINCT FROM, so a legacy
-- row (NULL xact_id) matches only a legacy act record. Signature and result unchanged.
CREATE OR REPLACE FUNCTION _event_is_admin_act(p_kind text, p_correlation uuid, p_emitter uuid, p_xact xid8)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT p_kind IN ('blob_erased', 'context_erased')
        OR (p_correlation IS NOT NULL AND EXISTS (
               SELECT 1
                 FROM kb_events act
                 JOIN kb_event_types act_t ON act_t.id = act.event_type_id
                WHERE act.category = 'admin'
                  AND act.correlation_id = p_correlation
                  AND act.emitter_entity_id = p_emitter
                  AND act.xact_id IS NOT DISTINCT FROM p_xact
                  AND act_t.name IN ('resource_erased', 'principal_erased',
                                     'resource_scrubbed', 'block_history_scrubbed')))
$$;

-- ── 3 and 4. The feed's subjects ────────────────────────────────────────────
-- 20261117000020's body with the subject derivation extended (item 3), the allow-list (item 4),
-- and `block` in the entry.
CREATE OR REPLACE FUNCTION context_changes_after(
    p_profile uuid, p_context uuid,
    p_after_xact xid8, p_after_seq bigint,
    p_limit int, p_kinds text[] DEFAULT NULL
) RETURNS jsonb LANGUAGE sql STABLE AS $$
    WITH gate AS (
        SELECT context_readable_by_profile(p_profile, p_context) AS ok,
               pg_snapshot_xmin(pg_current_snapshot())          AS h
    ),
    bound AS (
        SELECT CASE WHEN p_limit BETWEEN 1 AND 1000 THEN p_limit
                    ELSE (SELECT NULL::int WHERE _feed_refuse_limit(p_limit)) END AS n
    ),
    -- Read only when an edge subject's endpoint is a resource; at most once per read.
    vis AS MATERIALIZED (
        SELECT resource_id FROM resources_visible_to(p_profile)
    ),
    page AS (
        SELECT e.xact_id, e.seq, e.emitter_entity_id, e.occurred_at, e.correlation_id,
               et.name AS kind, subj.t, subj.id, subj.block
          -- The scan budget: at most _feed_scan_budget() of the context's events past the position,
          -- whatever is withheld among them (see item 3).
          FROM (SELECT e0.*
                  FROM kb_events e0
                  CROSS JOIN gate g0
                 WHERE g0.ok
                   AND p_after_xact IS NOT NULL
                   AND e0.producing_anchor_table = 'kb_contexts'
                   AND e0.producing_anchor_id = p_context
                   AND e0.xact_id IS NOT NULL
                   AND e0.xact_id < g0.h
                   AND (e0.xact_id, e0.seq) > (p_after_xact, coalesce(p_after_seq, 0))
                 ORDER BY e0.xact_id, e0.seq
                 LIMIT _feed_scan_budget()) e
          JOIN kb_event_types et ON et.id = e.event_type_id
          -- The subject as the payload names it (20261117000000's derivation, unchanged).
          CROSS JOIN LATERAL (
              SELECT CASE
                         WHEN e.payload->'owner' ? 'table' THEN e.payload->'owner'->>'table'
                         WHEN _feed_uuid(e.payload->>'resource_id') IS NOT NULL THEN 'kb_resources'
                         WHEN _feed_uuid(e.payload->>'edge_id') IS NOT NULL THEN 'kb_edges'
                         WHEN _feed_uuid(e.payload->>'block_id') IS NOT NULL THEN 'kb_content_blocks'
                         WHEN _feed_uuid(e.payload->>'blob_id') IS NOT NULL THEN 'kb_blobs'
                         ELSE 'kb_contexts'
                     END AS t,
                     CASE
                         WHEN e.payload->'owner' ? 'table' THEN _feed_uuid(e.payload->'owner'->>'id')
                         WHEN _feed_uuid(e.payload->>'resource_id') IS NOT NULL THEN _feed_uuid(e.payload->>'resource_id')
                         WHEN _feed_uuid(e.payload->>'edge_id') IS NOT NULL THEN _feed_uuid(e.payload->>'edge_id')
                         WHEN _feed_uuid(e.payload->>'block_id') IS NOT NULL THEN _feed_uuid(e.payload->>'block_id')
                         WHEN _feed_uuid(e.payload->>'blob_id') IS NOT NULL THEN _feed_uuid(e.payload->>'blob_id')
                         ELSE p_context
                     END AS id
          ) raw
          -- A block's subject is its resource, and the block rides beside it (item 3).
          CROSS JOIN LATERAL (
              SELECT CASE WHEN raw.t = 'kb_content_blocks' THEN 'kb_resources' ELSE raw.t END AS t,
                     CASE WHEN raw.t = 'kb_content_blocks'
                          THEN (SELECT b.resource_id FROM kb_content_blocks b WHERE b.id = raw.id)
                          ELSE raw.id END AS id,
                     CASE WHEN raw.t = 'kb_content_blocks' THEN raw.id
                          -- block_created nests its block (payloads::BlockCreated).
                          WHEN raw.t = 'kb_resources'
                          THEN coalesce(_feed_uuid(e.payload->>'block_id'),
                                        _feed_uuid(e.payload#>>'{block,block_id}'))
                     END AS block
          ) subj
         WHERE e.category = 'domain'
           -- Only a subject the feed knows how to name (item 4).
           AND subj.t IN ('kb_resources', 'kb_edges', 'kb_blobs', 'kb_contexts')
           AND subj.id IS NOT NULL
           AND (p_kinds IS NULL OR et.name = ANY (p_kinds))
           AND NOT _feed_withholds_kind(et.name)
           AND NOT _event_is_admin_act(et.name, e.correlation_id, e.emitter_entity_id, e.xact_id)
           AND (subj.t IS DISTINCT FROM 'kb_edges' OR EXISTS (
                   SELECT 1
                     FROM (
                       -- The edge's row. The home is the context the gate admitted; it bounds
                       -- the lookup to this context's edges.
                       SELECT ed.source_table AS st, ed.source_id AS si,
                              ed.target_table AS tt, ed.target_id AS ti
                         FROM kb_edges ed
                        WHERE ed.id = subj.id
                          AND ed.home_anchor_table = 'kb_contexts'
                          AND ed.home_anchor_id = p_context
                       UNION ALL
                       -- A re-assertion names an id with no row (see item 1): its own source,
                       -- target and home are the existing edge's, so they are tested instead.
                       SELECT e.payload#>>'{source,table}', _feed_uuid(e.payload#>>'{source,id}'),
                              e.payload#>>'{target,table}', _feed_uuid(e.payload#>>'{target,id}')
                        WHERE et.name = 'relationship_asserted'
                          AND e.payload#>>'{home,table}' = 'kb_contexts'
                          AND _feed_uuid(e.payload#>>'{home,id}') = p_context
                          AND NOT EXISTS (SELECT 1 FROM kb_edges ed2 WHERE ed2.id = subj.id)
                     ) ends
                    WHERE CASE ends.st
                              WHEN 'kb_resources' THEN ends.si IN (SELECT resource_id FROM vis)
                              ELSE endpoint_readable_by_profile(p_profile, ends.st, ends.si)
                          END
                      AND CASE ends.tt
                              WHEN 'kb_resources' THEN ends.ti IN (SELECT resource_id FROM vis)
                              ELSE endpoint_readable_by_profile(p_profile, ends.tt, ends.ti)
                          END
                   -- Keeps the lookup a per-event key probe (see Cost above).
                   OFFSET 0))
         ORDER BY e.xact_id, e.seq
         LIMIT (SELECT n + 1 FROM bound)
    ),
    -- The last event the budget reaches, when the context has that many past the position. Read
    -- only when the page did not fill.
    cut AS (
        SELECT e.xact_id, e.seq
          FROM kb_events e
          CROSS JOIN gate g
         WHERE g.ok
           AND p_after_xact IS NOT NULL
           AND e.producing_anchor_table = 'kb_contexts'
           AND e.producing_anchor_id = p_context
           AND e.xact_id IS NOT NULL
           AND e.xact_id < g.h
           AND (e.xact_id, e.seq) > (p_after_xact, coalesce(p_after_seq, 0))
         ORDER BY e.xact_id, e.seq
         OFFSET _feed_scan_budget() - 1
         LIMIT 1
    ),
    numbered AS (
        SELECT p.*, row_number() OVER (ORDER BY p.xact_id, p.seq) AS rn FROM page p
    ),
    served AS (
        SELECT n.* FROM numbered n WHERE n.rn <= (SELECT b.n FROM bound b)
    ),
    shaped AS (
        SELECT s.rn, s.xact_id, s.seq,
               jsonb_strip_nulls(jsonb_build_object(
                   'position',    jsonb_build_object('xact', s.xact_id::text, 'seq', s.seq),
                   'kind',        s.kind,
                   'subject',     jsonb_build_object('table', s.t, 'id', s.id),
                   'block',       s.block,
                   'actor',       s.emitter_entity_id,
                   'occurred_at', s.occurred_at,
                   'act',         s.correlation_id
               )) AS entry
          FROM served s
    ),
    last_served AS (
        SELECT xact_id, seq FROM shaped ORDER BY rn DESC LIMIT 1
    )
    SELECT CASE WHEN NOT g.ok THEN NULL ELSE jsonb_build_object(
        'entries',  coalesce((SELECT jsonb_agg(entry ORDER BY rn) FROM shaped), '[]'::jsonb),
        'has_more', (SELECT count(*) FROM page) > (SELECT b.n FROM bound b)
                    OR EXISTS (SELECT 1 FROM cut),
        'through',  (
            SELECT jsonb_build_object('xact', t.x::text, 'seq', t.s)
              FROM (
                  SELECT CASE WHEN (SELECT count(*) FROM page) > (SELECT b.n FROM bound b)
                              THEN (SELECT ls.xact_id FROM last_served ls)
                              WHEN EXISTS (SELECT 1 FROM cut) THEN (SELECT ct.xact_id FROM cut ct)
                              ELSE g.h END AS x,
                         CASE WHEN (SELECT count(*) FROM page) > (SELECT b.n FROM bound b)
                              THEN (SELECT ls.seq FROM last_served ls)
                              WHEN EXISTS (SELECT 1 FROM cut) THEN (SELECT ct.seq FROM cut ct)
                              ELSE 0 END AS s
              ) c
              -- Never move back from the position given.
              CROSS JOIN LATERAL (
                  SELECT CASE WHEN p_after_xact IS NOT NULL
                                   AND (p_after_xact, coalesce(p_after_seq, 0)) > (c.x, c.s)
                              THEN p_after_xact ELSE c.x END AS x,
                         CASE WHEN p_after_xact IS NOT NULL
                                   AND (p_after_xact, coalesce(p_after_seq, 0)) > (c.x, c.s)
                              THEN coalesce(p_after_seq, 0) ELSE c.s END AS s
              ) t
        )
    ) END
      FROM gate g
$$;
SELECT declare_migration(
    20261117000040,
    'additive',
    'Replaces _feed_withholds_kind, resource_delete, _resource_departure_append, _event_is_admin_act and context_changes_after, each with its signature and result type unchanged. A deletion also appends resource_departed (an existing kind) before it projects; the feed withholds resource_deleted, names a block''s resource as the subject of a block entry with an optional `block` key beside it, and serves only subjects in kb_resources, kb_edges, kb_blobs or kb_contexts. _event_is_admin_act matches legacy rows (NULL xact_id) to legacy act records. Nothing an older binary reads is dropped, renamed or reshaped: an older binary ignores the new `block` key, and refuses resource_deleted in `kinds` as the new one does, since both ask _feed_withholds_kind.'
);
