-- ============================================================================
-- Auto-join membership materializes at the standing committer (additive:
-- CREATE OR REPLACE x2, one new function, COMMENT updates).
-- ----------------------------------------------------------------------------
-- INVARIANT (20260629000002's header): a kb_teams row with `auto_join_role`
-- is an always-complete "everyone pool" — membership ⊇ {profiles where
-- has_system_access}. Since 20260722000100 dropped trg_sync_system_membership,
-- enrollment lived ONLY in review_request's approval arm: the direct-grant
-- door (admin_approve), admin promotion, reactivation and the boot-seed all
-- left the pool incomplete, with no signal anywhere.
--
-- The fix materializes the enrollment arm at the ONE committer every standing
-- transition routes through (20260720000030: "ONE COMMITTER, NOT NINE"), so
-- the standing write and its roster consequence are atomic by construction
-- and no door can forget it.
--
-- ENROLLMENT ONLY, and this is decided, not inherited:
--   - enrollment is ON CONFLICT DO NOTHING (matches backfill_auto_join_team;
--     makes 20260722000010's "existing owner rows are NOT rewritten" posture
--     permanent rather than incidental; the old DO UPDATE rewrote an existing
--     row's role on every re-enroll, clobbering admin-authored grants);
--   - the pool is APPEND-ONLY: no standing transition ever deletes an
--     auto-join membership. The dropped trigger's ELSE DELETE belonged to the
--     pre-D11 model where gating-team membership WAS system access — its
--     cleanup semantics retired with the column. Post-D11, memberships are
--     owned by authorities other than the standing committer, and each has a
--     documented, tested claim that outlives this fix:
--       * D14 machine hygiene: machine_registration_service enrolls a
--         born-`denied` machine into the gating team when its minter is a
--         member (`enroll_in_gating_team`) — a standing write must not undo
--         it;
--       * D17/D11 revocation intent: credential revocation deliberately
--         leaves grants AND memberships untouched, so a rebind cannot
--         silently resurrect or silently lose them
--         (machine_client_service::revoke);
--       * IdP provenance: `source = 'idp'` rows are owned by
--         reconcile_idp_memberships (20260702000001), which skips any
--         (team, profile) pair the profile holds natively — deleting one
--         silently converts SAML authority into a native row the IdP never
--         reasserts. The mirror holds at INSERT time: enrollment writes
--         NATIVE rows, so on a team that also carries a SAML group mapping,
--         the first approval — or one reconcile run over a drifted
--         instance — permanently pre-empts the IdP's later role assertions
--         for that pair (native-wins-skip: the IdP can neither set nor
--         revoke that membership again; team role drives reach). This is
--         the decided DO-NOTHING posture made legible, not new semantics —
--         the request-review door always had this property; this fix
--         widens it to every door and mass-creates the conversion at
--         reconcile time. The reconcile verb therefore names any touched
--         team that carries SAML group mappings in its outcome and warns
--         on the server log, so an operator mapping IdP groups onto an
--         auto-join team sees the conversion the repair is making.
--     The invariant is therefore one-directional by design: every
--     standing-approved profile is a member (this fix); the converse holds
--     only among transitions this committer saw. Stale rows for revoked or
--     deactivated principals are harmless under D18 (membership confers no
--     access; admission is denied at every surface) — "always-complete"
--     reads as "complete among the standing-approved". An operator converges
--     history with auto_join_reconcile() below.
--   - the roster writes emit NO events, deliberately: un-evented by the same
--     precedent as 20260911000000 arm 13, and replay-safe because
--     kb_team_members is a replay INPUT table (restored verbatim) and the
--     principal_standing_changed walk arm is a no-op — this arm never
--     re-executes during replay.
--
-- auto_join_reconcile() converges an instance that drifted before this
-- migration (profiles approved through the un-enrolling doors) and reports
-- each (team, profile) pair it added — the operator repair path. Approved
-- MACHINE principals enroll like any other profile (D2 makes no human/machine
-- split; eligibility is has_system_access) — on a drifted instance one
-- reconcile widens machine read reach into the pool; the report is the
-- operator's review artifact.
-- ============================================================================

-- Enroll an eligible profile (has_system_access) into EVERY auto-join team at
-- the team's auto_join_role. ON CONFLICT DO NOTHING: enrollment defers to
-- explicit state — an existing row at any role, native or IdP-authored, is
-- never rewritten.
CREATE OR REPLACE FUNCTION ensure_auto_join_memberships(p_profile uuid)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    IF NOT has_system_access(p_profile) THEN
        RETURN;  -- not eligible (standing not approved); enroll nothing
    END IF;
    INSERT INTO kb_team_members (team_id, profile_id, role)
    SELECT t.id, p_profile, t.auto_join_role
      FROM kb_teams t
     WHERE t.auto_join_role IS NOT NULL
    ON CONFLICT (team_id, profile_id) DO NOTHING;
END;
$$;

COMMENT ON FUNCTION ensure_auto_join_memberships IS
  'Enroll an eligible profile (has_system_access) into EVERY auto-join team at '
  'the team''s auto_join_role. ON CONFLICT DO NOTHING: enrollment defers to '
  'explicit state — an existing row at any role is never rewritten. Called at '
  'the standing committer; not a decision point.';

-- The standing committer now materializes the auto-join enrollment in the
-- same transaction — body otherwise verbatim from 20260720000030.
CREATE OR REPLACE FUNCTION principal_standing_apply(
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

    -- Materialize the auto-join enrollment in this same transaction: the
    -- standing write and its roster consequence commit together or not at
    -- all. ENROLLMENT ONLY — no standing transition ever removes an
    -- auto-join membership; those are owned by D14 machine hygiene, D17
    -- revocation intent, and IdP provenance (header).
    PERFORM ensure_auto_join_memberships(p_profile);

    RETURN p_resulting;
END;
$$;

COMMENT ON FUNCTION principal_standing_apply IS
  'The ONE writer of kb_principal_standing. Commits row + log + ledger event, '
  'plus the auto-join enrollment (members of the pool while '
  'has_system_access; never removes — memberships are owned by D14/D17/IdP, '
  'not by standing), in one transaction (spec §10, D4). Does NOT decide '
  'legality -- temper-principal does, and duplicating that here would create '
  'two transition tables in two languages.';

-- The operator repair path: converge every auto-join team to the
-- standing-approved population and report each pair added. Idempotent — a
-- converged instance reconciles to zero rows; ON CONFLICT keeps a race with
-- a concurrent standing transition a quiet no-op instead of a 23505.
CREATE FUNCTION auto_join_reconcile()
RETURNS TABLE (team_slug text, profile_handle text)
LANGUAGE sql AS $$
    WITH ins AS (
        INSERT INTO kb_team_members (team_id, profile_id, role)
        SELECT t.id, p.id, t.auto_join_role
          FROM kb_teams t
         CROSS JOIN kb_profiles p
         WHERE t.auto_join_role IS NOT NULL
           AND has_system_access(p.id)
           AND NOT EXISTS (
               SELECT 1 FROM kb_team_members m
                WHERE m.team_id = t.id AND m.profile_id = p.id)
        ON CONFLICT (team_id, profile_id) DO NOTHING
        RETURNING team_id, profile_id
    )
    SELECT t.slug, pr.handle
      FROM ins
      JOIN kb_teams t     ON t.id = ins.team_id
      JOIN kb_profiles pr ON pr.id = ins.profile_id
     ORDER BY t.slug, pr.handle;
$$;

COMMENT ON FUNCTION auto_join_reconcile IS
  'Converge every auto-join team to the standing-approved population; returns '
  'each (team, profile) pair this call added. Adds only: an existing row at '
  'any role, and IdP-authored rows, are never rewritten or removed.';

SELECT declare_migration(
    20260923000010,
    'additive',
    'Auto-join membership materializes at the standing committer. Since 20260722000100 '
    'dropped trg_sync_system_membership, the "always-complete everyone pool" '
    '(20260629000002''s own invariant) grew only through the request-review door: direct '
    'grants, promotions and reactivations conferred standing while enrolling nowhere, so '
    'approved principals drifted out of every auto-join team with no signal. The fix '
    'materializes the enrollment arm inside principal_standing_apply — the ONE committer '
    'every standing transition routes through (20260720000030) — so the standing write '
    'and its roster consequence are atomic by construction and no door can forget it. '
    'ENROLLMENT ONLY, decided against the delete arm the dropped trigger also carried: '
    'post-D11, auto-join memberships are owned by authorities other than standing — D14 '
    'machine hygiene enrolls born-denied machines into the gating team, D17/D11 '
    'revocation deliberately preserves grants and memberships for the rebind story, and '
    'IdP-source rows are owned by reconcile_idp_memberships (20260702000001) — so a '
    'standing transition removes nothing, and the invariant reads one-directional '
    '(membership ⊇ {profiles where has_system_access}); stale rows for '
    'no-longer-approved principals are harmless under D18. Enrollment is ON CONFLICT DO '
    'NOTHING: derivation defers to explicit state, making 20260722000010''s "existing '
    'owner rows are NOT rewritten" posture permanent. ensure_auto_join_memberships is '
    'OR-REPLACEd with the DO NOTHING conflict arm; principal_standing_apply is '
    'OR-REPLACEd with the enrollment appended, body otherwise verbatim from '
    '20260720000030; auto_join_reconcile is NEW — the operator repair path that '
    'converges an instance drifted before this change and reports each pair it added '
    '(served by admin access reconcile-auto-join / '
    'POST /api/access/admin/auto-join/reconcile). Safe ahead of the paired binary: '
    'CREATE OR REPLACE keeps every existing signature (ensure, '
    'principal_standing_apply) so older binaries calling them are unaffected in shape, '
    'and the new function is additive; a pre-pairing binary simply does not call the '
    'reconcile function and writes standing with the enrollment already firing.'
);
