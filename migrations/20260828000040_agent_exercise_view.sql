-- Whether an agent principal is being exercised, at the grain a credential is keyed on.
--
-- "Has this agent actually run recently?" was answerable only by hand: the evidence is spread across
-- kb_workflow_jobs (claims, leases, deaths), kb_machine_clients (authentication) and kb_events
-- (what actually changed), and nothing joined them. PR #809 made that worse in a way that was
-- correct on its own terms -- a quota-exhausted auditor now logs a warning and returns green, so a
-- FAILING TICK is no longer the accidental exercise signal it used to be. The auditor holds its own
-- IdP application and therefore its own client-level issuance quota, so it can go quiet for a quota
-- period while the steward keeps running normally, behind three green crons.
--
-- EXERCISE IS A LADDER, NOT A PREDICATE, and the columns are in rung order deliberately. The
-- diagnosis is WHERE the signal stops, because each gap names a failure the neighbouring rungs
-- cannot see:
--   1. last_seen_at      -- reached: authenticated at all. A quota-exhausted agent is refused at the
--                           IdP's token endpoint and never arrives, so this alone catches #809's case.
--   2. last_claim_at     -- claimed: took work off the queue. A gap here is USUALLY BENIGN -- see
--                           docs/playbooks/deploy-a-steward-agent.md, "claimed 0 job(s) ... is the
--                           steady state, not a fault". This view does not judge it.
--   3. last_completion_at / last_death_at -- ran to a terminal state, and which one.
--   4. last_emitted_at   -- moved: something in the corpus actually changed under this credential.
-- Collapsing these into one boolean would throw the whole diagnosis away, which is why there is no
-- `is_stale` column here and no now()-relative judgment anywhere in this view.
--
-- NO LOOKBACK PARAMETER, and no threshold. Raw timestamps out; the caller decides what "recently"
-- means. Staleness already has named predicates elsewhere and this must not become a second
-- definition of one -- the same drift 20260727000050 exists to prevent. CONFORMs to
-- vw_saml_reconcile_staleness (20260827000030), which makes the same choice for the same reason.
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
-- THE GRAIN IS THE MACHINE PRINCIPAL. Not the persona: `persona` exists in exactly one place in this
-- schema (kb_workflow_jobs.persona) and nothing binds it to a principal -- kb_machine_clients.label
-- is free text with no defined semantics. So last_persona_claimed is OBSERVED (which persona's work
-- this credential last took) and never declared. And not the cogmap: a credential, an issuance quota
-- and a kb_events emitter are all keyed on a principal. Embed, Region and Shape hold no credential at
-- all -- workflow_job.rs calls them non-agent server-side workers -- so they are out of scope here by
-- construction, not by omission. Their question is queue health, which is a different object.
--
-- REVOKED CLIENTS ARE KEPT, with revoked_at exposed: an operator asking why an agent went quiet is
-- exactly the reader who needs to see that it was revoked. Same inclusive posture as
-- 20260827000030, whose own header argues that an inner join "would silently omit every principal
-- that has none, which is the population the view exists to show."
--
-- ADDITIVE: one new index and one new view. No existing object is altered and both names are new, so
-- no deployed binary can reach them or disagree with this schema across the apply.

-- Rung 2/3 support. kb_workflow_jobs had no index on claimed_by_profile_id, so the laterals below
-- would seq-scan the queue once per principal. Partial because the column is NULL for every job
-- claimed by an unscoped caller, and those rows can never satisfy the laterals' equality predicate.
CREATE INDEX idx_workflow_jobs_claimant
    ON kb_workflow_jobs (claimed_by_profile_id, leased_at DESC)
    WHERE claimed_by_profile_id IS NOT NULL;

CREATE VIEW vw_agent_exercise AS
SELECT
    mc.profile_id,
    mc.client_id,
    mc.label,
    mc.issuer,
    mc.revoked_at,
    mc.last_seen_at,
    q.last_claim_at,
    q.last_persona_claimed,
    q.last_completion_at,
    d.last_death_at,
    d.last_error,
    e.last_emitted_at
  FROM kb_machine_clients mc
  -- Rungs 2 and 3. An aggregate over zero rows still yields one row, so a principal that has never
  -- claimed anything survives with NULLs rather than vanishing.
  LEFT JOIN LATERAL (
      SELECT max(j.leased_at)                                               AS last_claim_at,
             max(j.completed_at) FILTER (WHERE j.status = 'done')           AS last_completion_at,
             -- NULLS LAST is load-bearing: DESC defaults to NULLS FIRST, which would let a job with
             -- no leased_at outrank every real claim and report the wrong persona.
             (array_agg(j.persona ORDER BY j.leased_at DESC NULLS LAST))[1] AS last_persona_claimed
        FROM kb_workflow_jobs j
       WHERE j.claimed_by_profile_id = mc.profile_id
  ) q ON true
  -- The dying arm, kept separate because it is a different fact: not "did it finish" but "how".
  -- LIMIT 1 returns ZERO rows for a healthy principal, so this join MUST be LEFT.
  LEFT JOIN LATERAL (
      SELECT j.completed_at AS last_death_at, j.last_error
        FROM kb_workflow_jobs j
       WHERE j.claimed_by_profile_id = mc.profile_id
         AND j.status = 'dead'
       ORDER BY j.completed_at DESC NULLS LAST
       LIMIT 1
  ) d ON true
  -- Rung 4, and the reason this view needs no per-persona branch: kb_events is where every agent's
  -- work lands regardless of persona, so this DEFINES nothing -- it reads the log's own record of
  -- what happened. idx_kb_events_emitter (emitter_entity_id, occurred_at DESC) already exists for
  -- exactly this. A profile may own several entities (kb_entities_profile_id_name_key is on
  -- (profile_id, name)), so this aggregates across all of them.
  LEFT JOIN LATERAL (
      SELECT max(ev.occurred_at) AS last_emitted_at
        FROM kb_events ev
        JOIN kb_entities en ON en.id = ev.emitter_entity_id
       WHERE en.profile_id = mc.profile_id
  ) e ON true;

COMMENT ON VIEW vw_agent_exercise IS
$c$Whether each registered machine principal is being exercised, in rung order: reached
(last_seen_at) -> claimed (last_claim_at) -> completed or died -> moved (last_emitted_at). The
diagnosis is WHERE the signal stops; a gap at "claimed" is usually benign (an empty queue), a gap at
"reached" is not. States no staleness policy and takes no lookback -- the caller thresholds these
timestamps itself, so this cannot become a second definition of staleness. Not profile-scoped: an
operator surface, gated by who may query it. Rationale: 20260828000040.$c$;

SELECT declare_migration(
    20260828000040,
    'additive',
    'One new view, vw_agent_exercise, and one new partial index on kb_workflow_jobs(claimed_by_profile_id, leased_at DESC). Answers "has this agent actually run recently?" -- previously a hand-written join per asking, and newly worth answering because PR #809 correctly made a quota-exhausted auditor skip quietly rather than fail its tick, removing the accidental exercise signal a failing tick used to provide. The auditor holds its own IdP application and therefore its own client-level issuance quota, so it can go quiet while the steward runs normally behind green crons. Grain is the machine principal, because a credential, a quota and a kb_events emitter are all keyed on one; persona exists only on kb_workflow_jobs and nothing binds it to a principal, so last_persona_claimed is observed rather than declared. Embed, Region and Shape hold no credential and are out of scope by construction. Four rungs as separate raw timestamps with NO now()-relative judgment and no lookback parameter: the diagnosis is where the signal stops, and collapsing the rungs into one boolean would discard it, while a threshold here would be a second definition of staleness. Conforms to vw_saml_reconcile_staleness on all three of its shapes: a plain view (no refresh obligation, unlike the kb_resource_standing table memo), no lookback, and deliberately not profile-scoped -- a view takes no arguments while every profile-aware predicate is a function taking a principal, and agent exercise is an operator question about the deployment rather than a tenant question about their own data. Revoked clients are kept with revoked_at exposed because an operator asking why an agent went quiet needs to see it. Additive: both names are new, nothing existing is altered, and no deployed binary can reach either across the apply, so this is safe to apply in either order relative to its branch.'
);
