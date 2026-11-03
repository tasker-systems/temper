-- The ledger records the order it was written in, and a context's changes are readable after a
-- position.
--
-- Ruling R5 (temper decision 01a1a1ad-b0ae-7562-88ea-2035b238d517, 2026-11-03), for task
-- 01a0f03d-0bef-7d70-9df3-a6d98b8cedaf. The read below is the substrate half of the change feed
-- (goal 01a1a1a2-e253-759c-8c82-3411a8b4e010, task 01a1a1a5-773a-7599-a157-6128705ef71e). The build's
-- forks F1–F6 are in temper-artifacts plans/2026-11-03-context-change-feed.md.
--
-- No row carried commit order before this migration:
--   - `id` is minted at insert by uuid_generate_v7(). On PG17 that is pg_uuidv7, which has no order
--     within a millisecond.
--   - `occurred_at` is the transaction's start time.
-- So a transaction that starts first and commits last puts rows behind rows a reader has already
-- passed.
--
--   1. xact_id (the writing transaction's 64-bit id) and seq (append order, from a sequence, monotonic
--      within one backend). Both are filled by column defaults, so every inserter gets them with no
--      code change. Rows that existed before this migration keep NULL in both. Their order within a
--      transaction was never recorded, and no value is invented for it: `id` is the very thing found
--      out of order, and `ctid` and `xmin` are proxies (the category backfill in 20260719000010
--      rewrote xmin on every admin- and system-category row).
--   2. A BEFORE INSERT trigger. Outside replay, a row must carry both values, and xact_id must be the
--      inserting transaction's own id. An explicit older id would plant a row behind a position a
--      reader has passed. Replay restores recorded rows verbatim under the transaction-local
--      `temper.replaying`, the flag `_resource_write_guard` already honours.
--   3. context_changes_after(): a context's domain events after a position, under the settled
--      horizon. It serves only rows whose transaction is below pg_snapshot_xmin of the snapshot that
--      scans them. Every such transaction has committed or aborted, and every transaction still
--      running, or not yet started, has an id at or above it. So no commit after the read can sort at
--      or before the position it returns. The cost is lag: a transaction holding an id anywhere in
--      the cluster holds the horizon until it ends. It never causes a skip. An entry is served only
--      when the reader can read its subject now, not merely the context (see the function's comment).

-- ---------------------------------------------------------------------------
-- Section 1. The order columns.
-- ---------------------------------------------------------------------------
CREATE SEQUENCE kb_events_seq AS bigint;

-- Added without defaults, and the defaults set after: existing rows stay NULL and the table is not
-- rewritten.
ALTER TABLE kb_events ADD COLUMN xact_id xid8, ADD COLUMN seq bigint;
ALTER TABLE kb_events
    ALTER COLUMN xact_id SET DEFAULT pg_current_xact_id(),
    ALTER COLUMN seq     SET DEFAULT nextval('kb_events_seq');
ALTER SEQUENCE kb_events_seq OWNED BY kb_events.seq;

ALTER TABLE kb_events
    ADD CONSTRAINT kb_events_order_both_or_neither CHECK ((xact_id IS NULL) = (seq IS NULL));

COMMENT ON COLUMN kb_events.xact_id IS
  'The top-level id of the transaction that appended this row (R5). NULL only for rows that predate '
  '20261116000010: their transaction was not recorded. The change feed orders by (xact_id, seq) and '
  'serves only rows below the settled horizon.';
COMMENT ON COLUMN kb_events.seq IS
  'Append order (R5). Within one transaction, seq order is the order the act wrote its events. Across '
  'transactions, seq alone is not commit order: (xact_id, seq) under the horizon is. NULL only for '
  'rows that predate 20261116000010, whose order within a transaction was never recorded.';

-- The feed's scan: one context's recorded rows, in feed order.
CREATE INDEX idx_kb_events_anchor_feed_order
    ON kb_events (producing_anchor_table, producing_anchor_id, xact_id, seq)
    WHERE xact_id IS NOT NULL;

-- A position names one row. Two rows sharing (xact_id, seq) would let a page cut between them skip
-- the second, and seq can be set explicitly. So the pair is unique.
CREATE UNIQUE INDEX kb_events_feed_position_unique
    ON kb_events (xact_id, seq)
    WHERE xact_id IS NOT NULL;

-- ---------------------------------------------------------------------------
-- Section 2. Every new row records its order, truthfully.
-- ---------------------------------------------------------------------------
CREATE FUNCTION kb_events_order_recorded() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF current_setting('temper.replaying', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF NEW.xact_id IS NULL OR NEW.seq IS NULL THEN
        RAISE EXCEPTION 'kb_events: a new row must record xact_id and seq (R5)'
            USING ERRCODE = 'check_violation';
    END IF;
    IF NEW.xact_id <> pg_current_xact_id() THEN
        RAISE EXCEPTION 'kb_events: xact_id % is not the inserting transaction''s own id (R5)', NEW.xact_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER kb_events_order_recorded
    BEFORE INSERT ON kb_events
    FOR EACH ROW EXECUTE FUNCTION kb_events_order_recorded();

-- ---------------------------------------------------------------------------
-- Section 3. The read.
-- ---------------------------------------------------------------------------
-- A payload id, or NULL when the value is not a uuid. A malformed id must not fail the whole read, and
-- must never be served as text.
CREATE FUNCTION _feed_uuid(p text) RETURNS uuid LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE WHEN p ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                THEN p::uuid END
$$;

-- A page bound outside 1..1000 (F6) is refused, never clamped: a caller asking for more must not
-- silently believe it got it. VOLATILE, so the planner never folds the refusal ahead of the
-- context gate: a caller who cannot read the context is answered NULL, whatever bound it sent.
CREATE FUNCTION _feed_refuse_limit(p_limit int) RETURNS boolean LANGUAGE plpgsql VOLATILE AS $$
BEGIN
    RAISE EXCEPTION 'context_changes_after: page bound % is outside 1..1000', p_limit
        USING ERRCODE = 'invalid_parameter_value';
END;
$$;

-- context_changes_after(profile, context, after_xact, after_seq, limit, kinds) → jsonb
--
-- Returns NULL when the profile cannot read the context now (goal: the-feed-reads-under-standing-now).
-- The caller refuses that exactly as it refuses a read of the context. Otherwise it returns
--   {"entries": [...], "through": {"xact": text, "seq": bigint}, "has_more": bool}.
--
-- A NULL after_xact is a read with no position (F5): no entries, and `through` at the current horizon.
-- `through` is the last entry served when the bound cut the page. Otherwise it is (horizon, 0). It never
-- moves back from the position given.
--
-- STABLE and a single SQL statement, so the horizon and the scan share one snapshot. Rule R5 rests on
-- that: a transaction below the horizon must be visible, or have aborted, in the snapshot that scans.
--
-- The context gate is not the whole gate. An event anchored on a readable context can be about a
-- subject the reader cannot read now: an edge homed here whose other endpoint is private, a resource
-- that has since moved to a context the reader cannot read, a deleted resource, a struck blob. Every
-- entry is served only when its subject passes that subject's own read predicate, evaluated now:
--   kb_resources       resources_visible_to (which carries the soft-delete floor)
--   kb_edges           the edge's home and both endpoints readable: endpoint_readable_by_profile and
--                      anchor_readable_by_profile, the authorization half of edges_visible_to. Its
--                      currency half (NOT is_folded) is left out on purpose: a retracted edge between
--                      readable endpoints is a change the reader may see.
--   kb_content_blocks  the block's resource, as kb_resources
--   kb_blobs           blob_readable_by_profile (which carries the strike)
--   kb_contexts        this context: the gate above
-- Any other subject table is refused (fail-closed). The filter runs before the page bound, so a
-- withheld row never sets `through` and never counts toward `has_more`: nothing in the answer moves
-- because of a row the reader cannot read. The position a reader holds therefore says nothing about
-- withheld rows, and a forged one discloses only what the filter would serve from it.
--
-- The cost, ruled 2026-11-03 for task 01a1a1a5-773a-7599-a157-6128705ef71e: once a resource is
-- deleted or moves out of the reader's view, none of its entries is served, its deletion included.
-- A relay holding a claim on it is not told it went.
--
-- An entry is built from an allowlist (R4, goal: an-entry-never-carries-content). No payload value is
-- copied whole. A property kind names its key, taken from the asserting event's payload. That payload
-- is the one erasure rewrites, and its kb_event_field_redactions row marks the rewrite. It never takes
-- the key from kb_properties. A key a redaction covers is served as `"key": null, "redacted": true`.
CREATE FUNCTION context_changes_after(
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
    -- Computed once per call. MATERIALIZED, because the subject filter references it per row.
    vis AS MATERIALIZED (
        SELECT v.resource_id FROM gate g CROSS JOIN LATERAL resources_visible_to(p_profile) v WHERE g.ok
    ),
    candidate AS (
        SELECT e.id, e.xact_id, e.seq, e.emitter_entity_id, e.occurred_at, e.correlation_id,
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
    ),
    subjected AS (
        SELECT c.*, prop.asserted_by_event_id, subj.subject_table, subj.subject_id
          FROM candidate c
          LEFT JOIN kb_properties prop
            ON c.kind LIKE 'property\_%' AND prop.id = _feed_uuid(c.payload->>'property_id')
          CROSS JOIN LATERAL (
              SELECT t.subject_table, t.subject_id FROM (
                  SELECT CASE
                      -- A property payload's owner is an AnchorRef, {table, id}.
                      WHEN c.payload->'owner' ? 'table' THEN c.payload->'owner'->>'table'
                      WHEN prop.id IS NOT NULL THEN prop.owner_table
                      WHEN _feed_uuid(c.payload->>'resource_id') IS NOT NULL THEN 'kb_resources'
                      WHEN _feed_uuid(c.payload->>'edge_id') IS NOT NULL THEN 'kb_edges'
                      WHEN _feed_uuid(c.payload->>'block_id') IS NOT NULL THEN 'kb_content_blocks'
                      WHEN _feed_uuid(c.payload->>'blob_id') IS NOT NULL THEN 'kb_blobs'
                      ELSE 'kb_contexts'
                  END AS subject_table,
                  CASE
                      WHEN c.payload->'owner' ? 'table' THEN _feed_uuid(c.payload->'owner'->>'id')
                      WHEN prop.id IS NOT NULL THEN prop.owner_id
                      WHEN _feed_uuid(c.payload->>'resource_id') IS NOT NULL THEN _feed_uuid(c.payload->>'resource_id')
                      WHEN _feed_uuid(c.payload->>'edge_id') IS NOT NULL THEN _feed_uuid(c.payload->>'edge_id')
                      WHEN _feed_uuid(c.payload->>'block_id') IS NOT NULL THEN _feed_uuid(c.payload->>'block_id')
                      WHEN _feed_uuid(c.payload->>'blob_id') IS NOT NULL THEN _feed_uuid(c.payload->>'blob_id')
                      ELSE p_context
                  END AS subject_id
              ) t
          ) subj
    ),
    page AS (
        SELECT s.* FROM subjected s
         WHERE CASE s.subject_table
                   WHEN 'kb_contexts' THEN s.subject_id = p_context
                   WHEN 'kb_resources' THEN s.subject_id IN (SELECT resource_id FROM vis)
                   WHEN 'kb_content_blocks' THEN EXISTS (
                       SELECT 1 FROM kb_content_blocks b
                        WHERE b.id = s.subject_id AND b.resource_id IN (SELECT resource_id FROM vis))
                   WHEN 'kb_blobs' THEN blob_readable_by_profile(p_profile, s.subject_id)
                   WHEN 'kb_edges' THEN EXISTS (
                       SELECT 1 FROM kb_edges ed
                        WHERE ed.id = s.subject_id
                          AND anchor_readable_by_profile(p_profile, ed.home_anchor_table, ed.home_anchor_id)
                          AND CASE ed.source_table
                                  WHEN 'kb_resources' THEN ed.source_id IN (SELECT resource_id FROM vis)
                                  ELSE endpoint_readable_by_profile(p_profile, ed.source_table, ed.source_id) END
                          AND CASE ed.target_table
                                  WHEN 'kb_resources' THEN ed.target_id IN (SELECT resource_id FROM vis)
                                  ELSE endpoint_readable_by_profile(p_profile, ed.target_table, ed.target_id) END)
                   ELSE false
               END
         ORDER BY s.xact_id, s.seq
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
                   'subject',     jsonb_build_object('table', s.subject_table, 'id', s.subject_id),
                   'actor',       s.emitter_entity_id,
                   'occurred_at', s.occurred_at,
                   'act',         s.correlation_id
               ))
               || CASE WHEN s.kind LIKE 'property\_%' THEN
                      CASE WHEN keyed.redacted THEN jsonb_build_object('key', NULL, 'redacted', true)
                           ELSE jsonb_build_object('key', keyed.key) END
                  ELSE '{}'::jsonb END AS entry
          FROM served s
          -- The event whose payload holds the key: this event when it carries `property_key`, else
          -- the event that asserted the property (property_retracted / _folded / _reweighted).
          LEFT JOIN kb_events kev
            ON s.kind LIKE 'property\_%'
           AND kev.id = CASE WHEN s.payload ? 'property_key' THEN s.id ELSE s.asserted_by_event_id END
          LEFT JOIN LATERAL (
              SELECT kev.payload->>'property_key' AS key,
                     EXISTS (SELECT 1 FROM kb_event_field_redactions r
                              WHERE r.event_id = kev.id AND r.path = 'property_key') AS redacted
          ) keyed ON s.kind LIKE 'property\_%'
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
    20261116000010,
    'additive',
    'Adds the nullable kb_events columns xact_id and seq with defaults (existing rows stay NULL, the table is not rewritten), their sequence, a both-or-neither CHECK, two partial indexes (one UNIQUE on (xact_id, seq)), a BEFORE INSERT trigger that refuses a new row without a truthful order outside replay, and the new functions _feed_uuid, _feed_refuse_limit and context_changes_after. A binary from before this migration inserts events without naming the new columns, so the defaults fill them and the trigger passes; nothing it reads is dropped, renamed or reshaped.'
);
