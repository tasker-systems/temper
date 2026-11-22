-- The per-edge unit that folds and erases one edge homed in an estate.
--
-- An edge homed in a context of the estate that no resource erasure reached keeps its label, its
-- facets and its ledger text after the person act. This migration adds the per-edge unit that
-- erases one such edge under the `edge` authority (20261202000020_edge_erasure_authority.sql).
-- Only tests call it: the person act does not.
-- TODO(person-act step): the person act calls edge_erasure_execute for each such edge.
--
--   1. _edge_erasure_apply_redaction(uuid, uuid): the projection body for one edge. Its label is
--      set to NULL and every property row it owns, live and folded, to erased-key-<n> / "erased"
--      and folded: step (9d) of _resource_erasure_apply_redaction
--      (20261127000010_block_content_hash_follows_redaction.sql) for this one edge. Outside
--      replay it runs only under an edge_erased record of that edge from its own transaction.
--   2. edge_erasure_execute(uuid, uuid, uuid, uuid, uuid): the unit. It locks the edge, refuses an
--      edge not homed in the estate (_edge_homed_in_estate), derives the ledger redaction, folds
--      the edge when it is live, appends one edge_erased record, projects its rows, runs the body,
--      then rewrites the edge's trail in one UPDATE.
--   3. _resource_write_guard_owner(text, uuid): the kb_edges arm also takes FOR KEY SHARE on the
--      edge row and refuses a write to an erased edge (_edge_is_erased, 20261202000020).
--   4. sensitivity.erased_place_findings(): admits the label and edge-owned property findings of
--      an erased edge (_edge_is_erased), so their digests expire.
--   5. _project_relationship_folded(uuid, jsonb): takes FOR KEY SHARE on the edge row and refuses
--      a fold of an erased edge (_edge_is_erased).
--   6. idx_kb_properties_edge_owner: the kb_edges-owned property rows by owner, live and folded,
--      for the readers of an edge's folded rows (the body, _edge_projection_erased in
--      20261202000040, the unit's already-erased test). kb_properties' owner index is partial on
--      NOT is_folded.
--
-- Sections 3 to 5 pin search_path = public, pg_temp; each is plpgsql or, for
-- erased_place_findings, read whole by its one caller, so no caller loses an inlined body.
--
-- A later edit must not break:
--   * derive before fold: _edge_erasure_payload_redaction runs before the fold event exists and
--     before the body rewrites the projection, as resource_erasure_execute's derivation does;
--   * the record follows the fold, and the trail UPDATE follows the body: the verifier admits an
--     `edge` rewrite only of a folded edge, homed in the estate the record names;
--   * the body reads no payload but, outside replay, its record's subject, so replay can call it
--     at the record's position;
--   * a refusal raises before anything is appended;
--   * each write verb of an edge takes FOR KEY SHARE on the edge row before it writes, so it
--     commits before the unit's derivation or waits for the act and is then refused: the property
--     writers (property_set, facet_set, the keyed assert, property_retracted), relationship_retype
--     and relationship_reweight through _resource_write_guard_owner, after the edge's resource
--     endpoints (Section 3); relationship_fold in _project_relationship_folded (Section 5).
--     relationship_corrected and relationship_decayed have no write path.
--   * an assertion of the edge's identity (uq_kb_edges_assertion) is not refused, by design.
--     relationship_assert's active-edge lookup (NOT is_folded, FOR UPDATE) waits on the unit's row
--     lock. When the unit commits, the READ COMMITTED re-check finds the edge folded with a NULL
--     label, so the lookup answers none. The assertion then inserts a new live edge at the same
--     home: its own minted id, the label its own payload carries, and its own relationship_asserted
--     event. The erased edge keeps its fold, its NULL label, its sentinels and its rewritten trail.
--     The new event is outside that trail: it names the new edge, and _edge_phantom_reassertions
--     reads only events whose edge_id has no row. The label is the asserter's own text, so this is
--     a fresh write. docs/concepts/erasure-and-terms-of-use.md rules that "a fresh commit into a
--     retired home mints live, lawful content, and any later erasure of the same estate strikes it
--     again". This unit retires nothing. The Rust doors check container authorship
--     (context_authorable_by_profile, which requires an active context) before the wait, so only
--     an assertion that passed them while the home was live gets here.

-- ---------------------------------------------------------------------------
-- Section 1. The projection body for one edge.
-- ---------------------------------------------------------------------------
-- Every kb_edges-owned row is folded, the uq_kb_properties_active reason
-- _resource_erasure_apply_redaction gives: two live rows of one key would map to one
-- (erased-key-<n>, "erased") pair. Keys are numbered within the edge by
-- _resource_erasure_key_numbers, in ledger order of first appearance.
--
-- The body erases an edge's projection whatever the edge, so it runs only for the record that
-- authorises it: an edge_erased naming p_edge, appended in this transaction (created = now(), the
-- tie kb_events_append_only reads), whose commit edge_erasure_requires_an_erased_owner
-- (20261202000040) then holds to a folded edge of a tombstoned estate. Replay applies a committed
-- record at its span's end, with temper.replaying on.
CREATE FUNCTION _edge_erasure_apply_redaction(p_edge uuid, p_event uuid)
RETURNS void
LANGUAGE plpgsql
SET search_path TO 'public', 'pg_temp'
AS $function$
BEGIN
    IF current_setting('temper.replaying', true) IS DISTINCT FROM 'on'
       AND NOT EXISTS (
           SELECT 1
             FROM kb_events e
             JOIN kb_event_types t ON t.id = e.event_type_id
            WHERE e.id = p_event
              AND t.name = 'edge_erased'
              AND e.payload ->> 'subject_table' = 'kb_edges'
              AND e.payload ->> 'subject_id' = p_edge::text
              AND e.created = now()) THEN
        RAISE EXCEPTION '_edge_erasure_apply_redaction: edge % has no edge_erased record of this transaction',
            p_edge;
    END IF;

    UPDATE kb_edges SET label = NULL WHERE id = p_edge;

    UPDATE kb_properties p
       SET property_key   = 'erased-key-' || k.n::text,
           property_value = '"erased"'::jsonb,
           is_folded      = true,
           last_event_id  = COALESCE(p_event, p.last_event_id)
      FROM kb_edges e
     CROSS JOIN LATERAL _resource_erasure_key_numbers('kb_edges', e.id) k
     WHERE e.id = p_edge
       AND p.owner_table = 'kb_edges' AND p.owner_id = e.id
       AND p.property_key = k.property_key;
END;
$function$;

COMMENT ON FUNCTION _edge_erasure_apply_redaction(uuid, uuid) IS
$c$The projection side of erasing one edge: its label set to NULL, and every kb_properties row it
owns, live and folded, set to erased-key-<n> (numbered within the edge by
_resource_erasure_key_numbers), value "erased", folded, last_event_id p_event. Step (9d) of
_resource_erasure_apply_redaction for one edge. Outside replay (temper.replaying not 'on') raises
unless p_event is an edge_erased record naming p_edge (subject_table kb_edges, subject_id) appended
in this transaction. Reads no other payload, so edge_erasure_execute and replay run the same body
at the edge_erased record's position.$c$;

-- ---------------------------------------------------------------------------
-- Section 2. The unit.
-- ---------------------------------------------------------------------------
-- The edge row lock serializes the unit against a second unit and against each write verb of the
-- edge. A second unit reads the first's committed state and raises `already erased`. The property
-- writers, relationship_retype and relationship_reweight take FOR KEY SHARE on the row in
-- _resource_write_guard_owner (Section 3), and relationship_fold in _project_relationship_folded
-- (Section 5): each either committed before this lock, so the derivation below sees its event, or
-- waits for the act to commit and is then refused, the edge being named by this unit's edge_erased
-- record. An edge
-- already folded is erased without a new fold, and its record says so (folded false). The record
-- carries `folded` in both cases.
CREATE FUNCTION edge_erasure_execute(p_edge uuid, p_estate_of uuid, p_operator uuid,
                                     p_emitter uuid, p_request_ref uuid)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path TO 'public', 'pg_temp'
SET lock_timeout TO '0'
AS $function$
DECLARE
    v_found      boolean;
    v_was_folded boolean;
    v_label      text;
    v_home_table text;
    v_home_id    uuid;
    v_redact     jsonb;
    v_fields     jsonb;
    v_fold       jsonb;
    v_ev         uuid;
    v_payload    jsonb;
BEGIN
    IF p_edge IS NULL THEN
        RAISE EXCEPTION 'edge_erasure_execute: p_edge is required';
    END IF;
    -- The request reference is the act's correlation id: every event the unit appends carries it.
    IF p_request_ref IS NULL THEN
        RAISE EXCEPTION 'edge_erasure_execute: p_request_ref is required';
    END IF;

    SELECT true, e.is_folded, e.label, e.home_anchor_table, e.home_anchor_id
      INTO v_found, v_was_folded, v_label, v_home_table, v_home_id
      FROM kb_edges e
     WHERE e.id = p_edge
       FOR UPDATE;
    IF v_found IS NULL THEN
        RAISE EXCEPTION 'edge_erasure_execute: edge % not found', p_edge;
    END IF;
    IF NOT _edge_homed_in_estate(p_edge, p_estate_of) THEN
        RAISE EXCEPTION 'edge_erasure_execute: edge % is not homed in the estate of %',
            p_edge, p_estate_of;
    END IF;

    -- The ledger redaction, derived before the fold event exists and before the body rewrites
    -- the projection.
    SELECT coalesce(jsonb_agg(jsonb_build_object('event', d.event_id, 'payload', d.new_payload,
                                                 'metadata', d.new_metadata, 'paths', d.paths)
                               ORDER BY d.event_id), '[]'::jsonb)
      INTO v_redact
      FROM _edge_erasure_payload_redaction(p_edge) d;
    SELECT coalesce(jsonb_agg(jsonb_build_object('event', r->'event', 'paths', r->'paths')
                               ORDER BY r->>'event'), '[]'::jsonb)
      INTO v_fields
      FROM jsonb_array_elements(v_redact) r;

    IF v_was_folded
       AND v_label IS NULL
       AND jsonb_array_length(v_redact) = 0
       AND NOT EXISTS (
               SELECT 1 FROM kb_properties p
                WHERE p.owner_table = 'kb_edges' AND p.owner_id = p_edge
                  AND p.property_key !~ '^erased-key-[1-9][0-9]*$') THEN
        RAISE EXCEPTION 'edge_erasure_execute: already erased';
    END IF;

    -- A live edge is folded by the incumbent verb, at its home, reason the fixed literal
    -- 'estate_erased'.
    IF NOT v_was_folded THEN
        v_fold := jsonb_build_object('edge_id', p_edge, 'reason', 'estate_erased');
        v_ev := _event_append('relationship_folded', p_emitter, v_home_table, v_home_id, v_fold,
                              p_correlation => p_request_ref);
        PERFORM _project_relationship_folded(v_ev, v_fold);
    END IF;

    v_payload := jsonb_build_object(
            'subject_table', 'kb_edges',
            'subject_id', p_edge,
            'actor', p_operator,
            'estate_of', p_estate_of,
            'folded', NOT v_was_folded,
            'redacted_fields', v_fields);
    v_ev := _event_append(
        'edge_erased', p_emitter, NULL, NULL, v_payload,
        p_references => jsonb_build_array(
            jsonb_build_object('rel','subject',
                'target', jsonb_build_object('kind','kb_edges','id', p_edge)),
            jsonb_build_object('rel','request',
                'target', jsonb_build_object('kind','kb_events','id', p_request_ref))),
        p_correlation => p_request_ref);

    PERFORM _project_edge_erased_redactions(v_ev, v_payload);

    PERFORM _edge_erasure_apply_redaction(p_edge, v_ev);

    -- kb_events_append_only admits each row against the record's redaction rows, the folded edge's
    -- home in the named estate and the class sentinels; kb_events_redaction_in_trail checks the
    -- edge's trail once for the statement.
    UPDATE kb_events e
       SET payload  = r->'payload',
           metadata = r->'metadata'
      FROM jsonb_array_elements(v_redact) r
     WHERE e.id = (r->>'event')::uuid;

    RETURN jsonb_build_object(
        'event_id',        v_ev,
        'folded',          NOT v_was_folded,
        'redacted_fields', v_fields);
END;
$function$;

COMMENT ON FUNCTION edge_erasure_execute(uuid, uuid, uuid, uuid, uuid) IS
$c$Erase one edge homed in a context of _erasure_estate_contexts(p_estate_of), under the `edge`
authority. Locks the edge row; raises when it is missing, not homed in that estate, or already
erased (folded, no label, nothing left to rewrite on its trail, every owned property key at its
sentinel). Derives _edge_erasure_payload_redaction, folds a live edge (relationship_folded, reason
estate_erased), appends one edge_erased record (subject, actor, estate_of, folded, redacted_fields;
references subject and request), projects its rows, runs _edge_erasure_apply_redaction, then
rewrites the derived events in one UPDATE. Every event carries p_request_ref as its correlation.
Returns {event_id, folded, redacted_fields}.$c$;

-- ---------------------------------------------------------------------------
-- Section 3. A writer of an edge waits on the unit, then is refused.
-- ---------------------------------------------------------------------------
-- 20260929040730's body; only the kb_edges arm changes. After guarding the edge's resource
-- endpoints it takes FOR KEY SHARE on the kb_edges row, which conflicts with the unit's FOR UPDATE:
-- a property, facet or relationship writer of the edge commits before the unit derives its trail,
-- or waits for the act to commit. The lock returns the row's is_folded, and a wait returns the
-- version the act committed (the READ COMMITTED re-check), so a writer that waited reads the fold.
-- Only a folded edge can be erased (a record commits only for one), so only then does the writer
-- read, in a later statement and so a later snapshot, the edge's edge_erased record
-- (_edge_is_erased), and is refused; a write of a live edge never reads the ledger. The endpoints
-- are locked before the edge because the resource act takes its resource FOR UPDATE before the
-- edges it folds: the reverse order would deadlock a writer against that act.
--
-- One deadlock this lock admits and no writer retries: a transaction that writes the properties
-- of two edges in descending id order, against an act that takes the same two edges FOR UPDATE in
-- ascending id order, each holding one row the other waits on. Postgres aborts one side with
-- 40P01. A person act retries a 40P01 (resource_erasure_service's is_retryable_act_error); a
-- property, facet or relationship writer has no retry and answers the incumbent 500 for a lost
-- race (temper-services backend::write_floor), so its request fails once, and when sent again it
-- succeeds or, the edge erased, is refused.
CREATE OR REPLACE FUNCTION _resource_write_guard_owner(p_table text, p_id uuid)
RETURNS void LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_resource uuid;
    v_folded   boolean;
BEGIN
    -- The owner tables kb_properties admits (kb_properties_owner_table_check) and the edge
    -- endpoint tables (kb_edges_{source,target}_table_check). kb_cogmaps and kb_blobs write
    -- into no resource, so they fall through unguarded.
    IF p_table = 'kb_resources' THEN
        PERFORM _resource_write_guard(p_id);
    ELSIF p_table = 'kb_edges' THEN
        FOR v_resource IN
            SELECT e.source_id FROM kb_edges e
             WHERE e.id = p_id AND e.source_table = 'kb_resources'
            UNION
            SELECT e.target_id FROM kb_edges e
             WHERE e.id = p_id AND e.target_table = 'kb_resources'
            ORDER BY 1
        LOOP
            PERFORM _resource_write_guard(v_resource);
        END LOOP;
        -- Replay projects a lawful write whose event sorts after the edge's edge_erased record,
        -- as _resource_write_guard lets one through after a resource_erased.
        IF current_setting('temper.replaying', true) IS DISTINCT FROM 'on' THEN
            SELECT e.is_folded INTO v_folded FROM kb_edges e WHERE e.id = p_id FOR KEY SHARE;
            IF v_folded AND _edge_is_erased(p_id) THEN
                RAISE EXCEPTION 'edge % is erased; writes are refused', p_id;
            END IF;
        END IF;
    ELSIF p_table = 'kb_content_blocks' THEN
        PERFORM _resource_write_guard(
            (SELECT b.resource_id FROM kb_content_blocks b WHERE b.id = p_id));
    END IF;
END;
$$;

COMMENT ON FUNCTION _resource_write_guard_owner(text, uuid) IS
'the write guard by owner (spec 2026-09-28 D13): resolves an owner to the resource or resources it
writes into and guards each — kb_resources itself; kb_edges each endpoint whose table is
kb_resources; kb_content_blocks the block''s resource_id; kb_cogmaps (and a kb_blobs endpoint)
nothing. For kb_edges it then takes FOR KEY SHARE on the edge row, which waits out
edge_erasure_execute''s FOR UPDATE, and RAISEs ''edge % is erased; writes are refused'' when the
row it locked is folded and _edge_is_erased answers true for the edge; a live edge is never looked
up on the ledger. Neither the edge lock nor that refusal runs while
temper.replaying is ''on''. A writer of two edges in descending id order can deadlock against an
act that locks them ascending; the writer has no retry, so that request fails once with a 500.';

-- ---------------------------------------------------------------------------
-- Section 4. An erased edge's label and property findings give up their digests.
-- ---------------------------------------------------------------------------
-- 20261011100000's body, plus two arms: a kb_edges.label finding, or a kb_properties.property_key /
-- property_value finding on an edge-owned row, whose edge is erased (_edge_is_erased). Such an
-- edge's endpoints are not erased resources, so the label and property arms that key on an erased
-- endpoint never hold it.
CREATE OR REPLACE FUNCTION sensitivity.erased_place_findings() RETURNS TABLE (finding_id uuid)
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
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
    -- An edge's label, where an edge_erased record names the edge (_edge_is_erased).
    SELECT f.id FROM f
     WHERE f.surface = 'kb_edges.label'
       AND _edge_is_erased(f.target_id)
    UNION
    -- An edge-owned property's key or value, on such an edge.
    SELECT f.id FROM f
      JOIN kb_properties p ON p.id = f.target_id AND p.owner_table = 'kb_edges'
     WHERE f.surface IN ('kb_properties.property_key', 'kb_properties.property_value')
       AND _edge_is_erased(p.owner_id)
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
    -- A ledger path an erasure act rewrote to its sentinel (kb_event_field_redactions):
    -- place_closure closes it under an `erasure` row, a `scrub` row the event still holds, or an
    -- `edge` row whose folded edge is homed in the estate its record names.
    SELECT f.id FROM f
     WHERE f.surface IN ('kb_events.payload', 'kb_events.metadata')
       AND sensitivity.place_closure(f.surface, f.target_id, f.content_hash, f.path) = 'sentinel';
$$;

COMMENT ON FUNCTION sensitivity.erased_place_findings() IS
'The unexpired findings, not yet in sensitivity.erased_closures, on a place an erasure act emptied or
reached: any place attributed to an erased resource; an edge''s label, or an edge-owned property''s
key or value, on an edge with an erased resource at either end or named as the subject of an
edge_erased record; block, chunk and header content place_closure answers content_empty; a remote
source row that no longer exists; and a ledger path place_closure answers sentinel.
sensitivity.expire_erased_fingerprints reads it to start each finding''s window.';


-- ---------------------------------------------------------------------------
-- Section 5. A fold of an erased edge waits on the unit, then is refused.
-- ---------------------------------------------------------------------------
-- 20261201000030's body, plus the lock and the refusal after the existence check. The fold does not
-- go through _resource_write_guard_owner: the resource act folds the edges of a resource it has
-- already erased, and the guard's endpoint arm would refuse those folds. Its callers and why none is
-- refused wrongly: the unit's own fold runs before its record exists; the resource act and the
-- block history scrub fold only live edges, which no record names; _erasure_member_flag calls
-- _edge_fold_apply directly; replay runs with temper.replaying on, which skips both. The lock and
-- the check are two statements, so a fold that waited reads the record in a later snapshot.
--
-- 20260929040730 leaves this projector unguarded on purpose: a fold writes no content, and the act
-- itself folds. An erased edge is the exception to the first half. relationship_folded carries a
-- caller-written reason and names the edge by edge_id, so it lands on the edge's own trail
-- (_edge_erasure_trail_scope) after the act rewrote that trail: fresh text on the trail of an edge
-- whose erasure is recorded as done. The second half still holds: the unit folds before its record
-- exists, so its own fold is never refused.
--
-- One read of the edge row: outside replay it is the locking read, whose is_folded decides
-- whether the record is looked for (a live edge is never erased, as in
-- _resource_write_guard_owner); under replay a plain read, for the existence check alone.
CREATE OR REPLACE FUNCTION _project_relationship_folded(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_edge   uuid := (p_payload->>'edge_id')::uuid;
    v_folded boolean;
BEGIN
    IF current_setting('temper.replaying', true) IS DISTINCT FROM 'on' THEN
        SELECT e.is_folded INTO v_folded FROM kb_edges e WHERE e.id = v_edge FOR KEY SHARE;
    ELSE
        SELECT e.is_folded INTO v_folded FROM kb_edges e WHERE e.id = v_edge;
    END IF;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'relationship_fold: edge % not found', v_edge;
    END IF;
    IF current_setting('temper.replaying', true) IS DISTINCT FROM 'on'
       AND v_folded AND _edge_is_erased(v_edge) THEN
        RAISE EXCEPTION 'edge % is erased; writes are refused', v_edge;
    END IF;
    PERFORM _edge_fold_apply(p_event, v_edge);
    RETURN v_edge;
END;
$$;

COMMENT ON FUNCTION _project_relationship_folded(uuid, jsonb) IS
$c$The relationship_folded projection: reads the edge row once, raising when it is missing; outside
replay (temper.replaying not 'on') that read takes FOR KEY SHARE, which waits out
edge_erasure_execute's FOR UPDATE, and raises 'edge % is erased; writes are refused' when the row
is folded and _edge_is_erased answers true for the edge (a live edge is never looked up on the
ledger); then folds it through _edge_fold_apply. Guarded, unlike the fold 20260929040730 leaves
unguarded, because a fold's reason is text on the erased edge's own trail.$c$;

-- ---------------------------------------------------------------------------
-- Section 6. The folded rows an edge owns, by owner.
-- ---------------------------------------------------------------------------
-- idx_kb_properties_owner is WHERE NOT is_folded, so a read of an edge's folded rows by owner
-- scanned kb_properties. Keyed on owner_id alone under the owner_table predicate, which each
-- reader spells. Built in the migration's transaction, not CONCURRENTLY: no migration in this
-- repo uses sqlx's no-transaction escape (20261117000000_context_change_feed_doors.sql). The build
-- holds SHARE on kb_properties, which queues its writers, not its readers, for one scan, under SET
-- LOCAL lock_timeout = '500ms', the bound and the reasons of
-- 20261201000010_erasure_records_schema.sql: when it fires the migration rolls back, the serving
-- deployment keeps its schema, and the next deploy retries.
SET LOCAL lock_timeout = '500ms';

CREATE INDEX idx_kb_properties_edge_owner
    ON kb_properties (owner_id)
    WHERE owner_table = 'kb_edges';

COMMENT ON INDEX idx_kb_properties_edge_owner IS
$c$Every kb_properties row an edge owns, live and folded, by owner_id. Serves the readers of an
edge's folded rows: _edge_erasure_apply_redaction, _edge_projection_erased and
edge_erasure_execute's already-erased test.$c$;

SELECT declare_migration(
    20261202000030,
    'additive',
    'Adds the functions _edge_erasure_apply_redaction(uuid, uuid), which outside replay runs only '
    'for an edge_erased record of its edge appended in the same transaction, and '
    'edge_erasure_execute(uuid, uuid, uuid, uuid, uuid), each with a COMMENT; nothing existing '
    'calls them. CREATE OR REPLACEs, at unchanged signatures, _resource_write_guard_owner(text, '
    'uuid), whose kb_edges arm also takes FOR KEY SHARE on the edge row and refuses a write to an '
    'edge an edge_erased record names (_edge_is_erased), and sensitivity.erased_place_findings(), which gains '
    'an arm for findings on such an edge; it also gains a COMMENT. CREATE OR REPLACEs '
    '_project_relationship_folded(uuid, jsonb) at its signature, which takes the same lock and '
    'refuses a fold of an edge an edge_erased record names, and gives it a COMMENT. No edge_erased '
    'record exists '
    'before edge_erasure_execute runs, so every existing answer is unchanged, and the added '
    'KEY SHARE conflicts only with a FOR UPDATE, a DELETE or an id change of the edge row (its one '
    'non-partial unique key is the primary key). The three replaced functions now pin '
    'search_path = public, pg_temp, and every name they read is in public or schema-qualified. '
    'Adds the partial index idx_kb_properties_edge_owner on kb_properties (owner_id) WHERE '
    'owner_table = ''kb_edges'', built in the transaction (SHARE on kb_properties for one scan) '
    'under SET LOCAL lock_timeout = 500ms: if its lock is not granted in time, the migration rolls '
    'back and the serving deployment is untouched. No table, column, '
    'constraint, event type or grant changes.'
);
