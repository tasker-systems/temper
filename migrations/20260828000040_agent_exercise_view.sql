-- Whether an agent principal is being exercised, at the grain of the principal itself.
--
-- Argument, alternatives and evidence: temper-artifacts
-- `specs/2026-08-28-agent-exercise-view-design.md`.
--
-- WHY THIS IS A DATABASE OBJECT AND NOT A QUERY SOMEONE REMEMBERS. A 401 at a token endpoint means
-- the credential is wrong and stays loud; a 429 means the issuer will not mint right now for a
-- credential it otherwise accepts, and `optional-agent.ts` turns that into a deliberate quiet skip.
-- The skip is correct and also traceless -- no session, no event, no failed tick, a green cron --
-- so exercise has to be queryable rather than inferred from what went red.
--
-- WHAT A LATER EDIT MUST NOT BREAK:
--
-- * THE LEADING AGGREGATION IS LOAD-BEARING, NOT STYLE. `kb_machine_clients` has no unique on
--   profile_id and `20260711000010` makes the multiplicity a rule ("Reactivation is a new
--   registration, never an UPDATE"). Selecting straight from it mixes a per-credential rung 1 with
--   per-profile rungs 2-4, so a REVOKED credential reports movement dated after its own revoked_at.
-- * SESSIONS COME FROM `kb_invocations` BECAUSE IT IS APPEND-ONLY.
--   `kb_workflow_jobs.claimed_by_profile_id` is current attribution, overwritten when a second
--   principal claims a reaped job: the rung goes from filled back to empty and the view says an
--   agent that ran never ran.
-- * REVOKED CREDENTIALS ARE COUNTED, NEVER FILTERED. A `WHERE` in the CTE drops a fully-revoked
--   principal out of the view entirely -- the population an operator asking "why did this go
--   quiet?" most needs. Same posture `20260827000030` argues for.
-- * NO now()-RELATIVE JUDGMENT, NO LOOKBACK. A threshold here would be a second definition of
--   staleness; the caller compares against the agent's own cadence.
-- * THE RUNGS ARE NOT TEMPORALLY ORDERED. A close is dated after the open it closes, so a later
--   `last_session_closed_at` is the healthy case, not an inversion.

-- Kept for `workflow_job_complete_claimed`, not for this view, which no longer reads the column.
-- The only index here leading with the claimant; partial because anchor- and resource-keyed claims
-- take no principal and leave it NULL.
CREATE INDEX idx_workflow_jobs_claimant
    ON kb_workflow_jobs (claimed_by_profile_id, leased_at DESC)
    WHERE claimed_by_profile_id IS NOT NULL;

-- This view introduces the access path: nothing had asked "which sessions did this principal open",
-- so the session lateral seq-scans without it. Shaped after `idx_kb_events_emitter`, which answers
-- the identical entity-then-recency question for rung 4.
CREATE INDEX idx_kb_invocations_scoped_entity
    ON kb_invocations (scoped_entity_id, opened_at DESC);

CREATE VIEW vw_agent_exercise AS
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
  LEFT JOIN LATERAL (
      SELECT max(i.opened_at) AS last_session_opened_at,
             max(i.closed_at) AS last_session_closed_at,
             (array_agg(i.status ORDER BY i.opened_at DESC))[1] AS last_session_status
        FROM kb_invocations i
        JOIN kb_entities en ON en.id = i.scoped_entity_id
       WHERE en.profile_id = p.profile_id
  ) s ON true
  LEFT JOIN LATERAL (
      SELECT max(ev.occurred_at) AS last_emitted_at
        FROM kb_events ev
        JOIN kb_entities en ON en.id = ev.emitter_entity_id
       WHERE en.profile_id = p.profile_id AND ev.category = 'domain'
  ) e ON true;

COMMENT ON VIEW vw_agent_exercise IS
$c$Whether each registered machine principal is being exercised, in rung order: reached
(last_seen_at) -> ran (last_session_opened_at) -> finished and how (last_session_closed_at,
last_session_status) -> moved (last_emitted_at). The diagnosis is WHERE the signal stops.

One row per PRINCIPAL, not per credential: a profile may hold several kb_machine_clients rows, so
they are aggregated into credentials / credentials_live and every rung is profile-grained. Sessions
come from the append-only kb_invocations, never from kb_workflow_jobs.claimed_by_profile_id, which
is overwritten on reclaim. Rung 4 counts domain events only; an admin or system act is not corpus
movement.

Takes no lookback and states no staleness policy -- the caller thresholds these timestamps against
the agent's own cadence. The rungs are not temporally ordered. Not profile-scoped: an operator
surface, gated by who may query it. Rationale: 20260828000040.$c$;

COMMENT ON COLUMN kb_workflow_jobs.claimed_by_profile_id IS
    'The principal that CURRENTLY holds this job, set at claim alongside correlation_id. Both '
    'credentialed agents record one: the auditor since 20260724000130, the steward since its claim '
    'began passing its own principal. NULL remains for anchor- and resource-keyed claims, whose '
    'personas are server-side workers rather than machine principals. ATTRIBUTION, NOT HISTORY: it '
    'is overwritten in place, so a reap followed by another principal''s claim replaces the previous '
    'claimant with no record it ever held the job -- which is why vw_agent_exercise reads sessions '
    'from kb_invocations instead. Unlike correlation_id it IS consulted: '
    'workflow_job_complete_claimed refuses to complete a job the caller did not claim.';

SELECT declare_migration(
    20260828000040,
    'additive',
    'Adds vw_agent_exercise, two indexes (idx_workflow_jobs_claimant, idx_kb_invocations_scoped_entity) and a refreshed COMMENT on kb_workflow_jobs.claimed_by_profile_id, whose previous text this branch made false by giving the steward''s claim a principal. Additive on every axis that matters to a deploy skew: no existing object''s shape, signature or return type is altered; all three new names are new, so no deployed binary can reach them or disagree with this schema across the apply; a COMMENT is read by no binary; and both indexes are non-unique, so neither can reject a write the running code makes. Safe to apply before or after its branch. Rationale: file header, and temper-artifacts specs/2026-08-28-agent-exercise-view-design.md.'
);
