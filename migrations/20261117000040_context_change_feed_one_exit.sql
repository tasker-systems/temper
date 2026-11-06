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
--      a deletion's exits are the same entry. What else each act leaves in that home is made the
--      same outside this migration: a deletion's resource_deleted is withheld here; an update that
--      moves fires the move before its edits (writes::update_resource_in_tx_prepared), so the
--      edits anchor on the destination; and a move settles the home it left as a deletion does
--      (DbBackend::update_resource, replay::resource_deleted_since counting resource_departed).
--      A move's own record anchors on its destination, and a deletion folds nothing.
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
--   5. SYSTEM-COMPUTED KINDS ARE NOT CHANGES (owner ruling 2026-11-06, security review H2).
--      region_materialized, salience_refreshed and relationship_decayed are the region producer's
--      and the decay sweep's own bookkeeping, anchored on the context they recompute. The settle
--      that follows an erasure runs as the system actor in its own transaction, so it escaped
--      _event_is_admin_act, and a system-actor settle in a context nearly fingerprinted an erasure
--      and dated it. They are withheld from the feed and the cursor alike, as _profile_change_events
--      (20261107000000) already leaves them out of what a profile changed. _feed_withholds_kind
--      names them. A relay mirrors records, not region computations, so it loses nothing.
--
--   6. A DELETION FROM BEFORE THIS MIGRATION STILL EXITS (security review L3). Deletions written
--      between 20261116000010 and this migration carry an order but no departure. Withholding
--      resource_deleted would have made their exit silent, which reads as an erasure. A
--      resource_deleted whose transaction holds no departure for its resource is served as
--      resource_departed (_feed_served_kind), so every deletion has exactly one exit entry.
--
--   7. THE CURSOR SKIPS WHAT THE FEED WITHHOLDS (owner ruling 2026-11-06, security review L1;
--      cost, review L2). latest_event_id_for_context now reads context_cursor, the newest event
--      the feed would serve: a domain event, under the same served-kind rule, admin-act rule,
--      subject allow-list and both-endpoints edge rule as context_changes_after. It differs from
--      the feed in two ways only, both about order rather than disclosure: it reads newest-first
--      by occurred_at, as the cursor always has, and it reads rows from before the order columns
--      (the feed skips them because it cannot place them, not because they disclose anything). The
--      predicate text is the feed's, repeated, because the feed's plan depends on its exact shape
--      (20261117000020's Cost notes); a witness pins the two to the same answer. One read scans at
--      most _feed_scan_budget() of the newest events, through idx_kb_events_anchor_occurred, and
--      answers NULL when none of them would be served. NULL never names a withheld event.
--
--   8. AN INVOCATION SHOWS NO ACT ON A DELETED RESOURCE (owner ruling 2026-11-06, security review
--      M2). invocation_show kept every act naming a soft-deleted resource visible to readers of its
--      home, and dropped the same acts once the resource had moved out of their view, so it told a
--      deletion from a move. The exception is removed (crates/temper-substrate/src/readback): a
--      deleted resource is unreadable there as everywhere else.
--
--   9. THE STALENESS DOOR COUNTS AN EXIT (review round 2, M1). anchor_staleness's edge-touch arm
--      kept counting an edge whose endpoint was deleted (still homed here) and stopped counting one
--      whose endpoint moved out, so latest_touch told a deletion from a move. A departure from the
--      anchor is now a touch itself, so both exits move the clock to the same moment. The function
--      is 20260825000020's, with that one arm added.
--
-- context_changes_after's signature and result type are unchanged; its entries gain the optional
-- key `block`, and `kind` is the served kind (item 6). resource_delete's signature is unchanged.

-- ── 1. The deletion departs ─────────────────────────────────────────────────
CREATE OR REPLACE FUNCTION _feed_withholds_kind(p_kind text)
RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT p_kind IN ('webhook_received', 'subscription_delivery_disposed', 'resource_deleted',
                      'region_materialized', 'salience_refreshed', 'relationship_decayed')
$$;

COMMENT ON FUNCTION _feed_withholds_kind(text) IS
  'A domain kind the context change feed never serves and refuses in `kinds` (20261117000020, '
  '20261117000040). A connection''s traffic (webhook_received, subscription_delivery_disposed) is '
  'no change to what lives in the context. A deletion is served as its departure, so '
  'resource_deleted is not served as well (_feed_served_kind serves a deletion with no departure '
  'as one). The system-computed region_materialized, salience_refreshed and relationship_decayed '
  'are bookkeeping, not changes, and the settle after an erasure would date it.';

-- The kind a feed or cursor read serves an event as, or NULL when it serves none (items 1, 5, 6).
-- A resource_deleted is served as resource_departed only when its transaction wrote no departure
-- for the resource from the same anchor (deletions from before this migration, and legacy rows the
-- cursor reads);
-- otherwise the departure is the deletion's one exit and the deletion itself is withheld.
CREATE FUNCTION _feed_served_kind(p_kind text, p_xact xid8, p_payload jsonb,
                                  p_anchor_table text, p_anchor_id uuid)
RETURNS text LANGUAGE sql STABLE AS $$
    SELECT CASE
               WHEN p_kind = 'resource_deleted' THEN
                   CASE WHEN EXISTS (
                            SELECT 1
                              FROM kb_events d
                              JOIN kb_event_types dt ON dt.id = d.event_type_id
                             WHERE d.xact_id = p_xact
                               AND dt.name = 'resource_departed'
                               AND d.producing_anchor_table = p_anchor_table
                               AND d.producing_anchor_id = p_anchor_id
                               AND d.payload->>'resource_id' = p_payload->>'resource_id')
                        THEN NULL ELSE 'resource_departed' END
               WHEN _feed_withholds_kind(p_kind) THEN NULL
               ELSE p_kind
           END
$$;

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
               sk.kind, subj.t, subj.id, subj.block
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
          -- The kind it is served as, if any (items 1, 5, 6).
          CROSS JOIN LATERAL (SELECT _feed_served_kind(et.name, e.xact_id, e.payload,
                                                  e.producing_anchor_table, e.producing_anchor_id) AS kind) sk
         WHERE e.category = 'domain'
           -- Only a subject the feed knows how to name (item 4).
           AND subj.t IN ('kb_resources', 'kb_edges', 'kb_blobs', 'kb_contexts')
           AND subj.id IS NOT NULL
           AND sk.kind IS NOT NULL
           AND (p_kinds IS NULL OR sk.kind = ANY (p_kinds))
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
-- ── 7. The cursor: the newest event the feed would serve ────────────────────
CREATE INDEX idx_kb_events_anchor_occurred
    ON kb_events (producing_anchor_table, producing_anchor_id, occurred_at DESC, id DESC);

-- The newest event anchored on p_context that context_changes_after would serve p_profile, or NULL
-- (item 7). Gated by the context read like the feed: an unreadable context answers NULL. The
-- predicates below are context_changes_after's, repeated verbatim except for the `kinds` filter and
-- the event-type lookup (a lateral, so the walk keeps the index order);
-- crates/temper-substrate/tests/context_change_feed.rs
-- (the_cursor_is_the_newest_event_the_feed_serves) pins the two together.
CREATE FUNCTION context_cursor(p_profile uuid, p_context uuid)
RETURNS uuid LANGUAGE sql STABLE AS $$
    WITH gate AS (
        SELECT context_readable_by_profile(p_profile, p_context) AS ok
    ),
    vis AS MATERIALIZED (
        SELECT resource_id FROM resources_visible_to(p_profile)
    )
    SELECT e.id
          -- The budget: at most _feed_scan_budget() of the context's newest events.
          -- Only the columns the predicates read, in index order, so the walk below streams and
          -- stops at the first event served: the event type is a per-row lookup, not a join that
          -- would hash away the order and filter the whole budget before the LIMIT.
          FROM (SELECT e0.id, e0.event_type_id, e0.category, e0.payload, e0.xact_id,
                       e0.correlation_id, e0.emitter_entity_id, e0.occurred_at,
                       e0.producing_anchor_table, e0.producing_anchor_id
                  FROM kb_events e0
                  CROSS JOIN gate g0
                 WHERE g0.ok
                   AND e0.producing_anchor_table = 'kb_contexts'
                   AND e0.producing_anchor_id = p_context
                 ORDER BY e0.occurred_at DESC, e0.id DESC
                 LIMIT _feed_scan_budget()) e
          CROSS JOIN LATERAL (SELECT t.name FROM kb_event_types t WHERE t.id = e.event_type_id) et
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
          -- The kind it is served as, if any (items 1, 5, 6).
          CROSS JOIN LATERAL (SELECT _feed_served_kind(et.name, e.xact_id, e.payload,
                                                  e.producing_anchor_table, e.producing_anchor_id) AS kind) sk
         WHERE e.category = 'domain'
           -- Only a subject the feed knows how to name (item 4).
           AND subj.t IN ('kb_resources', 'kb_edges', 'kb_blobs', 'kb_contexts')
           AND subj.id IS NOT NULL
           AND sk.kind IS NOT NULL
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
         ORDER BY e.occurred_at DESC, e.id DESC
         LIMIT 1
$$;

COMMENT ON FUNCTION context_cursor(uuid, uuid) IS
  'The context cursor (20261117000040): the newest event on the context that the change feed would '
  'serve the reader, or NULL. Reads at most _feed_scan_budget() of the newest events.';

-- ── 9. The staleness door counts an exit ────────────────────────────────────
-- 20260825000020's body (read back from the live definition), with the departure arm added.
CREATE OR REPLACE FUNCTION anchor_staleness(p_anchor_table text, p_anchor_id uuid, p_principal_kind text, p_principal_id uuid)
 RETURNS TABLE(materialized_at timestamp with time zone, latest_touch timestamp with time zone, is_stale boolean)
 LANGUAGE sql
 STABLE
AS $function$
    WITH vis AS MATERIALIZED (
        -- Computed ONCE for the regions arm. Empty for a non-profile principal, which is why the
        -- cogmap self-read arms below are exemptions rather than redundancies. Carried from
        -- 20260823000010:36-38 / 20260713000050:250-252.
        SELECT v.resource_id FROM resources_visible_to(p_principal_id) v
    ),
    gate AS (
        -- VERBATIM from anchor_shape's gate CTE (20260823000010:59-66), EXISTS conjunct included.
        -- Always exactly one row (no FROM).
        --
        -- THE EXISTS CONJUNCT IS CARRIED FOR SAMENESS, NOT BECAUSE IT IS LOAD-BEARING HERE, and the
        -- difference is worth stating so nobody later "simplifies" the sibling by analogy with this
        -- one. In anchor_shape the EXISTS is load-bearing: that function forces a row via
        -- `LEFT JOIN ... ON true` (20260823000010:181) to carry its envelope, so a tautological
        -- cogmap self-read arm would hand back `emptiness` and `materialized_at` for an invented
        -- uuid -- the oracle argued at :43-58. THIS function ends in `FROM mat, touch`, and `mat` is
        -- empty for a uuid naming no anchor, so the cross join yields ZERO ROWS however the gate
        -- answers. Measured, not reasoned: the INCUMBENT has no gate at all and already returns 0
        -- rows for an invented uuid.
        --
        -- So it is kept because one rule in two places must read identically to stay in sync, not
        -- because removing it would open a hole here. Claiming otherwise would be a safety claim
        -- that is true of one function and false of another -- exactly the kind of prose
        -- 20260823000010:55-58 exists to stop outliving the person who knew the difference.
        SELECT (
            (p_principal_kind = 'profile'
                 AND anchor_readable_by_profile(p_principal_id, p_anchor_table, p_anchor_id))
            OR (p_principal_kind = 'cogmap'
                 AND p_anchor_table = 'kb_cogmaps'
                 AND p_principal_id = p_anchor_id
                 AND EXISTS (SELECT 1 FROM kb_cogmaps m WHERE m.id = p_anchor_id))
        ) AS readable
    ),
    mat AS (
        -- UNCHANGED from 20260823000020:57-67. Its inner UNION ALL subquery is byte-identical to the
        -- one in anchor_shape's `clock` CTE (20260823000010:95-101) and must stay so: the two read
        -- the SAME column off the SAME two tables, and a divergence between them would be a
        -- divergence between what a shape read calls "materialized" and what a staleness read
        -- compares against. (The two CTEs are not byte-identical in full: `clock` additionally
        -- projects `a.eid`, which its `never_clustered` arm needs and this function has no use for.)
        --
        -- Deliberately NOT gated in place: the deny path is the outer WHERE, which yields zero rows,
        -- so the watermark is never projected to a caller who fails the gate. Gating here as well
        -- would put the same predicate in two places for one function to disagree with itself later.
        SELECT ev.occurred_at AS materialized_at
        FROM (
            SELECT c.shape_materialized_event_id AS eid FROM kb_contexts c
             WHERE p_anchor_table = 'kb_contexts' AND c.id = p_anchor_id
            UNION ALL
            SELECT m.shape_materialized_event_id FROM kb_cogmaps m
             WHERE p_anchor_table = 'kb_cogmaps' AND m.id = p_anchor_id
        ) a
        LEFT JOIN kb_events ev ON ev.id = a.eid
    ),
    touch AS (
        SELECT max(occurred_at) AS latest_touch FROM (
            SELECT ev.occurred_at FROM kb_cogmap_regions reg
              JOIN kb_events ev ON ev.id = reg.last_event_id
             WHERE reg.home_anchor_table = p_anchor_table
               AND reg.home_anchor_id    = p_anchor_id
               -- NO `NOT reg.is_folded` HERE, AND THAT IS CORRECT. See header §2. A fold advances
               -- last_event_id and IS a touch; narrowing this arm makes a stale anchor read fresh,
               -- silently. Do not "tighten" it to match anchor_shape's region select.
               --
               -- The member rule, same words and same reason as both incumbent doors
               -- (20260823000010:87-88 as a count, 20260713000050:262-268 as this EXISTS): a region
               -- you can see nothing in is not a region you can see, so its clock is not your clock.
               -- The EXISTS form is taken over the CROSS JOIN LATERAL count because nothing here
               -- needs the cardinality -- only whether the region is enumerable at all.
               -- The cogmap self-read arm is exempt for the same reason it is exempt there.
               AND (p_principal_kind = 'cogmap' OR EXISTS (
                     SELECT 1
                     FROM kb_cogmap_region_members m
                     JOIN vis v ON v.resource_id = m.member_id
                     JOIN kb_resources r ON r.id = m.member_id AND r.is_active
                     WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
               ))
            UNION ALL
            SELECT ev.occurred_at FROM kb_edges e
              JOIN kb_events ev ON ev.id = e.last_event_id
             WHERE e.home_anchor_table = p_anchor_table
               AND e.home_anchor_id    = p_anchor_id
               -- Again NO `NOT e.is_folded`, deliberately, and the covering index built for this
               -- exact folded-inclusive scan is idx_kb_edges_home_all (20260708000008:21-22).
               --
               -- WHY HALF OF AN INCUMBENT AND NOT THE INCUMBENT. The obvious call here is
               -- `edges_visible_to` (20260712000010:295-309), and it is the WRONG call: it mixes the
               -- two predicate classes in one expression -- `NOT e.is_folded` at :297 (CURRENCY)
               -- alongside endpoint visibility at :302-309 (AUTHORIZATION). Calling it would import
               -- the fold narrowing this function must not have, through the back door and without a
               -- line of code to notice. What is needed is only its authorization half, and the
               -- schema already exposes that half under its own name --
               -- `endpoint_readable_by_profile` (20260624000002:292) -- which is also exactly what
               -- element_trail_edge composes for the same purpose (20260719000010:167-168). So:
               -- both endpoints, independently, and the home half is already handled by `gate`.
               --
               -- THIS ARM CANNOT BE TOO TIGHT ON AN UNKNOWN TABLE, AND THAT IS A CONSTRAINT
               -- ARGUMENT, NOT A ROW COUNT. `endpoint_readable_by_profile` falls closed on an
               -- unrecognised table via its `ELSE false` (20260624000002:297). That branch is
               -- UNREACHABLE for a kb_edges endpoint: `kb_edges_source_table_check` and
               -- `kb_edges_target_table_check` constrain both columns to exactly
               -- ARRAY['kb_resources','kb_cogmaps'], which is precisely the domain the CASE
               -- dispatches on. Being drawn from a CHECK constraint rather than from present data,
               -- that holds for rows that do not exist yet.
               --
               -- ── THE `is_active` DISJUNCT, AND WHY IT IS NOT OPTIONAL ────────────────────────
               --
               -- `endpoint_readable_by_profile` is NOT purely an authorization predicate, and
               -- taking it for one is a real defect that an adversarial review caught here before
               -- this shipped. Its kb_resources branch resolves to
               -- `IN (SELECT resource_id FROM resources_visible_to(p_profile))`, and that function
               -- ENDS in the soft-delete read floor -- `JOIN kb_resources r ON ... AND r.is_active`
               -- (20260807000010:223-225), commented in the schema as "a deleted resource is
               -- invisible on every axis".
               --
               -- So `is_active` wears both hats, and for THIS function the currency hat is the one
               -- that matters. A `resource_deleted` is a STRUCTURAL event
               -- (crates/temper-substrate/src/replay.rs:737-744) -- the resource leaves the node set
               -- (substrate.rs:93-99, `AND r.is_active`), so the partition genuinely changes and the
               -- anchor genuinely IS stale. `_project_resource_deleted` flips `is_active` and
               -- NOTHING ELSE: it folds no edge and advances no `last_event_id`, so the edge stays
               -- live and unfolded, still carrying its real post-watermark stamp.
               --
               -- Gate on readability ALONE and the sequence below tells the OWNER a lie:
               --   t0 materialize        -> fresh
               --   t1 assert edge X->Y   -> stale   (correct, both versions)
               --   t2 delete Y           -> stale under the incumbent, FRESH under a readable-only
               --                            gate -- and the anchor is MORE out of date at t2, not
               --                            less. Nothing is denied to the owner; they simply
               --                            cannot see Y because Y is DEAD.
               -- That is the same silent failure §2 refuses for `is_folded`, arriving through
               -- `is_active` instead, and the gate can only ever REMOVE touches, so every
               -- differential it produces is an under-report.
               --
               -- The fix is the second disjunct: an endpoint homed in THIS anchor still moves this
               -- anchor's clock, alive or dead. It is disclosure-safe rather than merely convenient,
               -- and the argument is exact: the outer WHERE has already established that this
               -- principal may read this anchor, and anchor-readability IMPLIES visibility of every
               -- ACTIVE resource homed in it -- `resources_visible_to`'s context arm calls
               -- `contexts_readable_by` with the same argument `context_readable_by_profile` does,
               -- and its cogmap arms are `cogmap_readable_by_profile`'s two disjuncts verbatim. So
               -- for an admitted caller this disjunct adds back EXACTLY the dead-but-homed
               -- endpoints and nothing else. It is redundant for live ones, deliberately: stating
               -- the rule as "readable, or part of the formation you are already reading" is what
               -- makes it self-documenting.
               --
               -- A cross-anchor endpoint -- one NOT homed here -- still requires readability. That
               -- is the disclosure this arm exists to close, and it is untouched. Such an edge is
               -- also formation-irrelevant, so gating it out removes a pre-existing over-report --
               -- but the citation for that takes TWO hops, and an earlier draft of this comment
               -- collapsed them into one wrong pointer. `substrate.rs:107-112` loads edges by HOME
               -- ANCHOR alone and applies no endpoint filter; the both-endpoints-must-be-members
               -- test is downstream in `affinity.rs:229`
               -- (`if members.contains(&e.src) && members.contains(&e.tgt)`), where `members`
               -- derives from the `nodes` set built at `substrate.rs:93-99`.
               --
               -- `kb_resource_homes` is the right table to ask: the delete projection does not
               -- touch it, so the home row survives the soft-delete. `kb_cogmaps` endpoints have no
               -- home row and are unaffected -- they keep the readability test alone, which is
               -- correct, since a cogmap endpoint is never part of a resource formation.
               AND (p_principal_kind = 'cogmap' OR (
                     (endpoint_readable_by_profile(p_principal_id, e.source_table, e.source_id)
                      OR EXISTS (SELECT 1 FROM kb_resource_homes h
                                  WHERE h.resource_id  = e.source_id
                                    AND h.anchor_table = p_anchor_table
                                    AND h.anchor_id    = p_anchor_id))
                 AND (endpoint_readable_by_profile(p_principal_id, e.target_table, e.target_id)
                      OR EXISTS (SELECT 1 FROM kb_resource_homes h
                                  WHERE h.resource_id  = e.target_id
                                    AND h.anchor_table = p_anchor_table
                                    AND h.anchor_id    = p_anchor_id))
               ))
            UNION ALL
            -- A member that left by an authored act (20261117000040, review round 2 M1). Both exits,
            -- a deletion and a move out, write one resource_departed on the home they leave, and
            -- that departure is the touch: the formation lost a member. Without it an edge to a
            -- deleted endpoint kept counting (it is still homed here) while one to a moved-out
            -- endpoint stopped (it is neither readable nor homed here), so latest_touch told this
            -- anchor's readers which exit it was. With it both exits move the clock to the same
            -- moment, which is later than any edge touch the moved endpoint stops contributing.
            -- Read newest-first through idx_kb_events_anchor_occurred, bounded below by the
            -- watermark, so it stops at the first departure or at the materialize.
            SELECT d.occurred_at
              FROM (SELECT d0.occurred_at
                      FROM kb_events d0
                     WHERE d0.producing_anchor_table = p_anchor_table
                       AND d0.producing_anchor_id    = p_anchor_id
                       AND d0.event_type_id = (SELECT dt.id FROM kb_event_types dt
                                                WHERE dt.name = 'resource_departed')
                       AND d0.occurred_at > coalesce((SELECT materialized_at FROM mat),
                                                     '-infinity'::timestamptz)
                     ORDER BY d0.occurred_at DESC
                     LIMIT 1) d
        ) t
    )
    SELECT mat.materialized_at, touch.latest_touch,
           COALESCE(touch.latest_touch > mat.materialized_at, mat.materialized_at IS NULL)
    FROM mat, touch
    -- Deny yields ZERO ROWS, never a NULL row and never an error -- the "no view from nowhere"
    -- pattern (20260628000001:1-3), and the same thing this function already does for an anchor that
    -- does not exist (`mat` contributes no row). A NULL `readable` rejects the row, so this fails
    -- CLOSED: `readable` is an expression over another function, and its two-valuedness is that
    -- function's property, not this one's (20260823000010:135-141 makes the same argument).
    WHERE (SELECT readable FROM gate);
$function$;

SELECT declare_migration(
    20261117000040,
    'additive',
    'Adds _feed_served_kind(text, xid8, jsonb, text, uuid), context_cursor(uuid, uuid) and the index idx_kb_events_anchor_occurred on kb_events (producing_anchor_table, producing_anchor_id, occurred_at DESC, id DESC). Replaces anchor_staleness (a departure is a touch), _feed_withholds_kind, resource_delete, _resource_departure_append, _event_is_admin_act and context_changes_after, each with its signature and result type unchanged. The feed and the cursor withhold region_materialized, salience_refreshed and relationship_decayed; a resource_deleted with no departure in its transaction is served as resource_departed. A deletion also appends resource_departed (an existing kind) before it projects; the feed withholds resource_deleted, names a block''s resource as the subject of a block entry with an optional `block` key beside it, and serves only subjects in kb_resources, kb_edges, kb_blobs or kb_contexts. _event_is_admin_act matches legacy rows (NULL xact_id) to legacy act records. Nothing an older binary reads is dropped, renamed or reshaped: an older binary ignores the new `block` key, and refuses resource_deleted in `kinds` as the new one does, since both ask _feed_withholds_kind.'
);
