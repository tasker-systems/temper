-- Whether an agent principal is being exercised, at the grain of the principal itself.
--
-- "Has this agent actually run recently?" was answerable only by hand: the evidence is spread
-- across kb_machine_clients (authenticated at all), kb_invocations (opened a session, and how it
-- ended) and kb_events (what actually changed), and nothing joined them.
--
-- WHY IT IS WORTH A DATABASE OBJECT, AND IT IS NOT ABOUT ANY VENDOR'S BILLING. This system draws a
-- deliberate line between two 4xx answers at a token endpoint, and only one side of that line is
-- loud. A 401 means the credential is WRONG -- the agent believes it is auditing and is not -- and
-- it throws, because a silent skip there would hide a misconfiguration. A 429 means the issuer will
-- not mint RIGHT NOW for a credential it otherwise accepts; `optional-agent.ts`'s
-- `tokenIssuanceUnavailable` pins exactly that status, and `agent/schedules/auditor.ts` turns it
-- into a `console.warn` and a non-throwing return. That quiet skip is CORRECT -- the contract that
-- file states for itself is "a skip, never a fallback", and an agent that cannot run should not
-- start as somebody else. But correct and traceless are the same thing here: no session, no event,
-- no failed tick, and a green cron. THE WELL-BEHAVED FAILURE IS THE ONE THAT LEAVES NOTHING BEHIND
-- TO NOTICE, which is why exercise has to be queryable rather than inferred from what went red.
--
-- EXERCISE IS A LADDER, NOT A PREDICATE, and the columns are in rung order deliberately. The
-- diagnosis is WHERE the signal stops, because each gap names a failure the neighbouring rungs
-- cannot see:
--   1. last_seen_at             -- reached: authenticated against this instance at all. A credential
--                                  refused at its issuer never arrives, so it never advances this.
--   2. last_session_opened_at   -- ran: opened an invocation envelope for a cogmap it can author.
--   3. last_session_closed_at,
--      last_session_status      -- finished, and how -- completed, failed or abandoned; `open` on
--                                  the newest session means it is still in flight, or died mid-loop.
--   4. last_emitted_at          -- moved: something in the corpus actually changed under it.
-- Collapsing these into one boolean would throw the whole diagnosis away, which is why there is no
-- `is_stale` column here and no now()-relative judgment anywhere in this view.
--
-- THE RUNGS ARE NOT TEMPORALLY ORDERED, and reading them as if they were is the trap this ordering
-- invites. A close is dated AFTER the open it closes, so `last_session_closed_at` later than
-- `last_session_opened_at` is the healthy just-finished case, and `last_session_opened_at` later is
-- the healthy mid-run case. Neither is an inversion. "Where the signal stops" means which rungs are
-- STALE against the agent's own cadence -- a comparison the caller makes, never this view.
--
-- THE GRAIN IS THE MACHINE PRINCIPAL, and the `principals` CTE is what makes that true rather than
-- merely asserted. kb_machine_clients has NO unique constraint on profile_id -- only the `id` PK and
-- `client_id` -- and 20260711000010's own column comment guarantees the multiplicity: "A revoked row
-- is dead. Reactivation is a new registration, never an UPDATE. Rows are never deleted."
-- `machine_registration_service::rebind` points a fresh client_id at an EXISTING agent profile and
-- revokes the old row only when asked to, so one principal routinely holds several credential rows.
-- Selecting straight FROM kb_machine_clients mixes two grains: rung 1 is per-credential while rungs
-- 2-4, correlated on profile_id, are per-principal. The consequence is not imprecision, it is a
-- false statement -- a REVOKED credential row reporting sessions and corpus movement dated after its
-- own revoked_at, shown to the one reader ("why did this agent go quiet?") the row exists to inform.
-- Aggregating first makes every rung profile-grained, and one principal exactly one row.
--
-- CREDENTIAL COUNTS REPLACE THE CREDENTIAL COLUMNS. client_id, issuer and revoked_at cannot survive
-- that regrain: a principal holding three credentials has three of each and no single value to
-- report. `credentials_live = 0` is the fact they were carried here to deliver, and the actionable
-- one -- this principal holds nothing that can authenticate, which is why it went quiet -- while
-- `credentials` says whether it ever did. `label` prefers a live credential over a revoked one and
-- the newest within each, so the name shown is the name in use.
--
-- REVOKED CREDENTIALS ARE COUNTED, NEVER FILTERED. A `WHERE revoked_at IS NULL` in the CTE would
-- leave a fully-revoked principal with no group at all and drop it out of the view entirely -- and
-- that is the population an operator asking why an agent stopped most needs to see. Same inclusive
-- posture as vw_saml_reconcile_staleness (20260827000030), whose `declare_migration` reason argues
-- that an inner join "would silently omit every principal that has none, which is the population the
-- view exists to show."
--
-- RUNGS 2-3 READ kb_invocations, NOT THE CLAIM COLUMN, and that is the load-bearing choice in this
-- file. kb_workflow_jobs.claimed_by_profile_id is a SINGLE MUTABLE COLUMN with no history:
-- workflow_job_claim UPDATEs it in place, and workflow_job_reap returns an expired job to
-- `waiting_for_retry` while leaving the claimant where it was. So when a second principal claims a
-- job the first had claimed and then lost to a reap, the first principal's evidence is not
-- superseded, it is OVERWRITTEN -- its rung goes from filled back to empty and the view says it
-- never ran. A rung another principal's activity can erase cannot answer "has this agent run".
-- kb_invocations is append-only: a row is inserted by `_project_delegated_launch` and closed once
-- (`close_invocation` refuses a non-`open` envelope), and nothing there can rewrite another
-- principal's history.
--
-- WHOSE SESSION IT IS. kb_invocations.scoped_entity_id is the OPENER's own entity:
-- `db_backend::open_invocation` resolves it from `self.profile_id` through `writes::resolve_emitter`
-- and passes that one entity as both the scoped entity and the emitter. So kb_entities.profile_id
-- names the principal that ran, and because this view's population is exactly the registered machine
-- principals, the envelopes these laterals reach are exactly agent runs. A profile may own several
-- entities (kb_entities_profile_id_name_key is on (profile_id, name), and `resolve_emitter` names
-- one per surface), so the lateral aggregates across all of them.
--
-- WHAT RUNG 2 COSTS: the envelope is opened BY THE AGENT, not forced by the server. Both agents are
-- instructed to open one at the start of every run (`agent/instructions.md`, the auditor subagent's
-- own instructions, and `invocation_open` in both tool allowlists), but a run that skipped it would
-- read here as never having run. That is the trade taken knowingly against a claim column that
-- another principal can rewrite: an under-report caused by this agent's own omission is legible and
-- local, an over-write caused by a different principal's claim is neither.
--
-- THERE IS NO PERSONA COLUMN. `persona` exists in exactly one place in this schema
-- (kb_workflow_jobs.persona) and is unavailable from the invocation envelope -- `invocation_open`
-- takes no persona at all, which is the same absence 20260724000130 records as the reason its
-- correlation bug stays unfixed. kb_machine_clients.label is TEXT with no COMMENT and no defined
-- semantics, so it is not a persona key either. Reporting the persona of the last CLAIM would have
-- meant reintroducing the mutable column, and its ordering was ambiguous besides: an
-- `array_agg(... ORDER BY leased_at DESC)` answers arbitrarily on a tie.
--
-- RUNG 4 IS DOMAIN EVENTS ONLY. kb_events.category is one of `domain`, `admin`, `system`
-- (kb_event_types carries the same discriminator, and the FK kb_events_category_matches_type pins
-- each event to its type's category). Rung 4 means "something in the corpus changed"; an admin-ledger
-- act or a system event is not corpus movement, and counting it would let a principal that only ever
-- performed administration read as productive. This DEFINES nothing -- it reads the log's own record
-- of what happened. idx_kb_events_emitter (emitter_entity_id, occurred_at DESC) already exists for
-- exactly this access path.
--
-- ORDERING WITHOUT `NULLS LAST`, deliberately, unlike the version this replaces. kb_invocations
-- .opened_at is NOT NULL, so `ORDER BY i.opened_at DESC` cannot put a null ahead of a real session
-- the way a DESC ordering over the nullable `leased_at` could.
--
-- NO LOOKBACK PARAMETER, and no threshold. Raw timestamps out; the caller decides what "recently"
-- means -- the steward's cadence and the auditor's are not the same number. Staleness already has
-- named predicates elsewhere and this must not become a second definition of one, the same drift
-- 20260727000050 exists to prevent. CONFORMs to vw_saml_reconcile_staleness, which makes the same
-- choice for the same reason.
--
-- A PLAIN VIEW, NOT A MEMO. kb_resource_standing is a TABLE memo and carries a standing refresh
-- obligation on any direct consumer (DbBackend::tick_resource_standing). A view carries none, which
-- is the whole reason it is the right object here: the task that produced this forbids anything that
-- something must remember to refresh.
--
-- NOT PROFILE-SCOPED, DELIBERATELY. A view takes no arguments and every profile-aware predicate in
-- this schema is a function taking a principal, so "a view" and "profile-scoped" cannot both hold.
-- Agent exercise is an OPERATOR question about the deployment, not a tenant question about their own
-- data -- the reach it needs is over the deployment's own agents, not over anybody's resources.
-- vw_saml_reconcile_staleness resolves the identical tension the identical way (it starts FROM
-- kb_profiles and exposes every handle). Gate this by who may query it, never by widening it.
--
-- WHO IS OUT OF SCOPE, AND WHY IT IS BY CONSTRUCTION. Embed, Region and Shape hold no
-- kb_machine_clients row -- workflow_job.rs calls them non-agent server-side workers, and their
-- claims go through workflow_job_claim_anchor / workflow_job_claim_resource, neither of which takes
-- a principal -- so they can never appear in `principals` and cannot be omitted by accident. (They
-- are not credential-less in every sense -- Embed's dispatch endpoint sits behind a shared
-- bearer-secret cron gate. They hold no machine PRINCIPAL, which is what this view is grained on.)
-- Their question is queue health, which is a different object.
--
-- ADDITIVE: one new index, one new view, and one COMMENT refreshed on an existing column. No
-- existing object's shape is altered, both new names are new, and no deployed binary can reach them
-- or disagree with this schema across the apply.

-- KEPT, AND NO LONGER FOR THIS VIEW. The laterals below read kb_invocations and kb_events; neither
-- touches kb_workflow_jobs, so nothing in vw_agent_exercise uses this index. It stays for the one
-- predicate that does read the column: `workflow_job_complete_claimed` (20260724000130), which
-- transitions a row only when `claimed_by_profile_id = p_principal`. Beat A made that predicate
-- meaningful for the steward as well as the auditor -- before it, every steward-claimed row carried a
-- NULL claimant, which is outside this index's own partial predicate and outside any equality on it.
-- Honest about what it does NOT buy: `workflow_job_complete_claimed` already locates its row through
-- uq_workflow_jobs_in_flight (cogmap_id, persona, dispatch_type) and applies the claimant as a
-- filter, so this index is not what makes that statement fast. What it is, is the only index in this
-- schema that leads with the claimant -- every other one on kb_workflow_jobs leads with persona,
-- cogmap, resource or context -- and so the only one that can serve the claimant DIRECTION at all:
-- "which jobs did this principal claim", the query the recorded-not-fixed remedy at
-- 20260724000130's `ORDER BY coalesce(j.claimed_by_profile_id = v_profile, false) DESC, j.leased_at
-- DESC` would need. Partial because the column is NULL for every anchor- and resource-keyed claim,
-- and those rows can never satisfy an equality on it.
CREATE INDEX idx_workflow_jobs_claimant
    ON kb_workflow_jobs (claimed_by_profile_id, leased_at DESC)
    WHERE claimed_by_profile_id IS NOT NULL;

-- Rung 2/3 support, and the reason it is here rather than left to a later migration: this view
-- INTRODUCES the access path. kb_invocations carries indexes on originating_cogmap_id, status and
-- correlation_id, and none on scoped_entity_id -- nothing had asked "which sessions did this
-- principal open" before, so the session lateral below plans as a Seq Scan on kb_invocations once
-- per principal (verified with EXPLAIN against this view). Shaped after idx_kb_events_emitter
-- (emitter_entity_id, occurred_at DESC), which serves the identical entity-then-recency question for
-- rung 4; the DESC matches the max()/newest-first reads the lateral makes. Not partial:
-- scoped_entity_id is NOT NULL, so there is no dead subset to exclude.
CREATE INDEX idx_kb_invocations_scoped_entity
    ON kb_invocations (scoped_entity_id, opened_at DESC);

CREATE VIEW vw_agent_exercise AS
-- One row per machine PRINCIPAL, not per credential row. See the header: rungs 2-4 are correlated on
-- profile_id, so anything but a profile-grained left-hand side reports one credential's ladder on
-- another credential's row.
WITH principals AS (
    SELECT profile_id,
           max(last_seen_at)                                                      AS last_seen_at,
           count(*)                                                               AS credentials,
           count(*) FILTER (WHERE revoked_at IS NULL)                             AS credentials_live,
           (array_agg(label ORDER BY (revoked_at IS NULL) DESC, created DESC))[1] AS label
      FROM kb_machine_clients
     GROUP BY profile_id
)
SELECT p.profile_id, p.label, p.credentials, p.credentials_live, p.last_seen_at,
       s.last_session_opened_at, s.last_session_closed_at, s.last_session_status,
       e.last_emitted_at
  FROM principals p
  -- Rungs 2 and 3. An aggregate over zero rows still yields one row, so a principal that has never
  -- opened a session survives with NULLs rather than vanishing.
  LEFT JOIN LATERAL (
      SELECT max(i.opened_at) AS last_session_opened_at,
             max(i.closed_at) AS last_session_closed_at,
             (array_agg(i.status ORDER BY i.opened_at DESC))[1] AS last_session_status
        FROM kb_invocations i
        JOIN kb_entities en ON en.id = i.scoped_entity_id
       WHERE en.profile_id = p.profile_id
  ) s ON true
  -- Rung 4, aggregated across every entity the profile owns.
  LEFT JOIN LATERAL (
      SELECT max(ev.occurred_at) AS last_emitted_at
        FROM kb_events ev
        JOIN kb_entities en ON en.id = ev.emitter_entity_id
       WHERE en.profile_id = p.profile_id AND ev.category = 'domain'
  ) e ON true;

COMMENT ON VIEW vw_agent_exercise IS
$c$Whether each registered machine principal is being exercised, in rung order: reached
(last_seen_at) -> ran (last_session_opened_at) -> finished and how (last_session_closed_at,
last_session_status) -> moved (last_emitted_at). ONE ROW PER PRINCIPAL: a profile may hold several
kb_machine_clients rows (reactivation is a new registration, never an UPDATE), so credentials are
aggregated into credentials / credentials_live and the ladder is profile-grained throughout.
Sessions come from the append-only kb_invocations, NOT from kb_workflow_jobs.claimed_by_profile_id,
which is current attribution and is overwritten when another principal reclaims a reaped job. The
diagnosis is WHERE the signal stops, and the rungs are not temporally ordered -- a close is dated
after its own open. States no staleness policy and takes no lookback: the caller thresholds these
timestamps itself, so this cannot become a second definition of staleness. Not profile-scoped: an
operator surface, gated by who may query it. Rationale: 20260828000040.$c$;

-- REFRESHED, because Beat A inverted exactly what the incumbent text says. It read "(NULL when the
-- claimer passed no principal -- the steward's unscoped claim)"; the steward's claim now passes its
-- principal, so both credentialed agents record a claimant and that parenthetical is false. The
-- replacement also states the property that kept this column OUT of vw_agent_exercise, so the next
-- reader tempted to build exercise on it finds the reason at the column rather than in a view header.
COMMENT ON COLUMN kb_workflow_jobs.claimed_by_profile_id IS
    'The principal that CURRENTLY holds this job, set at claim alongside correlation_id. Both '
    'credentialed agents record one: the auditor since 20260724000130, the steward since the claim '
    'began passing its own principal. NULL remains for anchor- and resource-keyed claims '
    '(workflow_job_claim_anchor, workflow_job_claim_resource), which take no principal because their '
    'personas are server-side workers rather than machine principals. THIS IS ATTRIBUTION, NOT '
    'HISTORY: it is overwritten in place, so a reap followed by another principal''s claim replaces '
    'the previous claimant with no record that it ever held the job -- which is why vw_agent_exercise '
    'reads sessions from the append-only kb_invocations instead of this column. Unlike '
    'correlation_id, it IS consulted: workflow_job_complete_claimed refuses to complete a job the '
    'caller did not claim.';

SELECT declare_migration(
    20260828000040,
    'additive',
    'One new view, vw_agent_exercise, one new partial index on kb_workflow_jobs(claimed_by_profile_id, leased_at DESC), and one refreshed COMMENT on kb_workflow_jobs.claimed_by_profile_id. Answers "has this agent actually run recently?" -- previously a hand-written join per asking across kb_machine_clients, kb_invocations and kb_events. It is worth a database object because of how this system treats two different 4xx answers at a token endpoint: a 401 means the credential is wrong and stays loud, while a 429 means the issuer will not mint right now for a credential it otherwise accepts and is turned into a deliberate quiet skip (optional-agent.ts tokenIssuanceUnavailable, agent/schedules/auditor.ts). That skip is correct -- an agent that cannot run must not start as somebody else -- and it is also traceless: no session, no event, no failed tick, a green cron. The well-behaved failure is the one that leaves nothing behind to notice, so exercise has to be queryable rather than inferred from what went red. GRAIN IS THE MACHINE PRINCIPAL, enforced by a leading aggregation rather than assumed: kb_machine_clients has no unique constraint on profile_id, 20260711000010 states that reactivation is a new registration and never an UPDATE, and machine_registration_service::rebind points a fresh client_id at an existing agent profile, so one principal routinely holds several credential rows. Selecting straight from that table would mix a per-credential rung 1 with per-profile rungs 2-4 and produce a false statement -- a revoked credential row reporting sessions and corpus movement dated after its own revoked_at, shown to exactly the operator asking why the agent went quiet. credentials and credentials_live replace client_id, issuer and revoked_at, which have no single value at principal grain; credentials_live = 0 is the actionable fact, and label prefers a live credential over a revoked one. Revoked credentials are counted rather than filtered, because filtering them would drop a fully-revoked principal out of the view entirely -- the same inclusive posture vw_saml_reconcile_staleness argues for. RUNGS 2-3 READ THE APPEND-ONLY kb_invocations, NOT kb_workflow_jobs.claimed_by_profile_id: that column is a single mutable attribution with no history, workflow_job_reap leaves it in place when a lease expires, and a second principal claiming the reaped job overwrites the first principal''s evidence rather than superseding it -- the rung goes from filled back to empty and the view says an agent that ran never ran. kb_invocations.scoped_entity_id is the opener''s own entity (db_backend::open_invocation resolves it from self.profile_id), so kb_entities.profile_id names the principal that ran; a profile may own several entities, so the lateral aggregates across them. The envelope is opened by the agent rather than forced by the server, which is a knowing trade: an under-report caused by this agent omitting its own envelope is local and legible, an over-write caused by a different principal''s claim is neither. There is no persona column: persona exists only on kb_workflow_jobs, invocation_open takes none, kb_machine_clients.label has no defined semantics, and reporting the persona of the last claim would have meant reintroducing the mutable column. Rung 4 filters category = domain -- the live categories are domain, admin and system, and an administrative or system act is not corpus movement. Four rungs as separate raw timestamps with NO now()-relative judgment and no lookback parameter: the diagnosis is where the signal stops, collapsing the rungs into one boolean would discard it, and a threshold here would be a second definition of staleness. The rungs are not temporally monotonic and the view does not pretend they are -- a close is dated after the open it closes. Conforms to vw_saml_reconcile_staleness on all three of its shapes: a plain view (no refresh obligation, unlike the kb_resource_standing table memo), no lookback, and deliberately not profile-scoped -- a view takes no arguments while every profile-aware predicate is a function taking a principal, and agent exercise is an operator question about the deployment rather than a tenant question about their own data. Embed, Region and Shape are out of scope by construction rather than by omission: they hold no kb_machine_clients row and claim through workflow_job_claim_anchor / workflow_job_claim_resource, neither of which takes a principal. The index no longer serves this view at all; it is kept for workflow_job_complete_claimed, which transitions a row only when claimed_by_profile_id = p_principal and which the steward''s claim now reaches too, and because it is the only index in this schema leading with the claimant. The refreshed column COMMENT replaces a parenthetical that this branch made false -- the steward''s claim no longer passes no principal -- and states at the column itself that the value is current attribution rather than run history. Additive: no existing object''s shape is altered, both new names are new, a COMMENT is not read by any binary, and nothing deployed can reach either object across the apply, so this is safe to apply in either order relative to its branch.'
);
