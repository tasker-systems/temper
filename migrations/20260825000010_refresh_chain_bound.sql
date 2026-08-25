-- A refresh chain is bounded, and belongs to a principal who can end it.
--
-- Session-lifecycle hardening for the SAML authorization server. Two properties an operator should
-- be able to state about a long-lived credential: how long it can live at most, and who can end it.
-- `kb_oauth_refresh_tokens` (20260701000006:67-76) is given one column for each.
--
-- 1. `chain_expires_at` -- the ABSOLUTE bound. Stamped once when a chain is born (the
--    authorization_code grant) and inherited UNCHANGED by every successor, so the deadline is a
--    property of the CHAIN rather than of whichever token is currently held. That is what makes it
--    statable: it is measured from the last full SAML login, and only another login moves it.
--    It is also the arm that is independent of standing -- reconciliation against the IdP acts on
--    `source='idp'` team memberships and leaves standing `approved`, so a clock is what covers that
--    axis and an admission check cannot.
--
-- 2. `profile_id` -- the OWNER, so an administrator's revoke reaches the credential itself and not
--    only the API gate its tokens meet. Nullable, deliberately: the AS resolves it through the
--    internal resolve endpoint, and that call is fail-open on the login path (a provisioning hiccup
--    must never block authentication -- design spec 3.8). A NULL row is outside the reach of
--    `standing_service::apply`'s terminal hook and is held by `chain_expires_at` alone; the AS
--    re-resolves it on the next rotation, so a NULL heals rather than persisting for a whole chain.
--
-- BACKFILL. Rows already on disk carry no recorded chain origin -- `created` is this token's issue
-- time, not its chain's -- so `created + 90 days` is generous for a chain already several rotations
-- old. That is the deliberate direction: the alternative (`expires_at`) would retroactively cut
-- live sessions to whatever remained of their current token. Every backfilled chain is bounded
-- within 90 days of this deploy at the latest, which is the property the playbook states.
-- `profile_id` stays NULL for them and heals on first rotation.
--
-- 90 days matches the AS_REFRESH_CHAIN_MAX_SECONDS default (packages/temper-cloud/src/oauth/
-- endpoints.ts). The literal is duplicated here because a migration cannot read the process env;
-- an operator who lowers the env var lowers it for NEW chains, and the backfilled ones age out.
--
-- THE DEFAULT IS A DEPLOY-ORDERING CONCESSION, and it is worth being exact about what it does and
-- does not buy. Without it this migration would be shape-breaking: the AS's INSERT does not name
-- `chain_expires_at` until the binary that pairs with this lands, so a lagging AS meeting the new
-- schema would fail every token issue -- i.e. break login for the duration of the window. With it,
-- a lagging AS keeps working and writes rows bounded 90 days from their own creation.
--
-- What the default does NOT do is make the bound a property of the CHAIN. That lives entirely in
-- the new binary, which reads the deadline off the token it rotates and hands it to the successor
-- UNCHANGED. NOT NULL still holds the line that matters: no row exists without a bound.
--
-- AND THE CONCESSION IS NOT FULLY CONTAINED, which is worth stating rather than leaving to be
-- discovered. While BOTH binaries are live -- a rolling deploy, a canary, a rollback -- a rotation
-- served by the old one omits the column, takes a fresh 90 days from the DEFAULT, and the new
-- binary then reads that value back and inherits it as authoritative. The deadline moves, and it
-- does not heal: nothing re-derives it from the login it was supposed to be measured from. The
-- owner heals on the next rotation; the deadline does not. Keep the mixed window short.
ALTER TABLE kb_oauth_refresh_tokens
    ADD COLUMN chain_expires_at TIMESTAMPTZ NOT NULL DEFAULT (now() + INTERVAL '90 days'),
    ADD COLUMN profile_id       UUID REFERENCES kb_profiles(id) ON DELETE CASCADE;

-- Rows already on disk take their bound from their own `created`, not from the deploy clock the
-- column default would give them.
--
-- The WHERE is not an optimization, it is the difference between rewriting the rows this backfill
-- is FOR and rewriting the table's whole history. Nothing reaps `kb_oauth_refresh_tokens` — every
-- token ever issued is still here, revoked and expired ones included — and this statement runs
-- inside the migration's transaction, holding the ACCESS EXCLUSIVE the ALTER above took. Every
-- login and every refresh queues behind it. A dead row's chain deadline is read by nothing: the
-- rotation guard already refuses on `revoked_at`/`expires_at` before `chain_expires_at` is
-- consulted, so the DEFAULT is a perfectly good value for them.
--
-- GREATEST, because `created + 90 days` can already be in the past. The "generous" reasoning above
-- silently assumes the 30-day default `AS_REFRESH_TTL_SECONDS`; an operator who raised it beyond 90
-- days has live tokens older than that, and stamping them with a deadline that has already passed
-- would have the rotation guard refuse their very next refresh -- an instant, unexplained logout at
-- deploy, while `expires_at` still says the token is good for weeks. The floor gives every existing
-- session a week to rotate normally onto a properly-stamped successor, and every chain on disk is
-- still bounded well within 90 days of this migration.
UPDATE kb_oauth_refresh_tokens
   SET chain_expires_at = GREATEST(created + INTERVAL '90 days', now() + INTERVAL '7 days')
 WHERE revoked_at IS NULL
   AND expires_at > now();

COMMENT ON COLUMN kb_oauth_refresh_tokens.chain_expires_at IS
  'Absolute end of this refresh CHAIN, stamped at the chain''s first token and inherited unchanged '
  'by every successor. Rotation never moves it -- that is the whole point. Enforced in '
  'rotateRefreshToken''s guard alongside revoked_at/expires_at. The column DEFAULT exists only so a '
  'lagging AS binary can still write a row; it bounds that row but does not make the chain '
  'absolute, which is the writer''s job.';

COMMENT ON COLUMN kb_oauth_refresh_tokens.profile_id IS
  'The principal this chain belongs to, resolved by temper-api through the same '
  '(auth_provider_name, sub) lookup the token itself will later resolve through. NULL when that '
  'resolution was unavailable at issue time (fail-open login); re-resolved on the next rotation.';

-- The terminal-standing revoker's access path: every LIVE token for one principal.
CREATE INDEX idx_kb_oauth_refresh_tokens_live_profile
    ON kb_oauth_refresh_tokens (profile_id)
 WHERE revoked_at IS NULL;

-- The AS learns a profile_id at the ACS, and the token endpoint stamps it on the first token of the
-- chain -- so it has to survive the round trip through the flow row, next to the claims it belongs
-- with. Nullable for the same fail-open reason, and for the pending window before the ACS binds.
ALTER TABLE kb_oauth_flow
    ADD COLUMN profile_id UUID REFERENCES kb_profiles(id) ON DELETE CASCADE;

COMMENT ON COLUMN kb_oauth_flow.profile_id IS
  'Profile resolved at the SAML ACS, carried to the token endpoint so the first token of the chain '
  'is stamped with its owner. NULL while pending, and NULL when resolution failed (fail-open).';

-- ============================================================================
-- principal_may_refresh -- may this principal be handed a NEW token pair?
-- ----------------------------------------------------------------------------
-- A different question from `has_system_access` (20260720000110:63), and the difference is the
-- whole reason this function exists rather than a call to that one.
--
-- `has_system_access` answers "may you use this instance?" -- `state = 'approved'`. Gating rotation
-- on that predicate would strand the population it is designed to serve: `denied` is the
-- BIRTH state of every human who has ever logged in (temper-principal/src/transition.rs:36-38), it
-- is the only state from which `Act::Request` is legal (transition.rs:48), and
-- `/api/access/request` sits on the authenticated-but-ungated router
-- (temper-api/src/routes.rs:20-35). A `denied` or `requested` principal legitimately holds a token;
-- taking their refresh away would make them re-authenticate every access-token TTL in order to ask
-- for the access they are entitled to ask for.
--
-- So this asks the narrower question: has admission ENDED? `Revoke` is legal only from `Approved`
-- (transition.rs:87-94), so `revoked` means precisely "was approved, and is not now"; `Deactivate`
-- is the other administrative end. Those two, and only those two, are the set
-- `standing_service::apply` ends live chains for -- this function is the AS-side reading of the
-- SAME set, which is why it is defined here once instead of restated in TypeScript where nothing
-- could compare it. `standing_service::tests::the_sql_refresh_gate_matches_the_rust_terminal_set`
-- fails if the two ever disagree, in either direction.
--
-- Absence denies, like every other standing predicate: a principal with no standing row is not one
-- we will mint fresh credentials for. Connection profiles deliberately have no row (D7).
CREATE FUNCTION principal_may_refresh(p_profile_id UUID) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM kb_principal_standing s
         WHERE s.profile_id = p_profile_id
           AND s.state NOT IN ('revoked', 'deactivated')
    )
$$;

COMMENT ON FUNCTION principal_may_refresh IS
  'May the AS mint this principal a new token pair? False once admission has ENDED (revoked / '
  'deactivated) and on absence. Deliberately NOT has_system_access: denied and requested '
  'principals hold tokens legitimately, to reach the ungated join-request surface.';

SELECT declare_migration(
    20260825000010,
    'additive',
    'Session-lifecycle hardening for the SAML authorization server: a refresh chain gets a statable maximum lifetime and a recorded owner (task 01a0388c-68ab-7602-accf-ce8e0c38ef31). Two columns on kb_oauth_refresh_tokens -- chain_expires_at (stamped once at the chain''s first token and inherited by every successor, so the deadline is a property of the chain rather than of whichever token is currently held) and profile_id (the principal an administrator revokes through, so a standing terminal reaches the credential itself and not only the API gate its tokens meet) -- one nullable profile_id on kb_oauth_flow to carry the resolved owner from the SAML ACS to the token endpoint, a partial index on live rows per profile for the revoker, and the new predicate principal_may_refresh(uuid). Additive rather than shape-breaking, and the deciding detail is chain_expires_at''s DEFAULT: the column is NOT NULL, and the AS''s INSERT does not name it until the binary that pairs with this lands, so without a default a lagging AS meeting this schema would fail every token issue and break login for the whole window. With the default a lagging AS keeps working, writing rows bounded 90 days from their own creation; the chain-level property lives in the new binary, which reads the deadline off the token it rotates and passes it to the successor unchanged. Everything else here is reachable only by code that knows about it: principal_may_refresh is new, both profile_id columns are nullable, and no existing column, constraint or function signature is altered. The backfill sets chain_expires_at from each row''s own `created` rather than from the deploy clock, so every chain on disk is bounded within 90 days of this migration at the latest; profile_id stays NULL for those rows and is re-resolved on their next rotation. 90 days duplicates the AS_REFRESH_CHAIN_MAX_SECONDS default as a literal because a migration cannot read the process env. Operator-facing bound documented in docs/playbooks/self-host-with-saml.md.'
);
