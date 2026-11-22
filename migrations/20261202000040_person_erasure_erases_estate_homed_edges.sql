-- The person act erases every edge homed in its estate.
--
-- 20261202000030's per-edge unit, edge_erasure_execute, becomes a step of the person act, and the
-- `edge` authority (20261202000020_edge_erasure_authority.sql) holds only for a tombstoned estate
-- owner, checked at commit. By section:
--   1-2. _edge_projection_erased(uuid), the projection half, and _edge_erasure_complete(uuid):
--      whether an edge's text is already erased. The unit raises `already erased` on the same
--      test (edge_erasure_execute, re-issued) and the act's step skips by it.
--   3. edge_erasure_requires_an_erased_owner(), behind two DEFERRABLE INITIALLY DEFERRED
--      constraint triggers (Section 7): the record's estate_of must be tombstoned at commit, and
--      its subject edge folded and homed in that estate (_edge_homed_in_estate).
--   4-6. _principal_erasure_edges_outcome(integer), the record's prose;
--      principal_erasure_survey_plan predicts the step as one kb_edges target and counts every
--      estate-homed edge's trail inside the estate for R8; principal_erasure_execute runs the
--      step, references each edge_erased, and pins lock_timeout = 0.
--   7. The two triggers, the authority column's COMMENT, which names the owner check, and
--      kb_profiles_tombstone_is_irreversible: a set tombstoned_at never changes, so the owner check
--      cannot be met by a tombstone cleared before commit.
--
-- A later edit must not break:
--   * the owner check runs at commit, never in the unit: the person act tombstones its subject
--     after its edge step. It stands down only under temper.replaying;
--   * the person act's edge step runs after every resource erasure, passes over an edge touching
--     a resource the act erased, selects by _edge_erasure_complete under each edge's lock, locks
--     in id order, and skips and counts an edge whose home has left the estate since the plan;
--   * the person act locks its estate contexts after its resource erasures and its edge step,
--     never at its start: a standalone resource erasure takes its resource's act queue before its
--     home context's row, and the reverse order deadlocks the two (the act's context lock comment
--     says how);
--   * an assertion of an edge's identity racing the step mints a new live edge at the same home
--     (20261202000030's header says how): a fresh write, lawful as it stands, which the act that
--     erased the first edge does not erase (ruling 9 of decision
--     01a1f6e8-81bd-758c-be87-a6824b66352a: the write doors do not re-check the home).
--
-- One residual is kept (ruling 12 of that decision): the edge step does not hold an edge's home
-- context. A context_reassign that moves the context out of the estate after the step found an
-- edge to erase homed there, and commits before the act's context lock, makes the act abort: the
-- unit raises `edge % is not homed in the estate of %` when the reassign lands between the step's
-- check and the unit's, and otherwise the commit check refuses the record, its edge no longer
-- homed in the estate. Neither is retried, so the service answers 500; nothing the act did persists, and the
-- act run again completes, its edge step reading the live estate and leaving that context's edges.
--
-- Section 7 runs under SET LOCAL lock_timeout = '500ms', the bound and the reasons of
-- 20261201000010_erasure_records_schema.sql: each CREATE TRIGGER and CREATE CONSTRAINT TRIGGER
-- takes SHARE ROW EXCLUSIVE on its table to commit, and kb_events takes every append. When the bound fires the
-- migration rolls back, the serving deployment keeps its schema, and the next deploy retries.

-- ---------------------------------------------------------------------------
-- Section 1. Whether an edge's text is already erased.
-- ---------------------------------------------------------------------------
-- One test, so the unit's refusal and the act's skip agree. The projection half is its own
-- function: the unit has already derived the ledger rewrite when it asks, so it reads this half
-- and its own derivation, and the derivation runs once there.
CREATE FUNCTION _edge_projection_erased(p_edge uuid)
RETURNS boolean
LANGUAGE sql STABLE
SET search_path TO 'public', 'pg_temp'
AS $function$
    SELECT EXISTS (SELECT 1 FROM kb_edges e
                    WHERE e.id = p_edge AND e.is_folded AND e.label IS NULL)
       AND NOT EXISTS (SELECT 1 FROM kb_properties p
                        WHERE p.owner_table = 'kb_edges' AND p.owner_id = p_edge
                          AND p.property_key !~ '^erased-key-[1-9][0-9]*$');
$function$;

COMMENT ON FUNCTION _edge_projection_erased(uuid) IS
$c$The projection half of _edge_erasure_complete: the edge is folded, its label is NULL, and every
kb_properties row it owns has an erased-key-<n> key. False for a missing edge. edge_erasure_execute
reads it with its own derivation of _edge_erasure_payload_redaction.$c$;

CREATE FUNCTION _edge_erasure_complete(p_edge uuid)
RETURNS boolean
LANGUAGE plpgsql STABLE
SET search_path TO 'public', 'pg_temp'
AS $function$
BEGIN
    -- The projection first: the derivation runs only for an edge whose projection is erased.
    IF NOT _edge_projection_erased(p_edge) THEN
        RETURN false;
    END IF;
    RETURN NOT EXISTS (SELECT 1 FROM _edge_erasure_payload_redaction(p_edge));
END;
$function$;

COMMENT ON FUNCTION _edge_erasure_complete(uuid) IS
$c$Whether one edge's text is already erased: its projection is erased (_edge_projection_erased:
folded, label NULL, every owned key at erased-key-<n>), and _edge_erasure_payload_redaction finds
nothing on its trail to rewrite. False for a missing edge. edge_erasure_execute raises `already
erased` when it holds, and the person act's edge step skips an edge when it holds, so the two
agree.$c$;

-- ---------------------------------------------------------------------------
-- Section 2. The unit reads that test.
-- ---------------------------------------------------------------------------
-- 20261202000030's body and attributes; only the `already erased` test changes, to
-- _edge_projection_erased and the unit's own derivation being empty: _edge_erasure_complete's
-- test, which answers what the inline test answered.
CREATE OR REPLACE FUNCTION edge_erasure_execute(p_edge uuid, p_estate_of uuid, p_operator uuid,
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

    -- _edge_erasure_complete over the derivation just made.
    IF _edge_projection_erased(p_edge) AND jsonb_array_length(v_redact) = 0 THEN
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
erased (_edge_erasure_complete's test). Derives _edge_erasure_payload_redaction, folds a live edge
(relationship_folded, reason estate_erased), appends one edge_erased record (subject, actor,
estate_of, folded, redacted_fields; references subject and request), projects its rows, runs
_edge_erasure_apply_redaction, then rewrites the derived events in one UPDATE. Every event carries
p_request_ref as its correlation. Its transaction commits only if p_estate_of is then tombstoned
(edge_erasure_requires_an_erased_owner). Returns {event_id, folded, redacted_fields}.
principal_erasure_execute calls it for each edge homed in the estate whose text is not yet
erased.$c$;

-- ---------------------------------------------------------------------------
-- Section 3. The edge authority holds only for a tombstoned estate owner, at commit.
-- ---------------------------------------------------------------------------
-- An edge erasure is a step of the person act, so its record's estate_of must name a profile the
-- act has tombstoned: the strength `erasure` has from erased_at and `principal` from tombstoned_at.
-- The person act tombstones its subject after the edge step, so the check is deferred to commit.
-- Two constraint triggers share one function:
--   * on kb_events, every edge_erased record: a unit call outside a person erasure commits nothing,
--     including one over an edge with nothing to redact, which projects no `edge` row;
--   * on kb_event_field_redactions, every `edge` row, read through its redacted_by record: a row
--     inserted by hand under a record from another transaction is held to the same owner.
-- The kb_events trigger's WHEN reads category, which kb_events_category_matches_type ties to the
-- type's (edge_erased is `admin`), so no edge_erased record escapes it; the function confirms the
-- type by name. Both stand down under temper.replaying: replay commits each event in its own
-- transaction, and the tombstone rides in with kb_profiles, an input table, not with the walk.
-- The triggers themselves are created in Section 7, under the lock bound.
--
-- The record's subject must also be, at commit, an edge that is folded and homed in a context of
-- that estate, as the verifier requires of an `edge` rewrite. A record alone makes _edge_is_erased
-- refuse every write to the edge it names, so without this a record naming any edge, under any
-- already tombstoned estate_of, would lock that edge with nothing redacted. The person act commits
-- no record that fails it: its edge step calls the unit only for an edge it finds homed in the live
-- estate under the edge's lock, the unit folds the edge, and no write path unfolds one. The home
-- is read again here, against the estate at commit. Once the act holds its estate contexts' rows
-- (principal_erasure_execute's context lock), no door moves one out; a context reassigned out of
-- the estate after the edge step and before that lock fails the act at commit instead, and the act
-- run again leaves that context's edges, its edge step reading the live estate (the residual this
-- migration's header names).
CREATE FUNCTION edge_erasure_requires_an_erased_owner()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_record    uuid;
    v_is_edge   boolean;
    v_estate_of uuid;
    v_subject   uuid;
BEGIN
    IF current_setting('temper.replaying', true) IS NOT DISTINCT FROM 'on' THEN
        RETURN NULL;
    END IF;
    -- Separate statements: PL/pgSQL resolves a NEW field whenever its expression runs.
    IF TG_TABLE_NAME = 'kb_events' THEN
        v_record := NEW.id;
    ELSE
        v_record := NEW.redacted_by;
    END IF;
    SELECT t.name = 'edge_erased'
      INTO v_is_edge
      FROM kb_events e
      JOIN kb_event_types t ON t.id = e.event_type_id
     WHERE e.id = v_record;
    v_is_edge := coalesce(v_is_edge, false);
    IF TG_TABLE_NAME = 'kb_events' AND NOT v_is_edge THEN
        RETURN NULL;
    END IF;
    -- estate_of and subject_id are read as uuids only from an edge_erased record, whose payload
    -- carries both.
    IF v_is_edge THEN
        SELECT (e.payload ->> 'estate_of')::uuid,
               CASE WHEN e.payload ->> 'subject_table' = 'kb_edges'
                    THEN (e.payload ->> 'subject_id')::uuid END
          INTO v_estate_of, v_subject
          FROM kb_events e
         WHERE e.id = v_record;
        IF NOT EXISTS (SELECT 1 FROM kb_profiles p
                        WHERE p.id = v_estate_of AND p.tombstoned_at IS NOT NULL) THEN
            RAISE EXCEPTION 'edge erasure: the estate''s owner % is not erased', v_estate_of
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'edge_erasure_requires_an_erased_owner';
        END IF;
        -- The record's subject as the verifier admits an `edge` rewrite of it: a record alone, with
        -- no row and no rewrite for the verifier to read, makes _edge_is_erased refuse every write
        -- to the edge it names.
        IF NOT EXISTS (SELECT 1 FROM kb_edges s WHERE s.id = v_subject AND s.is_folded)
           OR NOT _edge_homed_in_estate(v_subject, v_estate_of) THEN
            RAISE EXCEPTION 'edge erasure: edge % is not a folded edge homed in the estate of %',
                v_subject, v_estate_of
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'edge_erasure_requires_an_erased_owner';
        END IF;
        RETURN NULL;
    END IF;
    RAISE EXCEPTION 'edge erasure: the estate''s owner % is not erased', v_estate_of
        USING ERRCODE = 'check_violation', CONSTRAINT = 'edge_erasure_requires_an_erased_owner';
END;
$$;

COMMENT ON FUNCTION edge_erasure_requires_an_erased_owner() IS
$c$At commit (DEFERRABLE INITIALLY DEFERRED constraint triggers
trg_edge_erased_requires_an_erased_owner on kb_events and
trg_edge_redaction_requires_an_erased_owner on kb_event_field_redactions): refuses the transaction
unless the edge_erased record (the inserted event, or an `edge` row's redacted_by) names in
estate_of a profile whose tombstoned_at is set, and names a kb_edges subject that is folded and
homed in a context of that estate (_edge_homed_in_estate). Confirms the type by name before it
reads estate_of. An `edge` row whose redacted_by is no edge_erased record is refused too; another
kind of event on kb_events passes. Raises 'edge erasure: the estate''s owner % is not erased' or
'edge erasure: edge % is not a folded edge homed in the estate of %' (check_violation). Stands down
while temper.replaying is 'on'.$c$;

-- ---------------------------------------------------------------------------
-- Section 4. The record's prose for the edges erased.
-- ---------------------------------------------------------------------------
-- One definition, so the survey's predicted entry and the act's entry read alike.
CREATE FUNCTION _principal_erasure_edges_outcome(p_n integer)
RETURNS text
LANGUAGE sql IMMUTABLE
SET search_path = public, pg_temp
AS $$
    SELECT p_n::text || ' edge(s) homed in the estate erased: folded, labels cleared, edge-owned '
           || 'properties and ledger text sentinel-scrubbed';
$$;

COMMENT ON FUNCTION _principal_erasure_edges_outcome(integer) IS
$c$The outcome text of the person erasure record's kb_edges target: p_n edges homed in the estate
erased by the edge step. principal_erasure_survey_plan writes it with the predicted count and
principal_erasure_execute with the count it erased.$c$;

-- ---------------------------------------------------------------------------
-- Section 5. The plan: the edge step's count, and R8 by home.
-- ---------------------------------------------------------------------------
-- 20261127000010_block_content_hash_follows_redaction.sql's body. It adds the kb_edges target
-- (would_erase_edges); v_trail gains the trail of every edge homed in the estate, folded or
-- not: the edge step rewrites it, so the R8 counts no longer name its labels and property values
-- as kept outside the estate; and a charter's outcome names as kept only its edges homed outside
-- the estate.
CREATE OR REPLACE FUNCTION public.principal_erasure_survey_plan(p_subject uuid)
 RETURNS jsonb
 LANGUAGE plpgsql
 SET search_path TO 'public', 'pg_temp'
AS $function$
DECLARE
    v_exists     uuid;
    v_governed   uuid[] := '{}';
    v_resources  uuid[] := '{}';
    v_hashes     text[] := '{}';
    v_blob_hash  text[] := '{}';
    v_targets    jsonb  := '[]'::jsonb;
    v_strikes    jsonb  := '[]'::jsonb;
    v_row        record;
    v_rel        boolean;
    v_struck     text[] := '{}';
    v_team  record;
    v_n     integer;
    v_text_rem text;
    v_prof  kb_profiles%ROWTYPE;
    v_charters   uuid[] := '{}';
    v_resrows    jsonb  := '[]'::jsonb;
    v_disp       text;
    v_counts     jsonb  := jsonb_build_object('erase', 0, 'complete', 0, 'skip', 0, 'charter', 0);
    v_ctx_erase  uuid[] := '{}';
    v_ctx_fields jsonb  := '[]'::jsonb;
    v_trail      uuid[] := '{}';
    v_retained   jsonb  := _erasure_retention_snapshot();
    v_kept       text;
    v_erasing    uuid[] := '{}';
    v_completing uuid[] := '{}';
BEGIN
    SELECT id INTO v_exists FROM kb_profiles WHERE id = p_subject;
    IF v_exists IS NULL THEN
        RAISE EXCEPTION 'principal_erasure_execute: subject % not found', p_subject;
    END IF;

    -- The estate contexts (R1, ruled 2026-10-10): the subject's @me contexts and their personal
    -- team's contexts, from _erasure_estate_contexts, the ONE definition. Other team contexts and
    -- map homes are NOT the estate (disposition iii). The act records this set in its event and
    -- the redaction reads it from there, so the arms no longer re-derive it.
    v_governed := ARRAY(SELECT _erasure_estate_contexts(p_subject));

    -- ── Scope: the estate, HOME-PURE (ruled 2026-09-11 with Pete — the "scope of
    -- engagement" ruling) ─────────────────────────────────────────────────────────────
    -- Everything homed in a governed context wipes with the estate, whoever authored,
    -- owns or emitted it: writing into someone's PRIVATE context under a grant declares
    -- the content's scope of engagement — it lives and dies with that estate (the
    -- terms-of-use documentation states this). The 20260909000025 actor halves are
    -- dissolved: the owner/originator OR kept reassigned-in-place authorship in reach
    -- (20260703140000 moves owner_profile_id in place), and the emitted half was already
    -- subsumed by the governed-home EXISTS it carried. Erasure keeps the resource rows
    -- live (D3), so a grantee whose prose dies with the estate holds their remaining
    -- access only against a retired context (arm 13), and their attribution dies by the
    -- pseudonym break, never by edits.
    SELECT coalesce(array_agg(DISTINCT h.resource_id), '{}') INTO v_resources
      FROM kb_resource_homes h
     WHERE h.anchor_table = 'kb_contexts'
       AND h.anchor_id = ANY(v_governed);

    -- ── Each estate resource's disposition, in resource-id order: the order the act erases them
    --    in (2c), which is also the order every person act takes their locks in.
    --      charter  — a cogmap's telos. Resource erasure refuses it; the act empties its text
    --                 by hash below and holds it until map-grain erasure (2a).
    --      erase    — live or soft-deleted: resource erasure runs on it (2b).
    --      complete — erased, and the ledger still holds paths to rewrite (an erasure from
    --                 before cut 2): resource erasure's completion pass runs (2f).
    --      skip     — erased and complete (2f). ──────────────────────────────────────────────
    FOR v_row IN
        SELECT r.id,
               r.erased_at IS NOT NULL AS erased,
               EXISTS (SELECT 1 FROM kb_cogmaps m WHERE m.telos_resource_id = r.id) AS charter
          FROM kb_resources r
         WHERE r.id = ANY(v_resources)
         ORDER BY r.id
    LOOP
        v_disp := CASE
            WHEN v_row.charter THEN 'charter'
            WHEN NOT v_row.erased THEN 'erase'
            WHEN resource_erasure_completion_fields(v_row.id) <> '[]'::jsonb THEN 'complete'
            ELSE 'skip'
        END;
        IF v_disp = 'charter' THEN
            v_charters := v_charters || v_row.id;
        END IF;
        v_resrows := v_resrows || jsonb_build_array(jsonb_build_object(
            'resource_id', v_row.id, 'disposition', v_disp));
        v_counts := jsonb_set(v_counts, ARRAY[v_disp], to_jsonb((v_counts->>v_disp)::integer + 1));
    END LOOP;

    -- The text hashes the act empties BY HASH (D5, ruled 2026-10-10): the charters' chunk and
    -- verbatim block content hashes. Every other estate resource is emptied row by row by
    -- resource erasure, which admits no hash to kb_erased_content.
    SELECT coalesce(array_agg(DISTINCT h), '{}') INTO v_hashes FROM (
        SELECT c.content_hash AS h FROM kb_chunks c
         WHERE c.resource_id = ANY(v_charters)
        UNION
        SELECT bc.content_hash AS h
          FROM kb_block_content bc
          JOIN kb_block_revisions br ON br.id = bc.block_revision_id
          JOIN kb_content_blocks b ON b.id = br.block_id
         WHERE b.resource_id = ANY(v_charters)
           AND bc.content <> ''
    ) s;

    -- ── The blob pass: HOME-PURE (the 2026-09-12 ruling), WOULD-STRIKE structured ───────
    -- Scope = every blob row homed in a governed context, WHOEVER committed it, plus the
    -- subject's own rows (owner/originator/emitted) wherever they home — the subject's
    -- team- and map-homed rows stay the named remainder (disposition iii). Governed-home
    -- LIVE rows report would-strike entries; governed-home struck rows report
    -- already-erased; EVERY other home is the named remainder. released_would_be SIMULATES
    -- the act's own sequential refcount: the ONE blob_delete live-row predicate (live rows
    -- with the hash, this one included, <= 1) minus the same-hash rows EARLIER IN THE
    -- PLAN'S OWN STRIKE SET — execute consumes the entries in order, so those siblings are
    -- already emptied when the act reaches this row. Run at survey time WITHOUT the hash's
    -- advisory lock: a PREDICTION, honest about the moment it ran; the wrapper's
    -- strike-time verdict is authoritative in the act. The enumeration is ORDERED BY id —
    -- the act strikes in plan order and the survey must walk the SAME sequence when it
    -- simulates it, so the order cannot be left to the scan.
    FOR v_row IN
        SELECT b.id, b.content_hash, b.blob_pathname, b.content_type,
               (b.home_table = 'kb_contexts' AND b.home_id = ANY(v_governed)) AS governed_home
          FROM kb_blobs b
         WHERE (b.home_table = 'kb_contexts' AND b.home_id = ANY(v_governed))
            OR b.owner_profile_id = p_subject
            OR b.originator_profile_id = p_subject
            OR b.asserted_by_event_id IN (
               SELECT e.id FROM kb_events e
                 JOIN kb_entities en ON en.id = e.emitter_entity_id
                WHERE en.profile_id = p_subject)
          ORDER BY b.id
    LOOP
        IF v_row.governed_home AND v_row.content_type IS NOT NULL THEN
            -- The act's own SEQUENTIAL refcount, simulated: the strike loop consumes the
            -- plan's rows by id IN ORDER and each blob_delete counts live rows at ITS
            -- moment, so every same-hash row already in this plan's strike set is emptied
            -- when the act reaches this row — subtract it (accumulated below, in plan
            -- order) from the ONE live-row predicate. Two same-hash rows in the estate
            -- therefore predict released=false then released=true, exactly as the act
            -- strikes them.
            v_rel := ((SELECT count(*) FROM kb_blobs live
                        WHERE live.content_hash = v_row.content_hash
                          AND live.content_type IS NOT NULL)
                      - (SELECT count(*) FROM unnest(v_struck) prior
                          WHERE prior = v_row.content_hash)) <= 1;
            v_struck := v_struck || ARRAY[v_row.content_hash];
            v_blob_hash := v_blob_hash || ARRAY[v_row.content_hash];
            v_targets := v_targets || jsonb_build_array(jsonb_build_object(
                'target',  'kb_blobs',
                'would_strike', jsonb_build_object(
                    'blob_id',           v_row.id,
                    'content_hash',      v_row.content_hash,
                    'pathname',          v_row.blob_pathname,
                    'released_would_be', v_rel)));
            v_strikes := v_strikes || jsonb_build_array(jsonb_build_object(
                'blob_id',           v_row.id,
                'released_would_be', v_rel));
        ELSIF v_row.governed_home THEN
            v_targets := v_targets || jsonb_build_array(jsonb_build_object(
                'target',  'kb_blobs',
                'outcome', 'already-erased'));
        ELSE
            -- The named remainder: independent_obligation-shaped, hash named (D2 vocabulary),
            -- never admitted to the erased-content set.
            v_targets := v_targets || jsonb_build_array(jsonb_build_object(
                'target',  'kb_blobs',
                'outcome', 'independent_obligation: home governed by a team or map; '
                           || 'hash ' || v_row.content_hash || ' not struck'));
        END IF;
    END LOOP;

    v_hashes := v_hashes || v_blob_hash;

    -- ── Per-target outcomes, read against the PRE-redaction state (the mutations live in
    -- _erasure_apply_redaction alone; these reads only report what it will find).
    -- 20260911000000: every read below scopes to GOVERNED homes with the same predicate
    -- the redaction spells — the record reports "erased" only for rows the act actually
    -- redacts; a same-hash row in an ungoverned home is nobody's outcome (it is not the
    -- subject's data and was never a target). ────────────────────────────────────────────
    SELECT * INTO v_prof FROM kb_profiles WHERE id = p_subject;
    IF v_prof.tombstoned_at IS NOT NULL THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_profiles.handle',        'outcome','already-erased'),
            jsonb_build_object('target','kb_profiles.display_name',  'outcome','already-erased'),
            jsonb_build_object('target','kb_profiles.tombstoned_at', 'outcome','already-erased'));
    ELSE
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_profiles.handle',        'outcome','sentinel-scrubbed'),
            jsonb_build_object('target','kb_profiles.display_name',  'outcome','sentinel-scrubbed'),
            jsonb_build_object('target','kb_profiles.tombstoned_at', 'outcome','tombstoned'));
        IF v_prof.email IS NOT NULL THEN
            v_targets := v_targets || jsonb_build_array(
                jsonb_build_object('target','kb_profiles.email','outcome','erased'));
        END IF;
        IF v_prof.preferences <> '{}'::jsonb THEN
            v_targets := v_targets || jsonb_build_array(
                jsonb_build_object('target','kb_profiles.preferences','outcome','erased'));
        END IF;
    END IF;

    SELECT * INTO v_team FROM kb_teams t WHERE t.personal_of = p_subject;
    IF FOUND THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_teams.slug',
                'outcome', CASE WHEN v_team.slug = 'personal-erased-' || p_subject::text
                                THEN 'already-erased' ELSE 'sentinel-scrubbed' END),
            jsonb_build_object('target','kb_teams.name',
                'outcome', CASE WHEN v_team.slug = 'personal-erased-' || p_subject::text
                                THEN 'already-erased' ELSE 'sentinel-scrubbed' END));
    END IF;

    -- The Slack stores, matched through a Slack auth link or the vault row's own profile_id (R3):
    -- a vault row can outlive its link.
    SELECT count(*) INTO v_n FROM kb_slack_grant_vault v
     WHERE v.slack_principal_id IN (SELECT _erasure_subject_slack_principals(p_subject));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_slack_grant_vault.slack_principal_id',
                'outcome','deleted'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_slack_link_intents i
     WHERE i.slack_principal_id IN (SELECT _erasure_subject_slack_principals(p_subject));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_slack_link_intents.slack_principal_id',
                'outcome','deleted'));
    END IF;

    -- The auth-link identifiers, the entity names, and the subject's own artifact content —
    -- outcomes read against the PRE-redaction state exactly like every arm above (the
    -- mutations live in _erasure_apply_redaction alone; these reads only report what it
    -- will find). The artifact remainder subset is named with the disposition-iii
    -- vocabulary the blob arm set: another principal's kind namespace on a governed
    -- resource is an independent obligation, re-named on every run because it stands.
    SELECT count(*) INTO v_n FROM kb_profile_auth_links l
     WHERE l.profile_id = p_subject AND l.email IS NOT NULL;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_profile_auth_links.email',
                'outcome','erased'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_profile_auth_links l
     WHERE l.profile_id = p_subject
       AND l.auth_provider_user_id IS DISTINCT FROM 'erased-' || l.id::text;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_profile_auth_links.auth_provider_user_id',
                'outcome','sentinel-scrubbed'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_entities en
     WHERE en.profile_id = p_subject
       AND en.name IS DISTINCT FROM 'erased-' || en.id::text;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_entities.name',
                'outcome','sentinel-scrubbed'));
    END IF;

    -- ── The operational identity arms (R3), read against the pre-redaction state through the
    --    same selectors the redaction uses. Each is claimed only when the act will change a row, so
    --    a re-run omits it. ──────────────────────────────────────────────────────────────────────
    SELECT count(*) INTO v_n FROM kb_entities en
     WHERE en.profile_id = p_subject AND en.metadata <> '{}'::jsonb;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_entities.metadata', 'outcome','erased'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_team_invitations i
     WHERE i.id IN (SELECT _erasure_subject_invitations(p_subject))
       AND i.invited_email IS DISTINCT FROM 'erased-' || i.id::text;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_team_invitations.invited_email',
                'outcome', v_n || ' invitation(s) sent by or to the subject; sentinel-scrubbed, '
                           || 'token replaced, pending ones revoked'));
    END IF;
    -- Invitations to an address the subject holds only unverified: perhaps theirs, perhaps not, so
    -- never touched, and named by count.
    SELECT count(*) INTO v_n FROM kb_team_invitations i
     WHERE i.invited_email IS DISTINCT FROM 'erased-' || i.id::text
       AND i.id NOT IN (SELECT _erasure_subject_invitations(p_subject))
       AND lower(i.invited_email COLLATE "C") IN (
           SELECT lower(l.email COLLATE "C") FROM kb_profile_auth_links l
            WHERE l.profile_id = p_subject AND l.email IS NOT NULL AND NOT l.email_verified);
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_team_invitations.invited_email (unverified address)',
                'outcome', 'independent_obligation: ' || v_n || ' invitation(s) to an address the '
                           || 'subject never verified; may be another person''s; not reached'));
    END IF;
    -- Unbound refresh tokens are reached by the login provider the doors pass. Without it they
    -- cannot be attributed, so say so rather than imply none exist.
    IF _erasure_auth_provider() IS NULL
       AND EXISTS (SELECT 1 FROM kb_oauth_refresh_tokens t
                    WHERE t.profile_id IS NULL AND t.revoked_at IS NULL) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_oauth_refresh_tokens (unbound)',
                'outcome', 'not checked: the act was not told the login provider, so tokens never '
                           || 'bound to a profile cannot be attributed'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_oauth_flow f
     WHERE f.id IN (SELECT _erasure_subject_oauth_flows(p_subject));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_oauth_flow', 'outcome', v_n || ' flow(s) deleted'));
    END IF;
    v_kept := _erasure_retained_by(v_retained, 'credential-replay-evidence');
    SELECT count(*) INTO v_n FROM kb_oauth_refresh_tokens t
     WHERE t.id IN (SELECT _erasure_subject_refresh_tokens(p_subject))
       AND (v_kept IS NULL OR t.claims <> '{}'::jsonb);
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_oauth_refresh_tokens',
                'outcome', CASE WHEN v_kept IS NULL
                                THEN v_n || ' refresh token(s) revoked and deleted'
                                ELSE v_n || ' refresh token(s) revoked; claims and hash erased' END));
    END IF;
    SELECT count(*) INTO v_n FROM kb_oauth_refresh_replays r
     WHERE r.profile_id = p_subject
        OR r.token_id IN (SELECT _erasure_subject_refresh_tokens(p_subject));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_oauth_refresh_replays',
                'outcome', CASE WHEN v_kept IS NULL THEN v_n || ' replay record(s) deleted'
                                ELSE 'retained under policy ' || v_kept END));
    END IF;
    SELECT count(*) INTO v_n FROM kb_connections c
     WHERE c.id IN (SELECT _erasure_subject_connections(p_subject, v_governed))
       AND (c.revoked_at IS NULL OR c.credential IS NOT NULL OR c.observed_reach IS NOT NULL);
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_connections.credential',
                'outcome', v_n || ' connection(s) revoked in temper; credential and observed reach '
                           || 'erased; tokens already minted at the provider are not revoked there'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_blob_uploads u
     WHERE u.id IN (SELECT _erasure_subject_uploads(p_subject, v_governed));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_blob_upload_segments.bytes',
                'outcome', v_n || ' staged upload(s) deleted'));
    END IF;
    -- The free text about the subject: erased unless a policy keeps its category (R4).
    SELECT count(*) INTO v_n FROM kb_principal_standing_events s
     WHERE s.profile_id = p_subject AND s.reason IS NOT NULL;
    IF v_n > 0 THEN
        v_kept := _erasure_retained_by(v_retained, 'standing-reasons');
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_principal_standing_events.reason',
                'outcome', coalesce('retained under policy ' || v_kept, 'erased')));
    END IF;
    SELECT count(*) INTO v_n FROM kb_join_requests j
     WHERE j.requesting_profile_id = p_subject
       AND (j.message IS NOT NULL OR j.decision_note IS NOT NULL);
    IF v_n > 0 THEN
        v_kept := _erasure_retained_by(v_retained, 'join-request-notes');
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_join_requests.message',
                'outcome', coalesce('retained under policy ' || v_kept, 'erased')));
    END IF;
    SELECT count(*) INTO v_n FROM kb_principal_review_requests r
     WHERE r.profile_id = p_subject
       AND (r.message IS NOT NULL OR r.decision_note IS NOT NULL);
    IF v_n > 0 THEN
        v_kept := _erasure_retained_by(v_retained, 'review-request-notes');
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_principal_review_requests.message',
                'outcome', coalesce('retained under policy ' || v_kept, 'erased')));
    END IF;
    -- The ledger copies a policy keeps: named, never rewritten.
    v_targets := v_targets || coalesce((
        SELECT jsonb_agg(jsonb_build_object(
                   'target',  'kb_events.payload (' || k.cls || ')',
                   'outcome', 'retained under policy ' || k.policy) ORDER BY k.cls)
          FROM (SELECT DISTINCT
                       CASE t.name WHEN 'principal_standing_changed'   THEN 'standing reasons'
                                   WHEN 'principal_governance_changed' THEN 'governance reasons'
                                   ELSE 'erasure-refusal details' END AS cls,
                       _erasure_retained_by(v_retained,
                           _principal_erasure_path_category(t.name, p.path)) AS policy
                  FROM kb_events e
                  JOIN kb_event_types t ON t.id = e.event_type_id
                  JOIN _principal_erasure_redact_paths() p ON p.event_type = t.name
                 WHERE t.name IN ('principal_standing_changed', 'principal_governance_changed',
                                  'principal_erasure_refused')
                   AND e.payload ->> 'subject_table' = 'kb_profiles'
                   AND e.payload ->> 'subject_id' = p_subject::text
                   AND jsonb_typeof(e.payload -> p.path) = 'string') k
         WHERE k.policy IS NOT NULL), '[]'::jsonb);
    SELECT count(*) INTO v_n FROM kb_data_artifacts da
      JOIN kb_data_artifact_content dac ON dac.artifact_id = da.id
     WHERE da.kind_owner_table = 'kb_profiles'
       AND da.kind_owner_id = p_subject
       AND dac.content <> '{}'::jsonb
       AND da.resource_id = ANY(v_charters);
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_data_artifact_content.content',
                'outcome','erased'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_data_artifacts da
      JOIN kb_data_artifact_content dac ON dac.artifact_id = da.id
     WHERE dac.content <> '{}'::jsonb
       AND (da.kind_owner_table <> 'kb_profiles' OR da.kind_owner_id <> p_subject)
       AND da.resource_id = ANY(v_charters);
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_data_artifact_content.content (other-kind)',
                'outcome','independent_obligation: the artifact''s kind is owned by '
                           || 'another principal; content on a governed resource not '
                           || 'struck'));
    END IF;

    IF EXISTS (SELECT 1 FROM kb_chunk_content cc
                 JOIN kb_chunks c ON c.id = cc.chunk_id
                WHERE c.content_hash = ANY(v_hashes)
                  AND c.resource_id IN (
                      SELECT h.resource_id FROM kb_resource_homes h
                       WHERE h.anchor_table = 'kb_contexts'
                         AND h.anchor_id = ANY(v_governed))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_chunk_content.content',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_chunk_content cc
                                               JOIN kb_chunks c ON c.id = cc.chunk_id
                                              WHERE c.content_hash = ANY(v_hashes)
                                                AND cc.content <> ''
                                                AND c.resource_id IN (
                                                    SELECT h.resource_id FROM kb_resource_homes h
                                                     WHERE h.anchor_table = 'kb_contexts'
                                                       AND h.anchor_id = ANY(v_governed)))
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;
    IF EXISTS (SELECT 1 FROM kb_block_content
                WHERE content_hash = ANY(v_hashes)
                  AND block_revision_id IN (
                      SELECT br.id FROM kb_block_revisions br
                        JOIN kb_content_blocks b ON b.id = br.block_id
                       WHERE b.resource_id IN (
                             SELECT h.resource_id FROM kb_resource_homes h
                              WHERE h.anchor_table = 'kb_contexts'
                                AND h.anchor_id = ANY(v_governed)))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_block_content.content',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_block_content
                                              WHERE content_hash = ANY(v_hashes)
                                                AND content <> ''
                                                AND block_revision_id IN (
                                                    SELECT br.id FROM kb_block_revisions br
                                                      JOIN kb_content_blocks b ON b.id = br.block_id
                                                     WHERE b.resource_id IN (
                                                           SELECT h.resource_id FROM kb_resource_homes h
                                                            WHERE h.anchor_table = 'kb_contexts'
                                                              AND h.anchor_id = ANY(v_governed))))
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;

    -- The text remainder (the attribution ruling, 2026-09-13, on the 2026-09-06 clause):
    -- prose ATTRIBUTED to the subject by authorship — content blocks whose genesis event
    -- the subject's entity emitted — in homes outside the governed estate (team and map
    -- alike). Named for audit with count + hashes (capped at 8, "and N more" beyond),
    -- never struck, never admitted to v_hashes: nothing in a shared home is the act's to
    -- strike, and the redaction's reads stay governed-scoped. Attribution is the emitting
    -- entity, the blob arm's own emitted-half shape; direct authorship only — carried
    -- attribution is provenance the pseudonym break already covers.
    v_text_rem := (
        SELECT 'independent_obligation: home governed by a team or map; '
               || count(DISTINCT bc.content_hash) || ' text hash(s) not struck'
               || CASE WHEN count(DISTINCT bc.content_hash) <= 8
                       THEN ': ' || array_to_string(array_agg(DISTINCT bc.content_hash), ', ')
                       ELSE ': ' || array_to_string(ARRAY(
                                   SELECT DISTINCT bc2.content_hash
                                     FROM kb_block_content bc2
                                     JOIN kb_block_revisions br2 ON br2.id = bc2.block_revision_id
                                     JOIN kb_content_blocks b2 ON b2.id = br2.block_id
                                     JOIN kb_events ge2 ON ge2.id = b2.genesis_event_id
                                     JOIN kb_entities ge2_en ON ge2_en.id = ge2.emitter_entity_id
                                    WHERE ge2_en.profile_id = p_subject
                                      AND bc2.content <> ''
                                      AND NOT EXISTS (SELECT 1 FROM kb_resource_homes hg2
                                                       WHERE hg2.resource_id = b2.resource_id
                                                         AND hg2.anchor_table = 'kb_contexts'
                                                         AND hg2.anchor_id = ANY(v_governed))
                                    ORDER BY 1 LIMIT 8), ', ')
                            || '; and ' || (count(DISTINCT bc.content_hash) - 8) || ' more'
                  END
          FROM kb_block_content bc
          JOIN kb_block_revisions br ON br.id = bc.block_revision_id
          JOIN kb_content_blocks b ON b.id = br.block_id
          JOIN kb_events ge ON ge.id = b.genesis_event_id
          JOIN kb_entities ge_en ON ge_en.id = ge.emitter_entity_id
         WHERE ge_en.profile_id = p_subject
           AND bc.content <> ''
           AND NOT EXISTS (SELECT 1 FROM kb_resource_homes hg
                            WHERE hg.resource_id = b.resource_id
                              AND hg.anchor_table = 'kb_contexts'
                              AND hg.anchor_id = ANY(v_governed))
        HAVING count(DISTINCT bc.content_hash) > 0);
    IF v_text_rem IS NOT NULL THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_block_content.content',
            'outcome', v_text_rem));
    END IF;
    IF EXISTS (SELECT 1 FROM kb_chunks
                WHERE content_hash = ANY(v_hashes)
                  AND resource_id IN (
                      SELECT h.resource_id FROM kb_resource_homes h
                       WHERE h.anchor_table = 'kb_contexts'
                         AND h.anchor_id = ANY(v_governed))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_chunks.embedding',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_chunks
                                              WHERE content_hash = ANY(v_hashes)
                                                AND embedding IS NOT NULL
                                                AND resource_id IN (
                                                    SELECT h.resource_id FROM kb_resource_homes h
                                                     WHERE h.anchor_table = 'kb_contexts'
                                                       AND h.anchor_id = ANY(v_governed)))
                                THEN 'erased' ELSE 'already-erased' END),
            jsonb_build_object('target','kb_chunks.embedded_with',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_chunks
                                              WHERE content_hash = ANY(v_hashes)
                                                AND embedding IS NOT NULL
                                                AND resource_id IN (
                                                    SELECT h.resource_id FROM kb_resource_homes h
                                                     WHERE h.anchor_table = 'kb_contexts'
                                                       AND h.anchor_id = ANY(v_governed)))
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;
    -- The heading trail, arm (4a) of the redaction (20261020100000). Claimed only when the act
    -- will null one: a chunk with no heading trail never held one, so "already-erased" would
    -- claim an act that never happened. A re-run of the act therefore omits the target.
    IF EXISTS (SELECT 1 FROM kb_chunks
                WHERE content_hash = ANY(v_hashes)
                  AND header_path IS NOT NULL
                  AND resource_id IN (
                      SELECT h.resource_id FROM kb_resource_homes h
                       WHERE h.anchor_table = 'kb_contexts'
                         AND h.anchor_id = ANY(v_governed))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_chunks.header_path', 'outcome','erased'));
    END IF;
    IF EXISTS (SELECT 1 FROM kb_resource_search_index si
                WHERE si.resource_id IN (SELECT DISTINCT c.resource_id FROM kb_chunks c
                                          WHERE c.content_hash = ANY(v_hashes)
                                            AND c.resource_id IN (
                                                SELECT h.resource_id FROM kb_resource_homes h
                                                 WHERE h.anchor_table = 'kb_contexts'
                                                   AND h.anchor_id = ANY(v_governed)))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_resource_search_index.search_vector',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_resource_search_index si
                                              WHERE si.resource_id IN (
                                                    SELECT DISTINCT c.resource_id FROM kb_chunks c
                                                     WHERE c.content_hash = ANY(v_hashes)
                                                       AND c.resource_id IN (
                                                           SELECT h.resource_id FROM kb_resource_homes h
                                                            WHERE h.anchor_table = 'kb_contexts'
                                                              AND h.anchor_id = ANY(v_governed)))
                                              AND si.search_vector <> '')
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;

    SELECT count(*) INTO v_n FROM kb_cogmaps m
     WHERE m.shape_materialized_event_id IS NOT NULL
       AND m.id IN (
           SELECT r.home_anchor_id FROM kb_cogmap_regions r
             JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
            WHERE r.home_anchor_table = 'kb_cogmaps' AND NOT r.is_folded
              AND mem.member_table = 'kb_resources'
              AND mem.member_id = ANY(v_resources));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_cogmaps.shape_materialized_event_id',
                'outcome','recompute-marked'));
    END IF;
    -- The estate contexts the act erases (R7, ruled 2026-10-10): every one without a
    -- context_erased event yet. Each gets one, which retires it and replaces its name and slug with
    -- the sentinels; a re-run finds none left. The context rows' earlier names and slugs on the
    -- ledger (context_renamed, context_retired, context_restored) are rewritten to the same
    -- sentinels under the act's own authority: redacted_fields, paths only.
    v_ctx_erase := _principal_erasure_contexts_to_erase(v_governed);
    IF cardinality(v_ctx_erase) > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_contexts.is_active','outcome','retired'),
            jsonb_build_object('target','kb_contexts.name',     'outcome','sentinel-scrubbed'),
            jsonb_build_object('target','kb_contexts.slug',     'outcome','sentinel-scrubbed'));
    END IF;
    v_ctx_fields := _principal_erasure_redacted_fields(p_subject, v_governed, v_retained);
    SELECT count(*) INTO v_n
      FROM jsonb_array_elements(v_ctx_fields) f
      JOIN kb_events e ON e.id = (f ->> 'event')::uuid
     WHERE e.producing_anchor_table = 'kb_contexts';
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_events.payload',
            'outcome', v_n
                       || ' context event(s) naming an estate context; names and slugs sentinel-scrubbed'));
    END IF;
    -- The subject's own admin events (R3): standing and governance reasons, and Slack ids.
    v_n := jsonb_array_length(v_ctx_fields) - v_n;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_events.payload',
            'outcome', v_n
                       || ' admin event(s) about the subject; reasons and refusal details erased and Slack ids sentinel-scrubbed'));
    END IF;

    -- The edges principal_erasure_execute's edge step erases, predicted by the step's own
    -- selection (homed in an estate context, not _edge_erasure_complete) over the state the
    -- resource erasures before it leave:
    --   * an edge touching an `erase` resource is complete then: that erasure folds every live
    --     edge touching its resource, nulls its label, sets its owned properties to their
    --     sentinels and rewrites its trail;
    --   * an edge touching a `complete` resource is complete then only if it is already folded:
    --     the completion pass does the rest of that but folds nothing, so a live one is the
    --     step's;
    --   * a `skip` or `charter` resource is not reached, so an edge touching one is decided by
    --     its own state.
    -- The act replaces this entry in place with the count it erased, and drops it at zero. The
    -- count differs from this prediction only when the state changes after the plan: a resource
    -- erased or moved by another act, an estate context leaving the estate.
    v_erasing := ARRAY(SELECT (r ->> 'resource_id')::uuid FROM jsonb_array_elements(v_resrows) r
                        WHERE r ->> 'disposition' = 'erase');
    v_completing := ARRAY(SELECT (r ->> 'resource_id')::uuid
                            FROM jsonb_array_elements(v_resrows) r
                           WHERE r ->> 'disposition' = 'complete');
    SELECT count(*) INTO v_n
      FROM kb_edges e
     WHERE e.home_anchor_table = 'kb_contexts'
       AND e.home_anchor_id = ANY(v_governed)
       AND NOT (e.source_table = 'kb_resources' AND e.source_id = ANY(v_erasing))
       AND NOT (e.target_table = 'kb_resources' AND e.target_id = ANY(v_erasing))
       AND NOT (e.is_folded
                AND ((e.source_table = 'kb_resources' AND e.source_id = ANY(v_completing))
                  OR (e.target_table = 'kb_resources' AND e.target_id = ANY(v_completing))))
       AND NOT _edge_erasure_complete(e.id);
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',            'kb_edges',
            'would_erase_edges', v_n,
            'outcome',           _principal_erasure_edges_outcome(v_n)));
    END IF;

    -- What the subject wrote outside the estate (R8, disposition iii): it stays, and the record
    -- names it by count per carrier class, never by content. A carrier is one of the subject's own
    -- ledger events (its emitting entity is theirs) still holding the text: outside the trail of
    -- every estate resource, which resource erasure rewrites, and of every edge homed in the
    -- estate, which the edge step rewrites, and with no redaction row at the path, which an
    -- earlier erasure or scrub already rewrote. An edge homed in the estate is inside it folded
    -- or not, since the edge step erases both.
    v_trail := ARRAY(SELECT DISTINCT ts.event_id
                       FROM unnest(v_resources) AS er(id),
                            LATERAL _resource_erasure_trail_scope(er.id) ts
                     UNION
                     SELECT ts.event_id
                       FROM kb_edges e,
                            LATERAL _edge_erasure_trail_scope(e.id) ts
                      WHERE e.home_anchor_table = 'kb_contexts'
                        AND e.home_anchor_id = ANY(v_governed));
    v_targets := v_targets || coalesce((
        SELECT jsonb_agg(jsonb_build_object(
                   'target',  'kb_events.payload (' || c.cls || ')',
                   'outcome', 'independent_obligation: authored by the subject outside the estate; '
                              || c.n || ' event(s) kept')
                   ORDER BY c.cls)
          FROM (SELECT s.cls, count(*) AS n
                  FROM (SELECT CASE
                                 WHEN t.name IN ('resource_created', 'resource_updated')
                                      AND jsonb_typeof(e.payload -> 'title') = 'string'
                                      AND NOT EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                                       WHERE r.event_id = e.id AND r.path = 'title')
                                   THEN 'titles'
                                 -- A doc_type value is a type name, not authored text.
                                 WHEN t.name IN ('property_set', 'property_asserted')
                                      AND e.payload ? 'value'
                                      AND e.payload ->> 'property_key' IS DISTINCT FROM 'doc_type'
                                      AND NOT EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                                       WHERE r.event_id = e.id AND r.path = 'value')
                                   THEN 'property values'
                                 WHEN t.name = 'relationship_asserted'
                                      AND NOT _erasure_label_is_kept(e.payload -> 'label')
                                      AND NOT EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                                       WHERE r.event_id = e.id AND r.path = 'label')
                                   THEN 'edge labels'
                                 WHEN t.name = 'citation_audited'
                                      AND jsonb_typeof(e.payload -> 'reason') = 'string'
                                      AND NOT EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                                       WHERE r.event_id = e.id AND r.path = 'reason')
                                   THEN 'citation-audit reasons'
                                 -- A rename the subject made of a context outside the estate: the
                                 -- estate's own are rewritten by this act.
                                 WHEN t.name = 'context_renamed'
                                      AND NOT (e.producing_anchor_id = ANY(v_governed))
                                      AND jsonb_typeof(e.payload -> 'to_name') = 'string'
                                      AND NOT EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                                       WHERE r.event_id = e.id AND r.path = 'to_name')
                                   THEN 'context names'
                               END AS cls
                          FROM kb_events e
                          JOIN kb_event_types t ON t.id = e.event_type_id
                          JOIN kb_entities en ON en.id = e.emitter_entity_id
                         WHERE en.profile_id = p_subject
                           AND t.name IN ('resource_created', 'resource_updated', 'property_set',
                                          'property_asserted', 'relationship_asserted',
                                          'citation_audited', 'context_renamed')
                           AND NOT (e.id = ANY(v_trail))) s
                 WHERE s.cls IS NOT NULL
                 GROUP BY s.cls) c), '[]'::jsonb);
    SELECT count(*) INTO v_n FROM kb_contexts c
     WHERE c.shape_materialized_event_id IS NOT NULL
       AND ( c.id IN (
               SELECT r.home_anchor_id FROM kb_cogmap_regions r
                 JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
                WHERE r.home_anchor_table = 'kb_contexts' AND NOT r.is_folded
                  AND mem.member_table = 'kb_resources'
                  AND mem.member_id = ANY(v_resources))
          -- Every estate context (R1).
          OR c.id = ANY(v_governed) );
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_contexts.shape_materialized_event_id',
                'outcome','recompute-marked'));
    END IF;

    -- The charters held (2a), and the resources already complete (2f), named in the record. The
    -- edge step erases a charter's edges homed in the estate, so only the others are kept.
    v_targets := v_targets || coalesce((
        SELECT jsonb_agg(jsonb_build_object(
                   'target',  'kb_resources',
                   'outcome', 'charter ' || ch || '; held until map-grain erasure (task '
                              || '01a0e960-0ca2-7f42-b33e-1ed19b024e6b); text emptied; title, '
                              || 'properties, its own ledger and edges homed outside the estate kept')
                   ORDER BY ch)
          FROM unnest(v_charters) AS ch), '[]'::jsonb);
    IF (v_counts->>'skip')::integer > 0 THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_resources',
            'outcome', (v_counts->>'skip') || ' estate resource(s) already erased and complete; skipped'));
    END IF;

    RETURN jsonb_build_object(
        'estate_contexts', (SELECT coalesce(jsonb_agg(g ORDER BY g), '[]'::jsonb) FROM unnest(v_governed) AS g),
        'estate',          v_counts || jsonb_build_object('total', jsonb_array_length(v_resrows)),
        'resources',       v_resrows,
        'charters',        (SELECT coalesce(jsonb_agg(ch ORDER BY ch), '[]'::jsonb) FROM unnest(v_charters) AS ch),
        'redacted_hashes', to_jsonb(v_hashes),
        'targets',         v_targets,
        'already_erased',  (v_prof.tombstoned_at IS NOT NULL),
        'blob_strikes',    v_strikes,
        'contexts_to_erase', (SELECT coalesce(jsonb_agg(g ORDER BY g), '[]'::jsonb) FROM unnest(v_ctx_erase) AS g),
        'redacted_fields',   v_ctx_fields,
        'retained',          v_retained);
END;
$function$;



-- ---------------------------------------------------------------------------
-- Section 6. The act: the edge step.
-- ---------------------------------------------------------------------------
-- 20261023100000_person_erasure_identifiers_and_credentials.sql's body. After the resource
-- erasures and before the strike loop, it runs edge_erasure_execute for every edge homed in an
-- estate context whose text is not yet erased, replaces the plan's kb_edges entry with the count
-- erased, and references each edge_erased from the record. The strikes, the record, the
-- redaction (whose step 7 tombstones the subject) and the context closure follow unchanged, so
-- the tombstone the edge records require at commit is set in the same transaction.
--
-- It pins lock_timeout = 0, as resource_erasure_execute and block_history_scrub_execute do
-- (20261015100000_erasure_acts_pin_lock_timeout.sql) and edge_erasure_execute does
-- (20261202000030_person_erasure_folds_estate_homed_edges.sql): the edge
-- step's FOR UPDATE waits on the edges' writers in this function's own body, and a lock_timeout
-- set on the session, the pool, the role or the database would otherwise end that wait with
-- 55P03. The service door also sets lock_timeout = 0 on the act's transaction
-- (operator_still_holds), so the pin is what holds for any other caller of the function.
CREATE OR REPLACE FUNCTION principal_erasure_execute(p_subject uuid, p_operator uuid, p_emitter uuid, p_request_ref uuid)
 RETURNS jsonb
 LANGUAGE plpgsql
 SET search_path TO 'public', 'pg_temp'
 SET lock_timeout TO '0'
AS $function$
DECLARE
    v_plan     jsonb;
    v_targets  jsonb;
    v_hashes   text[] := '{}';
    v_row      jsonb;
    v_bid uuid; v_rel boolean; v_path text;
    v_i        integer;
    v_ev       uuid;
    v_out      jsonb;
    v_erasures jsonb := '[]'::jsonb;
    v_refs     jsonb;
    v_estate   uuid[];
    v_rid      uuid;
    v_erased   boolean;
    v_homed    boolean;
    v_raced    integer := 0;
    v_payload  jsonb;
    v_cid      uuid;
    v_cpayload jsonb;
    v_cev      uuid;
    v_ctx_erase  uuid[];
    v_ctx_fields jsonb;
    v_eid      uuid;
    v_edge_erasures jsonb := '[]'::jsonb;
    v_n        integer;
    v_edges_raced integer := 0;
    v_reached  uuid[];
BEGIN
    -- The request reference is the correlation id of every event the act appends: the resource
    -- erasures' events, the strikes and the completion. Replay finds the act's span by it (D7).
    IF p_request_ref IS NULL THEN
        RAISE EXCEPTION 'principal_erasure_execute: p_request_ref is required';
    END IF;


    -- ONE computation per act. The existence RAISE lives in the plan (its message keeps the
    -- act's voice); the Rust gate resolves existence before either door is reached.
    v_plan := principal_erasure_survey_plan(p_subject);
    v_targets := v_plan->'targets';
    -- The plan's hash array comes back out IN ORDER (ORDINALITY makes the order load-bearing,
    -- not incidental): it is the redacted set the event carries.
    v_hashes := (SELECT coalesce(array_agg(h ORDER BY ord), '{}')
                   FROM jsonb_array_elements_text(v_plan->'redacted_hashes')
                        WITH ORDINALITY AS t(h, ord));

    -- ── The resource erasures (R5, 2c): every `erase` and `complete` resource, in the plan's
    --    resource-id order, through resource_erasure_execute itself, so each is that act whole:
    --    its edge folds, its resource_erased record, its body and its ledger rewrite. No blob list
    --    is passed: the strike loop below strikes every live blob homed in the estate. Each takes
    --    its own resource's locks as it reaches it, in id order, so two person acts cannot
    --    deadlock on them. A raise from any of them (a raced fold or remote source) aborts the
    --    whole act; the service retries it.
    --
    --    The plan was read before any of these locks, so each resource is decided again under its
    --    own: the act queue, then the row, the order resource_erasure_execute itself takes them,
    --    as resource_field_scrub_execute and block_history_scrub_execute do (both locks are
    --    re-entrant in this transaction). A resource another act erased completely
    --    since the plan, or one moved out of the estate since, is skipped and counted; the kind
    --    recorded is the state found here, not the plan's prediction. ─────────────────────────
    v_estate := ARRAY(SELECT jsonb_array_elements_text(v_plan->'estate_contexts')::uuid);
    FOR v_row IN
        SELECT r FROM jsonb_array_elements(v_plan->'resources') r
         WHERE r->>'disposition' IN ('erase', 'complete')
    LOOP
        v_rid := (v_row->>'resource_id')::uuid;
        PERFORM pg_advisory_xact_lock(_resource_act_queue_key(v_rid));
        SELECT r.erased_at IS NOT NULL,
               EXISTS (SELECT 1 FROM kb_resource_homes h
                        WHERE h.resource_id = r.id AND h.anchor_table = 'kb_contexts'
                          AND h.anchor_id = ANY(v_estate))
          INTO v_erased, v_homed
          FROM kb_resources r WHERE r.id = v_rid
           FOR UPDATE;
        IF NOT coalesce(v_homed, false)
           OR (v_erased AND resource_erasure_completion_fields(v_rid) = '[]'::jsonb) THEN
            v_raced := v_raced + 1;
            CONTINUE;
        END IF;
        v_out := resource_erasure_execute(v_rid, p_operator, p_emitter, p_request_ref, '{}'::uuid[]);
        -- Keyed `resource` and `event`, never `resource_id`: the completion payload carries no
        -- key a trail function joins on (the D2 shape), so no resource's trail ever reaches it.
        v_erasures := v_erasures || jsonb_build_array(jsonb_build_object(
            'resource', v_rid,
            'event',    v_out->>'event_id',
            'kind',     CASE WHEN v_erased THEN 'completion' ELSE 'erasure' END));
    END LOOP;
    IF v_raced > 0 THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_resources',
            'outcome', v_raced || ' estate resource(s) erased or moved by another act after the plan; skipped'));
    END IF;

    -- ── The edge erasures: every edge homed in an estate context whose text is not yet erased,
    --    live or folded before the act, through edge_erasure_execute, so each is that unit whole:
    --    a live edge's fold, its edge_erased record, its body and its ledger rewrite. A folded
    --    edge is erased without a new fold.
    --
    --    After every resource erasure, so the selection reads what they left: each folds every
    --    edge touching its resource, nulls its label and sets its owned properties to their
    --    sentinels (steps 9c and 9d of _resource_erasure_apply_redaction), and rewrites the
    --    edge's trail, which is inside the resource's: _resource_erasure_trail_scope reads the
    --    edge's events by edge_id, its owned properties' events, and every event with the
    --    resource as source or target, so its phantom re-assertions too. Such an edge is then
    --    complete by _edge_erasure_complete, the one test the unit raises `already erased` on, so
    --    the completeness check below is the exclusion of the edges a resource erasure of this act
    --    already erased: the selection and the unit cannot disagree. The selection passes over an
    --    edge touching a resource this act erased (kind `erasure`) before that test, which derives
    --    the edge's whole trail under the edge locks: it would answer true for each. A resource
    --    this act completed (kind `completion`) is not passed over: the completion pass folds
    --    nothing, so a live edge touching it is the step's, and the test decides it.
    --
    --    The edges are locked FOR UPDATE in id order, and each is decided under its lock. The
    --    order is not global across the two loops: a resource erasure above has already locked
    --    the edges touching its resource. So two person acts whose estates share edges, or the act
    --    and a writer of two edges, can deadlock; Postgres aborts one side with 40P01, and the
    --    service retries it.
    --
    --    The selection reads the plan's estate, so an edge is homed in the estate again under its
    --    lock (_edge_homed_in_estate, the unit's own check): an edge whose context has left the
    --    estate since the plan is skipped and counted, as the resource loop skips a resource
    --    moved out. Its records' estate_of is the subject, tombstoned below in this transaction:
    --    the owner check runs at commit (edge_erasure_requires_an_erased_owner), and reads each
    --    record's edge folded and homed in the estate again then.
    --
    --    The step does not lock the edge's home context, so the three reads can disagree: a
    --    context_reassign moving the context out of the estate after this check and committing
    --    before the act's context lock below aborts the act, in the unit (`not homed in the
    --    estate`) or at commit (the owner check). Neither is retried: the service answers 500,
    --    nothing the act did persists, and the act run again leaves that context's edges, this
    --    check reading the live estate (ruling 12 of decision 01a1f6e8-81bd-758c-be87-a6824b66352a).
    --    ────────────────────────────────────────────────────────────────────────────────────────
    v_reached := ARRAY(SELECT (x ->> 'resource')::uuid FROM jsonb_array_elements(v_erasures) x
                        WHERE x ->> 'kind' = 'erasure');
    FOR v_eid IN
        SELECT e.id FROM kb_edges e
         WHERE e.home_anchor_table = 'kb_contexts'
           AND e.home_anchor_id = ANY(v_estate)
           AND NOT (e.source_table = 'kb_resources' AND e.source_id = ANY(v_reached))
           AND NOT (e.target_table = 'kb_resources' AND e.target_id = ANY(v_reached))
         ORDER BY e.id
           FOR UPDATE
    LOOP
        -- A complete edge (one another act already erased, or one touching a resource this act
        -- completed) is passed over before the home check, so it is never counted as skipped.
        CONTINUE WHEN _edge_erasure_complete(v_eid);
        IF NOT _edge_homed_in_estate(v_eid, p_subject) THEN
            v_edges_raced := v_edges_raced + 1;
            CONTINUE;
        END IF;
        v_out := edge_erasure_execute(v_eid, p_subject, p_operator, p_emitter, p_request_ref);
        v_edge_erasures := v_edge_erasures || jsonb_build_array(jsonb_build_object(
            'edge',  v_eid,
            'event', v_out->>'event_id'));
    END LOOP;
    -- The plan's predicted entry is replaced in place with the count erased here, the strike
    -- loop's pattern, and dropped at zero; a count the plan did not predict is appended. The
    -- edges skipped for their home are counted in their own entry.
    v_n := jsonb_array_length(v_edge_erasures);
    v_i := (SELECT t.ord - 1 FROM jsonb_array_elements(v_targets) WITH ORDINALITY AS t(x, ord)
             WHERE t.x ? 'would_erase_edges' ORDER BY t.ord LIMIT 1);
    IF v_i IS NOT NULL AND v_n = 0 THEN
        v_targets := v_targets - v_i;
    ELSIF v_i IS NOT NULL THEN
        v_targets := jsonb_set(v_targets, ARRAY[v_i::text], jsonb_build_object(
            'target',  'kb_edges',
            'outcome', _principal_erasure_edges_outcome(v_n)));
    ELSIF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_edges',
            'outcome', _principal_erasure_edges_outcome(v_n)));
    END IF;
    IF v_edges_raced > 0 THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_edges',
            'outcome', v_edges_raced || ' edge(s) homed in a context that left the estate '
                       || 'after the plan; skipped'));
    END IF;

    -- ── The strike loop: the plan's would-strike rows, IN ORDER, through the wrapper —
    -- per-row blob_delete('blob_erased', …) events exactly as before (the pairing with the
    -- completion is a fact, D1). Each structured entry is replaced IN PLACE with the prose
    -- built from the WRAPPER'S verdict — authoritative at strike time; the plan's
    -- released_would_be was a prediction and is discarded here.
    FOR v_i IN 0 .. jsonb_array_length(v_targets) - 1 LOOP
        v_row := v_targets->v_i->'would_strike';
        IF v_row IS NOT NULL THEN
            SELECT blob_id, released, pathname
              INTO v_bid, v_rel, v_path
              FROM blob_delete('blob_erased',
                               jsonb_build_object('blob_id', (v_row->>'blob_id')::uuid),
                               p_emitter,
                               p_correlation => p_request_ref);
            v_targets := jsonb_set(v_targets, ARRAY[v_i::text],
                jsonb_build_object(
                    'target',  'kb_blobs',
                    'outcome', blob_strike_outcome_text(v_rel, v_path)));
        END IF;
    END LOOP;

    -- ── The estate contexts are locked here, FOR NO KEY UPDATE, after the resource erasures, the
    --    edge step and the strikes, and before anything reads them for the record (R7).
    --    context_rename, context_retire, context_restore and context_reassign take the same row
    --    lock before they read the row, so one that arrives once the act holds it waits for the act
    --    and then finds the context erased; without it, a rename could land a real name back on an
    --    erased context, or a retirement write a real slug onto the ledger after the rewrite below.
    --    In id order, so two person acts do not deadlock on it.
    --
    --    Not at the act's start. A standalone resource erasure takes its resource's act queue, then
    --    its row, then its edges, and then, in steps (7) and (7b) of
    --    _resource_erasure_apply_redaction, an UPDATE of its home context's row (NO KEY UPDATE)
    --    when the context holds a shape watermark or a telos centroid. An act holding that context
    --    from its start and queued on the same resource closes a cycle: the act waits on the
    --    queue the standalone act holds, which waits on the context row the act holds. Taken here,
    --    after every resource and edge lock the act takes, the lock is behind them, as the
    --    standalone act's own context write is. The resource erasures above can still meet a
    --    standalone one on the pair: one whose step (7) or (7b) wrote a context's row holds it while
    --    the act queues on the next resource homed there. Postgres breaks that with 40P01 and the
    --    services retry both acts. A field scrub and a block history scrub take their resource's
    --    act queue and row in the same order, and neither writes a context row
    --    (_block_history_scrub_apply returns before step (7)), so neither closes that cycle.
    --
    --    So the estate can change between the plan and this lock: context_reassign moves a context
    --    out until the act first holds its row. For such a context the act's halves follow
    --    different estates (ruling 11 of decision 01a1f6e8-81bd-758c-be87-a6824b66352a, (a)):
    --      * the ledger rewrites follow the live estate: the edge step skips the edges its
    --        resource erasures did not reach (_edge_homed_in_estate), and _principal_erasure_redacted_fields leaves its earlier
    --        names and slugs (_principal_erasure_context_events reads the live estate);
    --      * the projection erasure follows the plan: the resource loop erases its resources (it
    --        re-checks a resource's home against the plan's estate), and
    --        _principal_erasure_contexts_to_erase gives it a context_erased, which retires it and
    --        sets its name and slug to the sentinels.
    PERFORM 1 FROM kb_contexts c
      WHERE c.id = ANY(v_estate)
      ORDER BY c.id
        FOR NO KEY UPDATE;
    v_ctx_erase  := _principal_erasure_contexts_to_erase(v_estate);
    -- The retention snapshot is the plan's: the policy as it stood when the act read it, recorded
    -- below so the redaction and replay keep exactly that.
    v_ctx_fields := _principal_erasure_redacted_fields(p_subject, v_estate, v_plan->'retained');

    -- ── The ONE completion event (D1), last in the record (2c, 2d). NULL-anchored (admin);
    -- emitter = the OPERATOR. It names the estate it reached (the redaction reads it back from
    -- here), the resource erasures it ran, in order, and the charters it holds. References carry
    -- the subject, the request, each resource erasure and then each edge erasure (rel `erasure`).
    -- The edge erasures are references only: the payload's keys are unchanged.
    v_refs := jsonb_build_array(
        jsonb_build_object('rel','subject',
            'target', jsonb_build_object('kind','kb_profiles','id', p_subject)),
        jsonb_build_object('rel','request',
            'target', jsonb_build_object('kind','kb_events','id', p_request_ref)))
        || coalesce((SELECT jsonb_agg(jsonb_build_object('rel','erasure',
                         'target', jsonb_build_object('kind','kb_events','id', e->>'event'))
                         ORDER BY ord)
                       FROM jsonb_array_elements(v_erasures) WITH ORDINALITY AS t(e, ord)),
                    '[]'::jsonb)
        || coalesce((SELECT jsonb_agg(jsonb_build_object('rel','erasure',
                         'target', jsonb_build_object('kind','kb_events','id', e->>'event'))
                         ORDER BY ord)
                       FROM jsonb_array_elements(v_edge_erasures) WITH ORDINALITY AS t(e, ord)),
                    '[]'::jsonb);
    v_payload := jsonb_build_object(
        'subject_table',     'kb_profiles',
        'subject_id',        p_subject,
        'actor',             p_operator,
        'redacted_hashes',   to_jsonb(v_hashes),
        'targets',           v_targets,
        'estate_contexts',   v_plan->'estate_contexts',
        'resource_erasures', v_erasures,
        'charters_held',     v_plan->'charters',
        'redacted_fields',   v_ctx_fields,
        'retained',          v_plan->'retained');
    v_ev := _event_append(
        'principal_erased', p_emitter, NULL, NULL, v_payload,
        p_references => v_refs,
        p_correlation => p_request_ref);
    -- The act's leave to rewrite the estate contexts' ledger copies (R7): one
    -- kb_event_field_redactions row per (event, path), authority `principal`. Replay calls the same
    -- projector at the event's position.
    PERFORM _project_principal_erased_redactions(v_ev, v_payload);

    -- ── The identity arms, the charters' text and the watermarks: the ONE definition, reading the
    --    estate and the retained categories from the event just appended.
    PERFORM _erasure_apply_redaction(p_subject, v_hashes, v_ev);

    -- ── The custody closure (R7): one context_erased per estate context not yet erased, in id
    --    order. It retires the context and replaces its name and slug with the sentinels; replay
    --    re-applies the same projector, so the closure no longer rests on the verbatim restore of
    --    kb_contexts. The payload carries only the sentinels: no from_* field, so no real name or
    --    slug is written to the ledger. Emitter = the operator; correlation = the request.
    FOREACH v_cid IN ARRAY v_ctx_erase LOOP
        v_cpayload := jsonb_build_object('context_id', v_cid,
                                         'to_name',    'erased',
                                         'to_slug',    'erased-' || v_cid::text);
        v_cev := _event_append('context_erased', p_emitter, 'kb_contexts', v_cid, v_cpayload,
                               p_correlation => p_request_ref);
        PERFORM _project_context_erased(v_cev, v_cpayload);
    END LOOP;

    -- ── The ledger rewrite (R7): every estate context's earlier name and slug, to the same
    --    sentinels. One statement, after the tombstone above, which the verifier requires of the
    --    `principal` authority, and after the redaction rows it reads.
    UPDATE kb_events e
       SET payload = _principal_erasure_payload_redaction(e.id, t.name, e.payload, f.paths)
      FROM jsonb_to_recordset(v_ctx_fields) AS f(event uuid, paths jsonb),
           kb_event_types t
     WHERE e.id = f.event
       AND t.id = e.event_type_id;

    RETURN jsonb_build_object(
        'event_id',          v_ev,
        'redacted_hashes',   to_jsonb(v_hashes),
        'targets',           v_targets,
        'already_erased',    (v_plan->>'already_erased')::boolean,
        'estate_contexts',   v_plan->'estate_contexts',
        'resource_erasures', v_erasures,
        'charters_held',     v_plan->'charters',
        'retained',          v_plan->'retained');
END;
$function$
;



COMMENT ON FUNCTION principal_erasure_execute(uuid, uuid, uuid, uuid) IS
$c$The person erasure act, in one transaction, from one plan (principal_erasure_survey_plan): every
resource homed in the subject's estate (_erasure_estate_contexts) through resource_erasure_execute,
in id order; then every edge homed in the estate whose text is not yet erased
(_edge_erasure_complete), live or folded before the act, through edge_erasure_execute, in id order,
skipping and counting an edge whose context has left the estate since the plan; the strike of every
live blob homed in the estate; the one principal_erased record, which references each resource and
edge erasure; the identity arms (_erasure_apply_redaction), which tombstone the subject; and one
context_erased per estate context, with the ledger rewrite of their names and slugs. Runs with
lock_timeout = 0 and search_path = public, pg_temp whatever the session, role or database sets.
The estate contexts are locked after the strikes. A context reassigned out of the estate before
the act holds its row, and before the edge step finds an edge to erase homed in it, keeps the
edges its resource erasures did not reach and its earlier names and slugs on the ledger, while its
resources are erased and it is closed by context_erased. One reassigned out after the edge step
found such an edge aborts the act (in edge_erasure_execute's home check, or at commit in
edge_erasure_requires_an_erased_owner); nothing persists, and the act run again completes. Content
committed into a retired estate context after the act is a fresh live row that this act does not
erase. Does not decide legality: the caller's system-admin gate is resolved before any
mutation.$c$;

-- ---------------------------------------------------------------------------
-- Section 7. The triggers, under the lock bound.
-- ---------------------------------------------------------------------------
SET LOCAL lock_timeout = '500ms';

COMMENT ON COLUMN kb_event_field_redactions.authority IS
$c$The authorising event's type: `erasure` (resource_erased, whose subject must be erased), `scrub`
(resource_scrubbed, whose subject must not be), `principal` (principal_erased, whose subject
profile must be tombstoned; context names and slugs, principal reasons and Slack ids only) or
`edge` (edge_erased, whose subject edge must be folded and homed in a context of the estate its
record names, and whose estate_of must be tombstoned at commit,
edge_erasure_requires_an_erased_owner). Written by that type's projector. A structural token,
never a value. The DEFAULT 'erasure' is the backfill and the reading of a binary that names no
authority; a writer must never lean on it for another authority. A row mislabelled `erasure` would occupy the key (event_id, path,
'erasure'): a later erasure of that path would collide on kb_event_field_redactions_pkey, which
resource_erasure_service reads as a concurrent completion and retries, colliding again every
time.$c$;

CREATE CONSTRAINT TRIGGER trg_edge_erased_requires_an_erased_owner
    AFTER INSERT ON kb_events
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    WHEN (NEW.category = 'admin')
    EXECUTE FUNCTION edge_erasure_requires_an_erased_owner();

COMMENT ON TRIGGER trg_edge_erased_requires_an_erased_owner ON kb_events IS
$c$At commit, an edge_erased record's estate_of must name a tombstoned profile, and its kb_edges
subject must be folded and homed in a context of that estate (_edge_homed_in_estate)
(edge_erasure_requires_an_erased_owner). Fires for every admin event; the function confirms the
type. Stands down while temper.replaying is 'on'.$c$;

CREATE CONSTRAINT TRIGGER trg_edge_redaction_requires_an_erased_owner
    AFTER INSERT ON kb_event_field_redactions
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    WHEN (NEW.authority = 'edge')
    EXECUTE FUNCTION edge_erasure_requires_an_erased_owner();

COMMENT ON TRIGGER trg_edge_redaction_requires_an_erased_owner ON kb_event_field_redactions IS
$c$At commit, an `edge` row's redacted_by must be an edge_erased record whose estate_of names a
tombstoned profile and whose kb_edges subject is folded and homed in a context of that estate
(_edge_homed_in_estate) (edge_erasure_requires_an_erased_owner). Stands down while
temper.replaying is 'on'.$c$;

-- A tombstone is irreversible. The owner check holds the `edge` authority to a tombstoned estate
-- owner at the moment it runs, and SET CONSTRAINTS ALL IMMEDIATE runs it mid-transaction: a
-- transaction could set tombstoned_at, call the unit, run the check, clear tombstoned_at and
-- commit, an edge erasure of a live owner. So an UPDATE that changes a set tombstoned_at, to NULL
-- or to another time, is refused. The person act's identity arm writes
-- COALESCE(tombstoned_at, its occurred_at), which keeps a set value, and that is the only write of
-- the column; replay restores kb_profiles by INSERT and re-runs that arm over the restored value.
CREATE FUNCTION kb_profiles_tombstone_is_irreversible()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    RAISE EXCEPTION 'profile % is tombstoned; its tombstone is irreversible', OLD.id
        USING ERRCODE = 'check_violation';
END;
$$;

COMMENT ON FUNCTION kb_profiles_tombstone_is_irreversible() IS
$c$Raises 'profile % is tombstoned; its tombstone is irreversible' (check_violation). Behind
kb_profiles_tombstone_is_irreversible, which fires it for an UPDATE that changes a kb_profiles
row's set tombstoned_at.$c$;

CREATE TRIGGER kb_profiles_tombstone_is_irreversible
    BEFORE UPDATE OF tombstoned_at ON kb_profiles
    FOR EACH ROW
    WHEN (OLD.tombstoned_at IS NOT NULL AND NEW.tombstoned_at IS DISTINCT FROM OLD.tombstoned_at)
    EXECUTE FUNCTION kb_profiles_tombstone_is_irreversible();

COMMENT ON TRIGGER kb_profiles_tombstone_is_irreversible ON kb_profiles IS
$c$Refuses an UPDATE that clears or changes a set tombstoned_at: a tombstone, once written, is
final, so the edge authority's owner check (edge_erasure_requires_an_erased_owner) cannot be met by
a tombstone set and then cleared in one transaction. Writing the same value, as the person act's
COALESCE(tombstoned_at, …) does on a tombstoned subject, passes.$c$;

SELECT declare_migration(
    20261202000040,
    'additive',
    'Adds the functions _edge_projection_erased(uuid), _edge_erasure_complete(uuid) and '
    '_principal_erasure_edges_outcome(integer), each with a COMMENT, and the trigger function '
    'edge_erasure_requires_an_erased_owner() with a '
    'COMMENT and two DEFERRABLE INITIALLY DEFERRED constraint triggers behind it, on kb_events '
    '(WHEN category = admin) and on kb_event_field_redactions (WHEN authority = edge), each with a '
    'COMMENT, and the trigger kb_profiles_tombstone_is_irreversible with its function, each with a '
    'COMMENT, which refuses an UPDATE that changes a set kb_profiles.tombstoned_at (the one write '
    'of the column, the person act''s COALESCE(tombstoned_at, occurred_at), never does), all '
    'created under SET LOCAL lock_timeout = 500ms: if a lock is not granted in time, the '
    'migration rolls back and the serving deployment is untouched. At commit an edge_erased record, '
    'or an edge row''s record, must name a tombstoned estate_of and a folded edge homed in a context '
    'of that estate; no edge_erased record or edge row '
    'exists when this migration runs, and any other admin event passes, so no existing write is '
    'refused. CREATE OR REPLACE, each signature and return type unchanged: '
    'edge_erasure_execute(uuid, uuid, uuid, uuid, uuid), SET clauses unchanged, whose '
    'already-erased test reads _edge_projection_erased and its own derivation and answers as '
    'before, and whose COMMENT names the owner check; principal_erasure_survey_plan(uuid), which '
    'predicts the edge step as one kb_edges target, counts the trails of edges homed in the estate '
    'inside it, and names as kept only a charter''s edges homed outside the estate; and '
    'principal_erasure_execute(uuid, uuid, uuid, uuid), whose COMMENT is rewritten, which gains '
    'SET lock_timeout = 0, as the other erasure acts carry, and which runs edge_erasure_execute '
    'for every edge homed in the estate whose text is not yet erased, after its resource erasures '
    'and before its record, passing over an edge touching a resource it erased, skipping and '
    'counting one whose context has left the estate since the plan: a person erasure now erases '
    'those edges, and its record gains a kb_edges target and one erasure reference per edge. Its '
    'estate contexts are still locked after its resource erasures and its edge step, so a context '
    'reassigned out of the estate before the act holds its row, and before the edge step finds an '
    'edge to erase homed in it, keeps its edges and its earlier names and slugs on the ledger, '
    'while its resources are erased and it is closed by context_erased; one reassigned out after '
    'the edge step found such an edge aborts the act (the unit''s home check or the commit-time '
    'owner check), which the service answers with a 500, nothing persisting, and the act run again '
    'completes. '
    'principal_erasure_execute, principal_erasure_survey_plan and _principal_erasure_edges_outcome '
    'pin search_path = public, pg_temp; every name they read is in public or schema-qualified. '
    'The record''s payload keys are unchanged. An older binary '
    'calls the same signatures and reads the same payload keys; its survey and act answer with '
    'the added kb_edges target. Rewrites the COMMENT of kb_event_field_redactions.authority. No '
    'table, column, event type or grant changes.'
);
