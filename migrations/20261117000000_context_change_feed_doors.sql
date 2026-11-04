-- The context change feed's doors (task 01a1a1a5-773a-7599-a157-6128705ef71e, PR 2): what the
-- HTTP, MCP and CLI reads of context_changes_after (20261116000010) need from the schema.
--
--   1. _event_is_admin_act answers from a partial index of admin records. As first written it
--      probed idx_kb_events_correlation and filtered emitter and kind row by row, so every event
--      sharing a correlation id was visited once for each feed row tested. A page over one
--      100,000-event correlated span (an ingest, say) took 15.5 s on a development database. Every
--      record it looks for (resource_erased, principal_erased, resource_scrubbed,
--      block_history_scrubbed) is category 'admin', and admin rows are few, so the probe now reads
--      only those: the same page takes 4 ms. Withholding a whole 100,000-event span that IS an
--      admin act takes 0.5 s, because each row must still be tested. Real admin-act spans are tens
--      of events. The category predicate changes no answer: category = 'admin' only restates what each named kind
--      already is (kb_event_types.category), so that predicate alone serves or withholds nothing
--      differently. Item 3 narrows what is withheld.
--      Not built CONCURRENTLY: no migration in this repo uses the no-transaction escape, and the
--      partial index holds only admin rows, so the build is one scan of kb_events.
--
--   2. kb_feed_position_keys: the one key that seals a feed position (owner ruling 2026-11-03,
--      held in the database, minted by the server on first use). A position must hide xact_id and
--      seq, which both expose cluster-wide write volume, and must be refused when it was issued for
--      another context or forged. The server seals (xact_id, seq) under this key with the context
--      as associated data. The key grants nothing: a holder of the database can already read
--      every row, and every read through a position is still gated on the context. The server
--      inserts the row (ON CONFLICT DO NOTHING) and reads back whichever won, so two first readers
--      agree. No migration writes it, because a migration cannot draw randomness the server
--      trusts. Replay does not copy it (it is in neither of replay's table lists): a rebuilt
--      database mints a new key, and every position issued before is refused, never misread.
--
--   3. An admin act's span is its own transaction. _event_is_admin_act withheld every event that
--      shared an admin record's correlation id and emitter, in any transaction. Correlation ids are
--      chosen by callers (ActInput.correlation_id on every authored write; principal erasure's
--      request reference), so an admin could hide ordinary authored writes from every reader's
--      feed by giving them an act's correlation id: afterwards, by reusing the reference an act
--      handed back, or before, by naming an earlier act's correlation id as principal erasure's
--      reference. Every admin act (resource_erasure_execute, resource_field_scrub_execute,
--      block_history_scrub_execute, principal_erasure_execute) is one statement and one
--      transaction, and what follows its commit (byte release, region settling as the system
--      actor) appends nothing under the operator's emitter. So the span withheld is now the
--      events written in the act's own transaction: same correlation id, same emitter, same
--      xact_id. Nothing an act wrote is served that was withheld before.

CREATE INDEX idx_kb_events_admin_act_correlation
    ON kb_events (correlation_id, emitter_entity_id, xact_id)
    WHERE category = 'admin';

-- p_xact is the served event's own xact_id. The feed serves only rows that have one.
DROP FUNCTION _event_is_admin_act(text, uuid, uuid);
CREATE FUNCTION _event_is_admin_act(p_kind text, p_correlation uuid, p_emitter uuid, p_xact xid8)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT p_kind IN ('blob_erased', 'context_erased')
        OR (p_correlation IS NOT NULL AND EXISTS (
               SELECT 1
                 FROM kb_events act
                 JOIN kb_event_types act_t ON act_t.id = act.event_type_id
                WHERE act.category = 'admin'
                  AND act.correlation_id = p_correlation
                  AND act.emitter_entity_id = p_emitter
                  AND act.xact_id = p_xact
                  AND act_t.name IN ('resource_erased', 'principal_erased',
                                     'resource_scrubbed', 'block_history_scrubbed')))
$$;

-- context_changes_after as 20261116000010 wrote it, with one change: it passes the event's
-- xact_id to _event_is_admin_act. Signature and result unchanged.
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
    page AS (
        SELECT e.xact_id, e.seq, e.emitter_entity_id, e.occurred_at, e.correlation_id,
               et.name AS kind, e.payload
          FROM kb_events e
          JOIN kb_event_types et ON et.id = e.event_type_id
          CROSS JOIN gate g
         WHERE g.ok
           AND p_after_xact IS NOT NULL
           AND e.producing_anchor_table = 'kb_contexts'
           AND e.producing_anchor_id = p_context
           AND e.xact_id IS NOT NULL
           AND e.xact_id < g.h
           AND (e.xact_id, e.seq) > (p_after_xact, coalesce(p_after_seq, 0))
           AND e.category = 'domain'
           AND (p_kinds IS NULL OR et.name = ANY (p_kinds))
           AND NOT _event_is_admin_act(et.name, e.correlation_id, e.emitter_entity_id, e.xact_id)
         ORDER BY e.xact_id, e.seq
         LIMIT (SELECT n + 1 FROM bound)
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
                   'subject',     jsonb_build_object('table', subj.t, 'id', subj.id),
                   'actor',       s.emitter_entity_id,
                   'occurred_at', s.occurred_at,
                   'act',         s.correlation_id
               )) AS entry
          FROM served s
          CROSS JOIN LATERAL (
              SELECT CASE
                         WHEN s.payload->'owner' ? 'table' THEN s.payload->'owner'->>'table'
                         WHEN _feed_uuid(s.payload->>'resource_id') IS NOT NULL THEN 'kb_resources'
                         WHEN _feed_uuid(s.payload->>'edge_id') IS NOT NULL THEN 'kb_edges'
                         WHEN _feed_uuid(s.payload->>'block_id') IS NOT NULL THEN 'kb_content_blocks'
                         WHEN _feed_uuid(s.payload->>'blob_id') IS NOT NULL THEN 'kb_blobs'
                         ELSE 'kb_contexts'
                     END AS t,
                     CASE
                         WHEN s.payload->'owner' ? 'table' THEN _feed_uuid(s.payload->'owner'->>'id')
                         WHEN _feed_uuid(s.payload->>'resource_id') IS NOT NULL THEN _feed_uuid(s.payload->>'resource_id')
                         WHEN _feed_uuid(s.payload->>'edge_id') IS NOT NULL THEN _feed_uuid(s.payload->>'edge_id')
                         WHEN _feed_uuid(s.payload->>'block_id') IS NOT NULL THEN _feed_uuid(s.payload->>'block_id')
                         WHEN _feed_uuid(s.payload->>'blob_id') IS NOT NULL THEN _feed_uuid(s.payload->>'blob_id')
                         ELSE p_context
                     END AS id
          ) subj
    ),
    last_served AS (
        SELECT xact_id, seq FROM shaped ORDER BY rn DESC LIMIT 1
    )
    SELECT CASE WHEN NOT g.ok THEN NULL ELSE jsonb_build_object(
        'entries',  coalesce((SELECT jsonb_agg(entry ORDER BY rn) FROM shaped), '[]'::jsonb),
        'has_more', (SELECT count(*) FROM page) > (SELECT b.n FROM bound b),
        'through',  (
            SELECT jsonb_build_object('xact', t.x::text, 'seq', t.s)
              FROM (
                  SELECT CASE WHEN (SELECT count(*) FROM page) > (SELECT b.n FROM bound b)
                              THEN (SELECT ls.xact_id FROM last_served ls) ELSE g.h END AS x,
                         CASE WHEN (SELECT count(*) FROM page) > (SELECT b.n FROM bound b)
                              THEN (SELECT ls.seq FROM last_served ls) ELSE 0 END AS s
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

CREATE TABLE kb_feed_position_keys (
    id      integer PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    key     bytea NOT NULL CHECK (length(key) = 32),
    created timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE kb_feed_position_keys IS
  'The one key that seals context change feed positions (20261117000000). Minted by the server on '
  'first use; never written by a migration and never replayed.';

SELECT declare_migration(
    20261117000000,
    'additive',
    'Adds a partial index on kb_events (admin rows only) and the table kb_feed_position_keys. Replaces _event_is_admin_act(text, uuid, uuid) with _event_is_admin_act(text, uuid, uuid, xid8), whose one caller, context_changes_after, is replaced in the same migration with its signature and result type unchanged. No binary query names _event_is_admin_act, and nothing an older binary reads is dropped, renamed or reshaped.'
);
