-- The grant chokepoint, SQL-resident (spec 2026-07-16 §7).
--
-- WHY SQL AND NOT RUST: cognition events are not fired from Rust alongside a Rust write --
-- fire() dispatches a SeedAction to a SQL function that appends the event AND projects, in one
-- txn (_event_append, canonical_functions.sql:765). Admin acts follow the same shape. A
-- Rust service-layer sink would also MISS connection_service::grant_reach, which bypasses
-- grant_capability and calls insert_grant directly (connection_service.rs:467).
--
-- BOTH-NULL PRODUCING ANCHOR, always. A grant is an authority act; it has no cognition home even
-- when its subject IS a context. Anchoring it would put it in front of every region producer and
-- break the "governance is traceable, but it isn't knowledge" boundary.
--
-- The payload spells the subject `subject_table`/`subject_id`, NEVER `resource_id`/`owner`:
-- element_trail_node/_edge match on payload key shape with no type filter and are gated only by
-- resources_visible_to, so those keys would leak the grant into any reader's element trail (spec §5,
-- tested). The payload is the typed temper_substrate::payloads::GrantCreated / GrantRevoked wire
-- contract — verify_ledger_roundtrip deserializes every one of these events into that struct.

CREATE FUNCTION _admin_grant_created(
    p_emitter         uuid,
    p_subject_table   text,
    p_subject_id      uuid,
    p_principal_table text,
    p_principal_id    uuid,
    p_can_read        boolean,
    p_can_write       boolean,
    p_can_delete      boolean,
    p_can_grant       boolean,
    p_granted_by      uuid,
    p_correlation     uuid DEFAULT NULL
) RETURNS boolean
LANGUAGE plpgsql AS $$
DECLARE
    v_prev     jsonb := NULL;
    v_inserted boolean;
    v_payload  jsonb;
BEGIN
    -- Capture the prior capabilities BEFORE the upsert overwrites them. An upsert that changes
    -- capabilities returns inserted = false, so keying emission on that bool alone would silently
    -- drop a real authority change. The event carries before/after instead.
    SELECT jsonb_build_object('can_read', can_read, 'can_write', can_write,
                              'can_delete', can_delete, 'can_grant', can_grant)
      INTO v_prev
      FROM kb_access_grants
     WHERE subject_table = p_subject_table AND subject_id = p_subject_id
       AND principal_table = p_principal_table AND principal_id = p_principal_id;

    INSERT INTO kb_access_grants
        (subject_table, subject_id, principal_table, principal_id,
         can_read, can_write, can_delete, can_grant, granted_by_profile_id)
    VALUES (p_subject_table, p_subject_id, p_principal_table, p_principal_id,
            p_can_read, p_can_write, p_can_delete, p_can_grant, p_granted_by)
    ON CONFLICT (subject_table, subject_id, principal_table, principal_id)
    DO UPDATE SET can_read = EXCLUDED.can_read, can_write = EXCLUDED.can_write,
                  can_delete = EXCLUDED.can_delete, can_grant = EXCLUDED.can_grant,
                  granted_by_profile_id = EXCLUDED.granted_by_profile_id, granted_at = now()
    RETURNING (xmax = 0) INTO v_inserted;

    v_payload := jsonb_build_object(
        'subject_table', p_subject_table, 'subject_id', p_subject_id,
        'principal_table', p_principal_table, 'principal_id', p_principal_id,
        'can_read', p_can_read, 'can_write', p_can_write,
        'can_delete', p_can_delete, 'can_grant', p_can_grant,
        'granted_by', p_granted_by);
    IF v_prev IS NOT NULL THEN
        v_payload := v_payload || jsonb_build_object('previous', v_prev);
    END IF;

    -- Emit UNCONDITIONALLY -- unlike _admin_grant_revoked, which suppresses no-op deletes. A re-grant
    -- with IDENTICAL capabilities is not a no-op: the upsert above still refreshes granted_by_profile_id
    -- and granted_at, so it is a real re-affirmation worth recording. The asymmetry is deliberate: a
    -- consumer reads `previous` to learn WHAT changed (present-and-equal ⇒ a re-affirm, absent ⇒ a
    -- fresh grant), rather than treating every grant_created as a capability change.
    PERFORM _event_append(
        'grant_created', p_emitter, NULL, NULL, v_payload,
        p_references => jsonb_build_array(
            jsonb_build_object('rel','subject',  'target', jsonb_build_object('kind', p_subject_table,   'id', p_subject_id)),
            jsonb_build_object('rel','principal','target', jsonb_build_object('kind', p_principal_table, 'id', p_principal_id))),
        p_correlation => p_correlation);

    RETURN v_inserted;
END;
$$;

CREATE FUNCTION _admin_grant_revoked(
    p_emitter         uuid,
    p_subject_table   text,
    p_subject_id      uuid,
    p_principal_table text,
    p_principal_id    uuid,
    p_revoked_by      uuid,
    p_correlation     uuid DEFAULT NULL
) RETURNS boolean
LANGUAGE plpgsql AS $$
DECLARE
    v_deleted boolean := false;
BEGIN
    DELETE FROM kb_access_grants
     WHERE subject_table = p_subject_table AND subject_id = p_subject_id
       AND principal_table = p_principal_table AND principal_id = p_principal_id;
    GET DIAGNOSTICS v_deleted = ROW_COUNT;
    v_deleted := (v_deleted::int > 0);

    -- Emit only when something was actually revoked: a no-op revoke is not an admin act, and the
    -- ledger is append-only -- a spurious row can never be corrected, only quarantined.
    IF v_deleted THEN
        PERFORM _event_append(
            'grant_revoked', p_emitter, NULL, NULL,
            jsonb_build_object(
                'subject_table', p_subject_table, 'subject_id', p_subject_id,
                'principal_table', p_principal_table, 'principal_id', p_principal_id,
                'revoked_by', p_revoked_by),
            p_references => jsonb_build_array(
                jsonb_build_object('rel','subject',  'target', jsonb_build_object('kind', p_subject_table,   'id', p_subject_id)),
                jsonb_build_object('rel','principal','target', jsonb_build_object('kind', p_principal_table, 'id', p_principal_id))),
            p_correlation => p_correlation);
    END IF;

    RETURN v_deleted;
END;
$$;

COMMENT ON FUNCTION _admin_grant_created IS
  'Grant upsert + grant_created event, one txn. Both-NULL producing anchor: a grant is an authority act with no cognition home, even when its subject is a context. Carries `previous` when it replaced an existing grant -- an upsert that changes capabilities returns inserted=false, so the bool alone would drop a real authority change.';

COMMENT ON FUNCTION _admin_grant_revoked IS
  'Grant DELETE + grant_revoked event, one txn. The DELETE stays: the row is the current-state projection, the ledger is the temporal record (access spec §3.7). Emits only when a row was actually deleted -- kb_events is append-only and a spurious event is immortal.';
