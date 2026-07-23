-- The principal-admission committer (spec 2026-07-20 §10, D4).
--
-- WHY SQL AND NOT RUST: production event emission is SQL-resident in this repo. There are ZERO
-- production `INSERT INTO kb_events` statements in crates/ -- the four grep hits are all inside
-- #[cfg(all(test, feature = "test-db"))] modules -- and substrate's events.rs describes itself as
-- the firing surface for "seeding, scenario loading, and tests", while "the SQL functions stay the
-- atomic event+materialize+commit mechanism". Admission acts follow that shape.
--
-- ONE COMMITTER, NOT NINE. §10 says "one SQL function per transition". Taken literally that is
-- nine functions differing by a string literal -- the enumerate-don't-compose shape. What §10 is
-- BUYING is atomicity: a standing change without its audit record must not be representable. One
-- function that always writes all three in one statement buys exactly that, in one place rather
-- than nine that can drift.
--
-- THIS FUNCTION DOES NOT DECIDE LEGALITY, AND MUST NEVER START. The transition table lives in
-- temper-principal (Rust, exhaustive, no catchall) and is tested as a pure matrix with no
-- database. If this function grows a legality check there are two transition tables in two
-- languages and they will disagree -- which is the class of bug this entire design removes. The
-- Rust machine judges; SQL commits.
--
-- BOTH-NULL PRODUCING ANCHOR, always. An admission act is an authority act; it has no cognition
-- home. Anchoring it would put it in front of every region producer and break the "governance is
-- traceable, but it isn't knowledge" boundary. kb_events_admin_is_unanchored enforces it.
--
-- Template: 20260718000010_admin_grant_fns.sql and 20260719000020_slack_disconnect_event.sql:144.

CREATE FUNCTION principal_standing_apply(
    p_profile   uuid,
    p_act       text,
    p_resulting text,
    p_actor     uuid    DEFAULT NULL,
    p_reason    text    DEFAULT NULL
) RETURNS text
LANGUAGE plpgsql AS $$
DECLARE
    v_prior   text;
    v_emitter uuid;
BEGIN
    -- The prior state, captured BEFORE the upsert -- it is the log's whole value.
    SELECT state INTO v_prior FROM kb_principal_standing WHERE profile_id = p_profile;

    INSERT INTO kb_principal_standing (profile_id, state, updated)
    VALUES (p_profile, p_resulting, now())
    ON CONFLICT (profile_id) DO UPDATE
      SET state = EXCLUDED.state, updated = EXCLUDED.updated;

    INSERT INTO kb_principal_standing_events
        (profile_id, act, prior_state, resulting_state, actor_profile_id, reason)
    VALUES (p_profile, p_act, v_prior, p_resulting, p_actor, p_reason);

    -- Events need a NOT NULL emitter. Prefer the acting principal's emitter entity; fall back to
    -- the canonical `system` actor, which bootseed.rs:31 guarantees exists.
    SELECT id INTO v_emitter FROM kb_entities
     WHERE profile_id = COALESCE(p_actor, p_profile) LIMIT 1;
    IF v_emitter IS NULL THEN
        SELECT e.id INTO v_emitter
          FROM kb_entities e JOIN kb_profiles pr ON pr.id = e.profile_id
         WHERE pr.handle = 'system' LIMIT 1;
    END IF;
    IF v_emitter IS NULL THEN
        RAISE EXCEPTION 'no emitter entity available for a principal-standing event (profile %)', p_profile;
    END IF;

    PERFORM _event_append(
        'principal_standing_changed', v_emitter, NULL, NULL,
        jsonb_strip_nulls(jsonb_build_object(
            'subject_table', 'kb_profiles',
            'subject_id',    p_profile,
            'act',           p_act,
            'prior',         v_prior,
            'resulting',     p_resulting,
            'actor',         p_actor,
            'reason',        p_reason)),
        p_references => jsonb_build_array(
            jsonb_build_object('rel','subject',
                'target', jsonb_build_object('kind','kb_profiles','id', p_profile))));

    RETURN p_resulting;
END;
$$;

COMMENT ON FUNCTION principal_standing_apply IS
  'The ONE writer of kb_principal_standing. Commits row + log + ledger event in one transaction '
  '(spec §10, D4). Does NOT decide legality -- temper-principal does, and duplicating that here '
  'would create two transition tables in two languages.';

-- ---------------------------------------------------------------------------------------------
-- What `Reactivate` restores from (spec §5: "restores rather than guesses").
--
-- Returns the resulting_state of the most recent entry BEFORE the deactivation, or NULL when
-- there is nothing to restore -- in which case the Rust machine refuses (Refusal::NoPriorStanding)
-- rather than defaulting. NULL here is a decision, not an accident.
--
-- Backfilled rows would otherwise always hit the NULL arm, since the log begins at migration time.
-- 20260720000050's genesis pass writes a synthetic entry for exactly that reason.
-- ---------------------------------------------------------------------------------------------
CREATE FUNCTION principal_prior_standing(p_profile uuid) RETURNS text
LANGUAGE sql STABLE AS $$
    SELECT prior_state
      FROM kb_principal_standing_events
     WHERE profile_id = p_profile
       AND act = 'deactivate'
     ORDER BY occurred_at DESC
     LIMIT 1
$$;

COMMENT ON FUNCTION principal_prior_standing IS
  'The state immediately before the most recent deactivation (spec §5). NULL means nothing to '
  'restore, and the Rust machine refuses with NoPriorStanding rather than defaulting.';

-- ---------------------------------------------------------------------------------------------
-- Governance (D10). Idempotent, and emits ONLY on a real change: a no-op is not an admin act, and
-- the ledger is append-only -- a spurious row can never be corrected, only quarantined.
-- ---------------------------------------------------------------------------------------------
CREATE FUNCTION principal_governance_set(
    p_profile uuid,
    p_granted boolean,
    p_actor   uuid DEFAULT NULL,
    p_reason  text DEFAULT NULL
) RETURNS boolean
LANGUAGE plpgsql AS $$
DECLARE
    v_rows    integer := 0;
    v_changed boolean := false;
    v_emitter uuid;
BEGIN
    IF p_granted THEN
        INSERT INTO kb_principal_governance (profile_id, granted_by)
        VALUES (p_profile, p_actor)
        ON CONFLICT (profile_id) DO NOTHING;
        GET DIAGNOSTICS v_rows = ROW_COUNT;
    ELSE
        DELETE FROM kb_principal_governance WHERE profile_id = p_profile;
        GET DIAGNOSTICS v_rows = ROW_COUNT;
    END IF;
    v_changed := (v_rows > 0);

    IF v_changed THEN
        SELECT id INTO v_emitter FROM kb_entities
         WHERE profile_id = COALESCE(p_actor, p_profile) LIMIT 1;
        IF v_emitter IS NULL THEN
            SELECT e.id INTO v_emitter FROM kb_entities e
              JOIN kb_profiles pr ON pr.id = e.profile_id WHERE pr.handle = 'system' LIMIT 1;
        END IF;
        IF v_emitter IS NULL THEN
            RAISE EXCEPTION 'no emitter entity available for a governance event (profile %)', p_profile;
        END IF;

        PERFORM _event_append(
            'principal_governance_changed', v_emitter, NULL, NULL,
            jsonb_strip_nulls(jsonb_build_object(
                'subject_table', 'kb_profiles',
                'subject_id',    p_profile,
                'change',        CASE WHEN p_granted THEN 'granted' ELSE 'revoked' END,
                'actor',         p_actor,
                'reason',        p_reason)),
            p_references => jsonb_build_array(
                jsonb_build_object('rel','subject',
                    'target', jsonb_build_object('kind','kb_profiles','id', p_profile))));
    END IF;

    RETURN v_changed;
END;
$$;

COMMENT ON FUNCTION principal_governance_set IS
  'Grant or revoke the authority to change the rules (spec D10). Idempotent; emits only on a real '
  'change. INVARIANT (enforced by callers): admin implies standing = approved.';
