-- The context change feed withholds two things the reader could not learn through any other door
-- (cross-review tasker-systems/temper-artifacts#203, 2026-11-04; owner ruling 2026-11-05).
--
--   1. AN EDGE IS SERVED ONLY WHILE BOTH ITS ENDPOINTS ARE READABLE. An edge homed in the context
--      can join a resource the reader cannot read. edges_visible_to hides such an edge, and so does
--      every edge door: the graph, the facets read, the trail, and since tasker-systems/temper#1299
--      the mutation doors, which answer it as they answer a missing edge. The feed named it from
--      birth, with every retype, reweight, facet change and fold, the time and actor of each, and so
--      its liveness. An entry whose subject is an edge (kb_edges: the relationship_* kinds and the
--      edge-owned property_* kinds alike) is now served only when the edge's row exists and both
--      endpoints are readable by the reader NOW, by the same per-endpoint rule as edges_visible_to
--      and endpoint_readable_by_profile. There is no fold test: a fold of a visible edge is a change
--      the reader could see, and stays served. The home is not tested again: it is the context, and
--      the gate already requires it readable. "Now" means an edge whose endpoint has since left the
--      reader's view is withheld for its whole history, its earlier entries included, which is what
--      edges_visible_to answers today. A relay loses nothing it held: it never held a hidden edge,
--      and an endpoint leaving the context is that endpoint's own entry (resource_departed,
--      resource_deleted). An edge whose row is gone (none is today: erasure folds, it does not
--      delete) is withheld, never served on a guess.
--      Resources, blocks and blobs are not filtered. Every event about them is written while the
--      subject lives in the context, so the reader had standing to see it then, and their
--      departures must stay served.
--      The test runs before the page bound, so a page is never short because of it, and `has_more`
--      and `through` count only what is served.
--
--   2. webhook_received IS NOT A CHANGE TO THE CONTEXT. It is a domain kind anchored on a
--      connection's home context (20260819000020), so every reader of that context was shown each
--      inbound webhook: when, through which emitter, under which act. Elsewhere only subscription
--      managers can read a receipt (delivery_service). _feed_withholds_kind names it, and is the one
--      definition: context_changes_after skips the kind, and the service refuses it in `kinds` as an
--      unknown kind, as it refuses admin kinds.
--
-- context_changes_after's signature and result are unchanged. Its subject derivation moves from
-- the shaping step into the page, so the edge test can read it before the bound; the derivation
-- itself is the one 20261117000000 wrote, unchanged.
--
-- Cost: the visible-resource set is computed at most once per read, and only when the page meets
-- an edge subject whose endpoint is a resource (a MATERIALIZED CTE read from a CASE arm). A read
-- that meets no edge pays nothing for it.

CREATE FUNCTION _feed_withholds_kind(p_kind text)
RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT p_kind = 'webhook_received'
$$;

COMMENT ON FUNCTION _feed_withholds_kind(text) IS
  'A domain kind the context change feed never serves and refuses in `kinds` (20261117000020). '
  'Today only webhook_received: an inbound receipt is no change to what lives in the context.';

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
               et.name AS kind, subj.t, subj.id
          FROM kb_events e
          JOIN kb_event_types et ON et.id = e.event_type_id
          CROSS JOIN gate g
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
          ) subj
         WHERE g.ok
           AND p_after_xact IS NOT NULL
           AND e.producing_anchor_table = 'kb_contexts'
           AND e.producing_anchor_id = p_context
           AND e.xact_id IS NOT NULL
           AND e.xact_id < g.h
           AND (e.xact_id, e.seq) > (p_after_xact, coalesce(p_after_seq, 0))
           AND e.category = 'domain'
           AND (p_kinds IS NULL OR et.name = ANY (p_kinds))
           AND NOT _feed_withholds_kind(et.name)
           AND NOT _event_is_admin_act(et.name, e.correlation_id, e.emitter_entity_id, e.xact_id)
           AND (subj.t IS DISTINCT FROM 'kb_edges' OR EXISTS (
                   SELECT 1
                     FROM kb_edges ed
                    WHERE ed.id = subj.id
                      AND CASE ed.source_table
                              WHEN 'kb_resources' THEN ed.source_id IN (SELECT resource_id FROM vis)
                              ELSE endpoint_readable_by_profile(p_profile, ed.source_table, ed.source_id)
                          END
                      AND CASE ed.target_table
                              WHEN 'kb_resources' THEN ed.target_id IN (SELECT resource_id FROM vis)
                              ELSE endpoint_readable_by_profile(p_profile, ed.target_table, ed.target_id)
                          END))
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
                   'subject',     jsonb_build_object('table', s.t, 'id', s.id),
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

SELECT declare_migration(
    20261117000020,
    'additive',
    'Adds _feed_withholds_kind(text). Replaces context_changes_after with its signature and result type unchanged: it now withholds webhook_received and any entry whose subject is an edge the reader cannot see both endpoints of. Nothing an older binary reads is dropped, renamed or reshaped; an older binary that names webhook_received in `kinds` is answered with no such entries.'
);
