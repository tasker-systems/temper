-- Principal admission — the persisted half (spec 2026-07-20 §10, D2/D7/D9/D10).
--
-- ONE AUTHORITATIVE STATE IN ONE TABLE (D2). The question "may this principal use this instance?"
-- previously required ANDing conditions across kb_profiles.system_access, kb_profiles.is_active,
-- gating-team membership, and kb_join_requests.status -- written by uncoordinated call sites whose
-- meanings differed by which door the principal entered through. That shape produced two latent
-- bugs in a single morning, neither visible in the diff that would have introduced it. This table
-- gives the question exactly one owner.
--
-- `state` IS text + CHECK, NOT A POSTGRES ENUM, and that is deliberate. `system_access` is an enum
-- and that is exactly what makes it awkward: adding a value needs ALTER TYPE, with transaction and
-- rollback constraints a CHECK does not have. An enum would also buy nothing for the obligation
-- that actually matters -- spec §7 obligation 2 is about a BINARY reading a value added after it
-- shipped, which no write-time constraint can help with. The Rust reader is total by construction
-- (`Standing::parse` returns Option and None refuses); this CHECK guards the write side only.
--
-- NO team_id, ANYWHERE IN THIS FILE (D9). `kb_join_requests` is shaped as though requests were
-- per-team -- team_id is a NOT NULL FK and the uniqueness constraint is
-- (team_id, requesting_profile_id) WHERE status = 'pending' -- but `create_join_request` only ever
-- targets the gating team (access_service.rs:680-688, it resolves gating_team_slug and errors if
-- none). So every row that exists is really "may I use this instance?" wearing a per-team shape.
-- Those are two different questions and this table asks only the first one. Do not read the
-- existing unique index as evidence that standing needs a per-team key.
--
-- CONNECTION PROFILES GET NO ROW AT ALL (D7). Absence denies, so their safety is structural rather
-- than a check someone can forget. The backfill's rule 0 enforces this and there is no
-- discriminator column to key on -- kind is inferable only via NOT EXISTS against kb_connections.

CREATE TABLE kb_principal_standing (
    profile_id  uuid PRIMARY KEY REFERENCES kb_profiles(id) ON DELETE CASCADE,
    state       text NOT NULL CHECK (state IN ('denied','requested','approved','revoked','deactivated')),
    updated     timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE kb_principal_standing IS
  'The one authoritative answer to "may this principal use this instance?" (spec 2026-07-20 D2). '
  'Absence denies. Written ONLY through the transition functions in 20260720000030 -- a direct '
  'UPDATE bypasses the log and the ledger event, which is the exact drift this design removes.';

-- Fast lookup of everyone awaiting a decision, for /admin/access. Partial: the interesting
-- population is tiny and the table is one row per principal.
CREATE INDEX idx_principal_standing_pending
    ON kb_principal_standing (state)
    WHERE state IN ('requested','denied');

-- ---------------------------------------------------------------------------------------------
-- The append-only transition log.
--
-- NEW CONSTRUCTION, not a pattern this repo already has. Every atomic function in migrations/ is
-- TWO-part: mutate the projection row, then PERFORM _event_append. There is no existing separate
-- transition-log table anywhere (the only log-shaped table is kb_resource_audits, which no
-- _event_append function writes). D4 requires both halves here, so this is the first of its kind.
--
-- WHY BOTH, given kb_events already records the act: `Reactivate` must restore the prior state
-- rather than guess it (spec §5), and reading that from kb_events would put the admission machine
-- behind the admin-ledger read gate -- which dispatches per act and is a very different question
-- from "what was this principal's standing before it was deactivated?". One cheap local read
-- beats coupling the gate to the ledger.
-- ---------------------------------------------------------------------------------------------
CREATE TABLE kb_principal_standing_events (
    id                uuid PRIMARY KEY DEFAULT uuid_generate_v7(),
    profile_id        uuid NOT NULL REFERENCES kb_profiles(id) ON DELETE CASCADE,
    act               text NOT NULL CHECK (act IN (
                        'provision','request','withdraw','approve','reject',
                        'revoke','deactivate','reactivate','request_review')),
    -- NULL exactly once per principal: the `provision` that created them, which has no prior.
    prior_state       text CHECK (prior_state IN ('denied','requested','approved','revoked','deactivated')),
    resulting_state   text NOT NULL CHECK (resulting_state IN ('denied','requested','approved','revoked','deactivated')),
    -- NULL for the boot-seed genesis act and for backfilled rows: there is no actor to name.
    actor_profile_id  uuid REFERENCES kb_profiles(id),
    reason            text,
    occurred_at       timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE kb_principal_standing_events IS
  'Append-only. NEVER UPDATE OR DELETE A ROW HERE. `Reactivate` reads the most recent '
  'resulting_state before the deactivation to restore rather than guess (spec §5).';

-- The read `Reactivate` performs: most recent entry for a principal, walking backwards.
CREATE INDEX idx_principal_standing_events_lookup
    ON kb_principal_standing_events (profile_id, occurred_at DESC);

-- ---------------------------------------------------------------------------------------------
-- Governance (D10) -- shipped at the outset rather than deferred to a second spec.
--
-- WHY THIS TABLE EXISTS AT ALL. The original design deferred governance and defined only a seam.
-- The seam did not hold: `is_system_admin` reads a kb_team_members row, so "maintain 'admin
-- implies Approved' by firing a demotion on transition" means adding a TWENTY-FIRST uncoordinated
-- writer to the system's most-written table -- §1's own diagnosis, prescribed as the cure.
--
-- What makes moving it cheap is narrow and was measured: gating-team ownership has exactly ONE
-- authorization reader, the SQL function `is_system_admin`. Every Rust caller (21 production call
-- sites) goes through access_service::is_system_admin, which is a pure passthrough
-- (`SELECT is_system_admin($1)`), and the one in-database caller
-- (20260715000010_context_reassign_fns.sql:76) calls the SQL function directly. So repointing the
-- SQL BODY -- done in 20260720000040 -- moves all 22 call sites at once, and gating-team ownership
-- stops carrying authorization meaning at all. The ~20 writers to kb_team_members become ordinary
-- team-role churn because there is no longer authority stored there to alter.
--
-- ONE ROW PER ADMIN, not a boolean column on kb_principal_standing. Keeping governance in its own
-- table is what keeps "may you act" and "may you govern" two questions (spec §2); a column would
-- re-couple exactly what §2 separates.
-- ---------------------------------------------------------------------------------------------
CREATE TABLE kb_principal_governance (
    profile_id   uuid PRIMARY KEY REFERENCES kb_profiles(id) ON DELETE CASCADE,
    granted_by   uuid REFERENCES kb_profiles(id),   -- NULL for the boot-seed genesis admin
    granted_at   timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE kb_principal_governance IS
  'Who may change the rules (spec 2026-07-20 D10). The presence of a row IS admin-ness. '
  'INVARIANT: admin implies standing = approved -- you cannot govern an instance you may not use. '
  'Enforced by the promote path and by Revoke/Deactivate demoting (20260720000030), never by an '
  'AND at read time.';
