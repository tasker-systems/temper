-- Backfill standing, governance, and a genesis log (spec 2026-07-20 §11, D8).
--
-- Numbered …000120 to follow the repoint at …000110 (see that migration for why the repoint sits
-- above 20260720000100). Tasks 7 and 8 must land in the same PR and, on a live instance, the same
-- deploy: until this backfill runs, every existing principal has no standing row and is therefore
-- denied by the repointed predicate.
--
-- BACKFILL BY EVALUATING THE OLD PREDICATE, NOT BY READING THE TIER (D8). A tier-based backfill
-- would silently lock out anyone whose access comes entirely from gating-team membership with
-- system_access = 'none' -- confirmed on temperkb.io as exactly the `anonymous` row, at exactly
-- the cardinality §11 predicted.
--
-- THE OLD PREDICATE IS INLINED HERE, NOT CALLED. 20260720000110 already replaced
-- has_system_access's body with the standing read, and migrations apply in filename order -- so
-- calling it here would read the new body and backfill every principal to `denied`. The logic
-- below is the pre-cutover body, transcribed from 20260624000002_canonical_functions.sql:1388.
--
-- RULES ARE ORDERED; FIRST MATCH WINS. Rules 1 and 2 both match a deactivated principal whose old
-- predicate is true, and the ordering IS the decision: Deactivated wins, because D6 folds
-- is_active in and a principal who is disabled is disabled. The cost is stated honestly in §11 --
-- the predicate flips true->false for those principals, deliberately -- and auth-observable
-- behaviour is unaffected because gate_resolved_profile (auth/mod.rs:246) rejects !is_active at
-- Level 1 and the type-state makes reaching Level 2 without Level 1 impossible.

-- The old predicate's verdict, materialized before anything is written.
CREATE TEMP TABLE _old_verdict ON COMMIT DROP AS
WITH settings AS (
    SELECT access_mode, gating_team_slug FROM kb_system_settings LIMIT 1
)
SELECT p.id AS profile_id,
       p.is_active,
       (SELECT CASE
            WHEN s.access_mode = 'open' THEN true
            WHEN s.access_mode = 'invite_only' THEN EXISTS (
                SELECT 1 FROM kb_team_members tm
                  JOIN kb_teams t ON t.id = tm.team_id
                 WHERE tm.profile_id = p.id AND t.slug = s.gating_team_slug)
            ELSE false
        END FROM settings s) AS old_access
  FROM kb_profiles p
 -- RULE 0 (D7): connection profiles get NO ROW AT ALL. Not optional -- under access_mode='open'
 -- the old predicate is true for EVERY profile, so a literal per-profile backfill would mint
 -- connection profiles `approved` rows, contradicting D7 and dissolving the structural safety D7
 -- claims. There is no discriminator column on kb_profiles; kind is FK-inferable only.
 WHERE NOT EXISTS (SELECT 1 FROM kb_connections c WHERE c.profile_id = p.id);

-- Rules 1-3. Rule 3 is written `IS TRUE ... ELSE denied` rather than `false -> denied` so that
-- NULL is handled BY DECISION rather than by omission: the old predicate returns NULL when
-- kb_system_settings is empty, and a rule with only true/false arms would leave that case to
-- whatever this migration happened to do.
INSERT INTO kb_principal_standing (profile_id, state)
SELECT profile_id,
       CASE
           WHEN is_active = false      THEN 'deactivated'   -- rule 1
           WHEN old_access IS TRUE     THEN 'approved'      -- rule 2
           ELSE                             'denied'        -- rule 3, including NULL
       END
  FROM _old_verdict
ON CONFLICT (profile_id) DO NOTHING;

-- ---------------------------------------------------------------------------------------------
-- PASS 2 -- pending requests (§11).
--
-- The old predicate cannot see status = 'pending', so in-flight requests would backfill to
-- `denied` and silently lose their request-ness; `requested` would be unreachable by the backfill
-- as specified. temperkb.io has ZERO join requests so this is correctness-only there, but the
-- enterprise instance is unverified.
--
-- Only promotes rows currently `denied`: a pending request from an already-approved principal is
-- not evidence to downgrade them, and a deactivated principal stays deactivated (rule 1 wins).
-- ---------------------------------------------------------------------------------------------
UPDATE kb_principal_standing s
   SET state = 'requested'
  FROM kb_join_requests jr
 WHERE jr.requesting_profile_id = s.profile_id
   AND jr.status = 'pending'
   AND s.state = 'denied';

-- ---------------------------------------------------------------------------------------------
-- PASS 3 -- GOVERNANCE. The most important pass in this file.
--
-- NOT IN §11, because governance came into scope at D10 after §11 was written. Without it,
-- 20260720000110's repoint of is_system_admin DE-ADMINS EVERY EXISTING ADMIN -- and under D11 no
-- door grants access, so the instance would have zero admins and no way to make one. The operator
-- would be locked out of their own instance by a migration.
--
-- Existing admins are gating-team OWNERS under the LIVE (pre-cutover) is_system_admin body. That
-- body is NOT the one 20260624000002_canonical_functions.sql:1409 originally shipped: 20260720000100
-- added `AND gating_team_slug <> ''` to close a fall-open (an empty slug matched any team slugged
-- '', making its owners admins). This pass must snapshot the LIVE predicate, so it mirrors that
-- guard -- omit it and an instance with an empty gating slug would mint governance rows the live
-- is_system_admin would deny, over-granting admin in exactly the direction …000100 closed. prod is
-- unaffected (gating_team_slug = 'temper-system'); the enterprise instance is unverified, which is
-- why the guard is carried rather than assumed harmless.
--
-- granted_by is NULL: there is no actor to name for a schema change, and inventing one would put a
-- fabricated attribution on the ledger.
--
-- The `admin implies approved` invariant is asserted at the end of this file rather than assumed.
-- ---------------------------------------------------------------------------------------------
INSERT INTO kb_principal_governance (profile_id, granted_by)
SELECT tm.profile_id, NULL
  FROM kb_team_members tm
  JOIN kb_teams t ON t.id = tm.team_id
  JOIN kb_system_settings st ON st.gating_team_slug = t.slug
 WHERE tm.role = 'owner'
   AND st.gating_team_slug <> ''
ON CONFLICT (profile_id) DO NOTHING;

-- ---------------------------------------------------------------------------------------------
-- PASS 4 -- the synthetic genesis log entry (§11).
--
-- §5 promises Reactivate "restores rather than guesses", but the log begins at migration time, so
-- every backfilled `deactivated` row would have nothing to restore -- exactly the case §5 says
-- cannot happen. This writes the pre-deactivation standing, computed as RULE 2 EVALUATED IGNORING
-- RULE 1 (i.e. what the principal's standing would have been had they not been deactivated).
--
-- This matters more than it looks because the evidence is actively destroyed: sync_system_membership
-- DELETEs auto-join memberships whenever the predicate reads false, so once the predicate is
-- repointed a deactivated principal's gating-team membership -- the very thing their access was
-- derived from -- is gone and does not come back. This pass is the last moment that information
-- exists.
--
-- actor_profile_id is NULL for every backfilled row: a migration is not an actor.
-- ---------------------------------------------------------------------------------------------
INSERT INTO kb_principal_standing_events
    (profile_id, act, prior_state, resulting_state, actor_profile_id, reason)
SELECT v.profile_id,
       'provision',
       -- For a deactivated principal this is what Reactivate will restore.
       CASE WHEN v.is_active = false
            THEN CASE WHEN v.old_access IS TRUE THEN 'approved' ELSE 'denied' END
            ELSE NULL
       END,
       s.state,
       NULL,
       'backfilled at cutover (migration 20260720000120); no actor'
  FROM _old_verdict v
  JOIN kb_principal_standing s ON s.profile_id = v.profile_id;

-- A deactivated principal needs a `deactivate` entry too, or principal_prior_standing -- which
-- reads the most recent act='deactivate' row -- finds nothing and Reactivate refuses.
INSERT INTO kb_principal_standing_events
    (profile_id, act, prior_state, resulting_state, actor_profile_id, reason)
SELECT v.profile_id,
       'deactivate',
       CASE WHEN v.old_access IS TRUE THEN 'approved' ELSE 'denied' END,
       'deactivated',
       NULL,
       'backfilled at cutover; prior standing reconstructed from the pre-cutover predicate'
  FROM _old_verdict v
 WHERE v.is_active = false;

-- ---------------------------------------------------------------------------------------------
-- ASSERTIONS. A backfill that silently did nothing is worse than one that fails loudly.
-- ---------------------------------------------------------------------------------------------
DO $$
DECLARE
    v_profiles      bigint;
    v_connections   bigint;
    v_standing      bigint;
    v_bad_admin     bigint;
BEGIN
    SELECT count(*) INTO v_profiles FROM kb_profiles;
    SELECT count(DISTINCT profile_id) INTO v_connections FROM kb_connections;
    SELECT count(*) INTO v_standing FROM kb_principal_standing;

    IF v_standing <> v_profiles - v_connections THEN
        RAISE EXCEPTION
            'backfill covered % of % non-connection profiles; rule 0 or the insert is wrong',
            v_standing, v_profiles - v_connections;
    END IF;

    -- D7, asserted rather than assumed: connection profiles must have NO row.
    IF EXISTS (SELECT 1 FROM kb_principal_standing s
                JOIN kb_connections c ON c.profile_id = s.profile_id) THEN
        RAISE EXCEPTION 'a connection profile received a standing row -- D7 is dissolved';
    END IF;

    -- The §9 invariant: admin implies approved. If this fires, the governance pass admitted
    -- someone whose standing does not permit them to use the instance they would govern (e.g. a
    -- deactivated gating-team owner). Failing loud is deliberate: silently dropping such an admin
    -- is the exact "silent" behaviour §11 rejects, and this is a genuinely contradictory state a
    -- human must resolve, not a migration.
    SELECT count(*) INTO v_bad_admin
      FROM kb_principal_governance g
      LEFT JOIN kb_principal_standing s ON s.profile_id = g.profile_id
     WHERE s.state IS DISTINCT FROM 'approved';
    IF v_bad_admin > 0 THEN
        RAISE EXCEPTION
            '% governance rows whose standing is not approved -- "admin implies Approved" (§9) is violated',
            v_bad_admin;
    END IF;

    RAISE NOTICE 'principal standing backfilled: % rows (% profiles, % connection profiles excluded)',
        v_standing, v_profiles, v_connections;
END $$;
