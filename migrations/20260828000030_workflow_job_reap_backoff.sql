-- `next_visible_at` has been read by every claim since `20260705000001` and written by nothing.
--
-- =================================================================================================
-- THE DEFECT
-- =================================================================================================
--
-- `kb_workflow_jobs.next_visible_at` exists (`20260705000001:31`), defaults to `now()`, is the third
-- column of `idx_workflow_jobs_claimable`, and is filtered by EVERY claim variant -- the cogmap one
-- (`20260705000001`), the resource twin (`20260707000001`), the anchor twin (`20260802000020`), and
-- the principal-scoped re-creation (`20260724000130`), each carrying `AND c.next_visible_at <= now()`.
--
-- Nothing ever advanced it. `workflow_job_reap` sets `status`, `last_error`, `lease_expires_at` and
-- `completed_at`, and leaves `next_visible_at` at its insert-time default -- so a reaped job is
-- claimable again the instant the reaper commits. The column, the index and the filter described a
-- backoff the system did not perform.
--
-- WHAT THAT COST, measured rather than supposed. `embed` runs four shards at `* * * * *` and `region`
-- at `* * * * *` (root `vercel.json`), and every persona leases for 600 s. So a job failing against a
-- persistently-unavailable upstream burned all three attempts in roughly thirty minutes of pure lease
-- time, went `dead`, released `uq_workflow_jobs_in_flight`, and was re-enqueued by the next sweep to
-- do it again. Observed on the citation auditor across 2026-07-29..08-28: 143 dead jobs, one every
-- three hours, each carrying the same fifteen citations of the same cogmap.
--
-- =================================================================================================
-- THE CURVE, AND WHY THESE NUMBERS
-- =================================================================================================
--
--   attempts=1 -> 300 s      attempts=2 -> 600 s      cap 3600 s
--
-- FIRST RETRY = HALF A LEASE. Below ~60 s a delay is invisible against a one-minute cron, and the job
-- already waited a full 600 s lease to be noticed at all -- so a delay materially shorter than the
-- lease is noise. Half a lease is a real gap for the minute-cadence personas while still letting a
-- transient upstream blip drain inside the same hour for every persona, including the hourly auditor.
--
-- BOUNDED, and the cap does NOT bind at `max_attempts = 3`. It exists so that raising that constant
-- stays safe: an unbounded doubling on a queue whose jobs are re-created by their own sweep every
-- tick is a leak, not a backoff, and nobody re-derives a bound when bumping a retry count.
--
-- THE DYING ARM IS UNTOUCHED. A job at `max_attempts` goes `dead` with `completed_at` stamped,
-- exactly as before. Deferring a terminal row would leave it looking scheduled forever, and `dead` is
-- not a state anything retries out of.
--
-- =================================================================================================
-- HONEST BOUND ON WHAT THIS BUYS -- read this before citing it as the fix for anything
-- =================================================================================================
--
-- THIS DOES NOT RESCUE A SUSTAINED OUTAGE. Three attempts are exhausted regardless; they are merely
-- exhausted more slowly. The auditor's 143 dead jobs were caused by an external funding ceiling (the
-- AI Gateway returning HTTP 402 for want of budget, 335 occurrences through 2026-08-28), and this
-- migration would have turned 143 dead jobs into perhaps 120. It is not the remedy for that, and the
-- remedy -- an optional agent that skips quietly when it cannot afford to run -- lives in
-- `schedules/auditor.ts`, not here.
--
-- What it does buy is real but modest: a failing job holds `uq_workflow_jobs_in_flight` for longer,
-- so a sweep's re-enqueue coalesces instead of the queue thrashing dead rows; and a genuinely
-- transient upstream gets a gap in which to recover instead of being retried inside the same minute.
-- It is worth doing because it is correct independently of any of that -- a written column that
-- nothing writes is a latent defect across all five personas that share this reaper.
--
-- ADDITIVE. `CREATE OR REPLACE`; signature (`p_error text DEFAULT 'lease expired'`) and return type
-- (`int`) unchanged, so no deployed binary can disagree with this schema across the apply. Body is
-- verbatim from `20260705000001` apart from the one added `SET` term.

CREATE OR REPLACE FUNCTION workflow_job_reap(p_error text DEFAULT 'lease expired') RETURNS int
LANGUAGE sql AS $$
    WITH expired AS (
        SELECT id, attempts, max_attempts
          FROM kb_workflow_jobs
         WHERE status = 'in_progress'
           AND lease_expires_at < now()
         FOR UPDATE SKIP LOCKED
    ), updated AS (
        UPDATE kb_workflow_jobs j
           SET status = CASE WHEN e.attempts >= e.max_attempts THEN 'dead' ELSE 'waiting_for_retry' END,
               last_error = p_error,
               lease_expires_at = NULL,
               completed_at = CASE WHEN e.attempts >= e.max_attempts THEN now() ELSE NULL END,
               -- THE ONLY ADDITION. Retry-only: the dying arm keeps today's value, because a
               -- terminal row deferred into the future reads as scheduled work that will never run.
               next_visible_at = CASE
                   WHEN e.attempts >= e.max_attempts THEN j.next_visible_at
                   ELSE now() + make_interval(
                            secs => least(300 * power(2, e.attempts - 1), 3600))
               END
          FROM expired e
         WHERE j.id = e.id
        RETURNING j.id
    )
    SELECT count(*)::int FROM updated;
$$;

COMMENT ON FUNCTION workflow_job_reap(text) IS
$c$Requeue or kill jobs whose lease expired, with a bounded exponential retry backoff.

Expired `in_progress` jobs go `waiting_for_retry`, or `dead` once `attempts >= max_attempts`
(`attempts` was already incremented at claim). `SKIP LOCKED` lets concurrent reapers coexist.
Persona-agnostic, exactly as the queue is — `p_error` is whichever caller's cron happened to run, so
an auditor job may legitimately carry `embed lease expired`.

`20260828000030` added the backoff: `next_visible_at` advances by
`least(300 * 2^(attempts-1), 3600)` seconds on the retry arm and is left alone on the dying arm. The
column had been read by every claim variant since `20260705000001` and written by nothing, so a
reaped job was claimable again the moment the reaper committed — with `embed` and `region` both on
`* * * * *`, three attempts against a persistently-unavailable upstream burned in about half an hour.

This is NOT a remedy for a sustained outage: three attempts are exhausted either way, only more
slowly. It buys a longer hold on `uq_workflow_jobs_in_flight` (so a sweep's re-enqueue coalesces
rather than thrashing dead rows) and a recovery gap for a genuinely transient failure.$c$;

SELECT declare_migration(
    20260828000030,
    'additive',
    'workflow_job_reap advances next_visible_at on the retry arm, by least(300 * 2^(attempts-1), 3600) seconds. That column has existed since 20260705000001, defaults to now(), is indexed by idx_workflow_jobs_claimable and is filtered by all four claim variants -- and was written by nothing, so a reaped job was claimable again the instant the reaper committed. With embed (four shards) and region both on * * * * * and every persona leasing 600s, a job failing against a persistently-unavailable upstream burned all three attempts in about half an hour, died, released single-flight, and was re-enqueued to repeat. First retry is half a lease because a delay materially shorter than the lease is noise; the cap does not bind at max_attempts=3 and exists so raising that constant stays bounded. The dying arm is untouched -- a terminal row deferred into the future reads as scheduled work that will never run. Deliberately NOT a remedy for a sustained outage: three attempts are exhausted either way, only more slowly. Additive: CREATE OR REPLACE with unchanged signature and return type, body otherwise verbatim from 20260705000001.'
);
