-- When a principal's IdP-derived reach was last brought into agreement, recorded as a fact.
--
-- `reconcile_idp_memberships` (crates/temper-services/src/services/saml_provisioning_service.rs:43)
-- applies an IdP's asserted group set to a principal's `source='idp'` team memberships. It records
-- what the IdP ASSERTS -- `kb_saml_seen_groups`, keyed `(idp_key, group_value)`
-- (20260702000001:26-32) -- and nothing about WHICH PRINCIPAL it ran for or WHEN. Those are
-- different questions at different grains, and this table answers the second.
--
-- THE GRAIN IS `(profile_id, idp_key)`, because that pair is what a reconcile is about: an IdP's
-- mappings are selected by `idp_key` (kb_saml_group_mappings, 20260702000001:10-17) and applied to
-- one profile. Not a column on kb_profiles -- a profile is not per-IdP -- and not on
-- kb_profile_auth_links, whose `auth_provider` is temper-api's single configured provider name and
-- so cannot name which IdP's mappings were applied.
--
-- TWO TIMESTAMPS, ONE MEANING EACH, AND NEITHER DERIVED FROM THE OTHER.
--
--   last_reconciled_at -- the memberships were brought into agreement at this moment. NULL means
--                         no reconcile has been RECORDED, which is not the same claim as "none has
--                         happened": every reconcile before this migration is unrecorded by
--                         construction. The column is deliberately NOT backfilled (see below).
--
--   last_skipped_at    -- group provisioning IS configured for this IdP and the assertion carried
--                         no group signal, so agreement was not attempted. A null is refused as an
--                         input to revocation: it is not "the provider conferred nothing". That
--                         refusal is correct and it is deliberately silent in the membership rows,
--                         which is exactly why it needs a positive record of its own.
--
--                         NARROW ON PURPOSE. `extractGroups` also answers null when an IdP has no
--                         `groups_attr` at all, and that is a different fact wearing the same
--                         shape: an authentication-only IdP derives no team membership ever, so
--                         there is no reconcile to perform and no de-provisioning to suspend.
--                         The ACS asks `groupProvisioningConfigured` first and makes no call in
--                         that case, so no row appears here. Recording both would have made every
--                         principal on an authentication-only deployment read as signal-missing —
--                         a permanent false alarm from a table built to surface real ones, and
--                         indistinguishable at the read from the case that genuinely matters.
--
-- A skip NEVER writes last_reconciled_at. That is the whole point of two columns rather than one
-- `last_attempt_at` plus a flag: with one timestamp, "we agreed" and "we declined to try" would
-- share a representation and could only be told apart by reading the flag correctly every time.
-- Here the read cannot get it wrong, because the fact it wants is the column it names.
--
-- CHECK, because a row exists only because something happened. Both-NULL is not a state this
-- table has; it would be a row asserting nothing.
--
-- ORDERING, IN THE DIRECTION THIS CLASSIFICATION DOES NOT COVER. `additive` means this is safe to
-- apply AHEAD of its binary, and it is: nothing reads or writes either object until that binary
-- lands. The reverse is NOT safe, and the consequence is worth naming rather than leaving to be
-- discovered. `record_reconciled` writes inside the reconcile's transaction, so a binary that
-- reaches this table before the table exists fails that statement, rolls the transaction back, and
-- takes the membership changes with it -- including revocations that would otherwise have applied.
-- Login is unaffected (the caller is fail-open), which is exactly what makes it quiet: group
-- provisioning stops for the whole window and nothing about the login says so. Apply this first.
CREATE TABLE kb_saml_principal_reconcile (
    profile_id         UUID        NOT NULL REFERENCES kb_profiles(id)   ON DELETE CASCADE,
    idp_key            TEXT        NOT NULL REFERENCES kb_saml_idp(idp_key) ON DELETE CASCADE,
    last_reconciled_at TIMESTAMPTZ,
    last_skipped_at    TIMESTAMPTZ,
    PRIMARY KEY (profile_id, idp_key),
    CONSTRAINT kb_saml_principal_reconcile_says_something
        CHECK (last_reconciled_at IS NOT NULL OR last_skipped_at IS NOT NULL)
);

-- Two access paths, both covered. The PK leads with profile_id because that is how the row is
-- written (the reconcile knows both) and how vw_saml_reconcile_staleness joins it. The index
-- carries the other direction -- every principal recorded against one IdP -- which the PK cannot
-- serve on its own, and which the FK's own cascade wants. Matches its sibling
-- idx_kb_saml_group_mappings_idp (20260702000001:18).
CREATE INDEX idx_kb_saml_principal_reconcile_idp ON kb_saml_principal_reconcile(idp_key);

COMMENT ON TABLE kb_saml_principal_reconcile IS
  'When one principal''s source=''idp'' team memberships were last brought into agreement with one '
  'IdP, and when a login last carried no group signal to bring them into agreement with. Grain '
  '(profile_id, idp_key). Distinct from kb_saml_seen_groups, which records which group values an '
  'IdP has asserted at grain (idp_key, group_value) and can say nothing about any principal.';

COMMENT ON COLUMN kb_saml_principal_reconcile.last_reconciled_at IS
  'The moment this principal''s idp memberships were last brought into agreement with this IdP. '
  'Written inside the reconcile''s own transaction, so it is true whenever it is present. NULL '
  'means no reconcile has been RECORDED -- which for a principal whose reach predates this table '
  'includes reconciles that happened before there was anywhere to record them.';

COMMENT ON COLUMN kb_saml_principal_reconcile.last_skipped_at IS
  'The moment an assertion for this principal last carried no group signal DESPITE this IdP having '
  'group provisioning configured, so agreement was not attempted -- the actionable case: it was '
  'expected to arrive and did not. An authentication-only IdP (groups_attr NULL) reaches no reconcile '
  'at all and produces no row here, because it has no de-provisioning to suspend. Never written '
  'together with last_reconciled_at for the same event: a skip must not read as a success.';

-- NOT BACKFILLED, and that is the load-bearing choice in this migration.
--
-- There is nothing on disk to backfill FROM: no login time, no prior reconcile time, nothing that
-- stands in for one. The only available value is `now()`, and stamping it would make every existing
-- principal read as freshly brought into agreement at deploy time -- a claim of agreement nothing
-- performed. A record whose first act is to overstate is worse than no record, because the
-- overstatement is indistinguishable from the truth it displaced.
--
-- So principals whose reach predates this table appear through the view below with a NULL
-- last_reconciled_at and their idp membership count, which is the honest reading: reach is held,
-- and no agreement is recorded for it. They acquire a real timestamp on their next reconcile.

-- ============================================================================
-- vw_saml_reconcile_staleness -- "how stale is each principal's IdP-derived reach"
-- ----------------------------------------------------------------------------
-- One view rather than a query each asker writes, because THE JOIN DIRECTION IS A TRAP and the
-- wrong one looks right. Start from kb_saml_principal_reconcile and INNER JOIN outward and every
-- row you get back has a record -- which silently excludes precisely the principals with none,
-- the population the question is most about. The answer is not empty, not an error, and not
-- obviously short; it is a plausible list missing its worst entries.
--
-- So the view starts from kb_profiles and LEFT JOINs both sides, keeping any profile that appears
-- in EITHER -- holds idp-derived reach, or has a reconcile record, or both. The three cases it
-- must be able to tell apart:
--
--   idp_memberships > 0, last_reconciled_at NULL  -- reach held, no agreement recorded for it
--   idp_memberships > 0, last_reconciled_at old   -- reach held, last agreed at that moment
--   idp_memberships = 0, record present           -- agreed (or declined) down to no reach
--
-- `last_signal_was_missing` is a pure projection of the two timestamps, stated once here so that
-- the comparison is not re-derived (and re-derived differently) at each site that asks. It is
-- true when the most recent thing that happened to this pair was a login carrying no group signal.
-- Note what it is NOT: it says nothing about assertions never presented, because a principal who
-- does not authenticate produces no event of either kind, and this view reports what was recorded.
CREATE VIEW vw_saml_reconcile_staleness AS
WITH idp_reach AS (
    SELECT profile_id, count(*) AS idp_memberships
      FROM kb_team_members
     WHERE source = 'idp'
     GROUP BY profile_id
)
SELECT p.id     AS profile_id,
       p.handle,
       r.idp_key,
       r.last_reconciled_at,
       r.last_skipped_at,
       COALESCE(x.idp_memberships, 0) AS idp_memberships,
       (r.last_skipped_at IS NOT NULL
        AND (r.last_reconciled_at IS NULL OR r.last_skipped_at > r.last_reconciled_at))
              AS last_signal_was_missing
  FROM kb_profiles p
  LEFT JOIN idp_reach x                      ON x.profile_id = p.id
  LEFT JOIN kb_saml_principal_reconcile r    ON r.profile_id = p.id
 WHERE x.profile_id IS NOT NULL
    OR r.profile_id IS NOT NULL;

COMMENT ON VIEW vw_saml_reconcile_staleness IS
  'Per principal: how much source=''idp'' team reach they hold, when it was last brought into '
  'agreement with the IdP, when a login last carried no group signal, and which of those two '
  'happened most recently. Includes principals holding idp reach with NO reconcile record -- the '
  'LEFT JOIN is the point, since an inner join would drop exactly them. Says nothing about '
  'principals who are not authenticating: absence of a recorded event is not an event.';

SELECT declare_migration(
    20260827000030,
    'additive',
    'A per-principal record of when IdP-derived reach was last brought into agreement, so that staleness is a fact the database holds rather than something reconstructed from logs (task 01a03893-e2bf-7973-b885-54978e6088f6, goal 01a035eb-3aea-7ea0-9dd3-f13acdf8cb36). One new table, kb_saml_principal_reconcile, at grain (profile_id, idp_key) -- the pair a reconcile is actually about, since kb_saml_group_mappings selects by idp_key and applies to one profile -- carrying two independent nullable timestamps, one index on idp_key for the other access direction, and one new view vw_saml_reconcile_staleness. Nothing existing is altered: no column, constraint, function signature or view is touched, and a lagging binary cannot reach either name because both are new. THE TWO TIMESTAMPS ARE NOT ONE TIMESTAMP PLUS A FLAG, deliberately. last_reconciled_at means the memberships were brought into agreement; last_skipped_at means group provisioning IS configured for that IdP and the assertion nonetheless carried no group signal, so agreement was not attempted -- the actionable case, since it was expected to arrive and did not. Deliberately narrower than extractGroups'' own null, which also covers an authentication-only IdP with no groups_attr at all: that IdP derives no team membership ever, has no de-provisioning to suspend, and the ACS asks groupProvisioningConfigured first and makes no call, so it produces no row. Recording both would have made every principal on an authentication-only deployment read as signal-missing -- a permanent false alarm from a table built to surface real ones. That null is refused as an input to revocation -- it is not a provider saying nothing-is-conferred -- and the refusal is deliberately invisible in the membership rows, which is why it is given a column of its own rather than being inferred from the absence of a fresher reconcile. A skip never writes last_reconciled_at, so the two can never be read as each other, and a CHECK forbids a row that asserts neither. last_reconciled_at is written INSIDE the reconcile''s own transaction so that its presence and the agreement it claims commit together -- unlike the discovery capture at saml_provisioning_service.rs:49-62, which is autonomous for the opposite reason: discovery data is meaningful without the reconcile, a claim of agreement is not. DELIBERATELY NOT BACKFILLED: nothing on disk records a prior reconcile or login to backfill from, the only available value is now(), and stamping it would assert fresh agreement for every existing principal that nothing performed -- an overstatement indistinguishable from the truth it replaced. Existing principals therefore read as reach-held-with-no-recorded-agreement until their next reconcile, which is the honest state. The view starts from kb_profiles and LEFT JOINs both idp membership counts and the record, keeping profiles present in either; an inner join from the record outward would silently omit every principal that has none, which is the population the view exists to show. Nothing writes either object until the paired binary lands, which is what makes this safe to apply first. The reverse order is not safe and the classification does not claim it is: record_reconciled writes inside the reconcile transaction, so a binary reaching this table before the table exists fails that statement and rolls the transaction back, taking the membership changes -- revocations included -- with it, while login continues unaffected because the caller is fail-open. Group provisioning would stop for the whole window with nothing about the login saying so, which is why the ordering is a requirement rather than a preference.'
);
