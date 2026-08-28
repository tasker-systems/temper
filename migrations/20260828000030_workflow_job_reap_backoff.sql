-- `next_visible_at` has been read by every claim since `20260705000001` and written by nothing.
--
-- THE DEFECT. The column exists, defaults to `now()`, is the third column of
-- `idx_workflow_jobs_claimable`, and is filtered by all four claim variants (`20260705000001`,
-- `20260707000001`, `20260802000020`, `20260724000130`) as `AND c.next_visible_at <= now()`.
-- `workflow_job_reap` never advanced it, so a reaped job was claimable again the instant the reaper
-- committed -- the column, the index and the filter described a backoff the system did not perform.
-- With `embed` (four shards) and `region` both on `* * * * *` and every persona leasing 600 s, three
-- attempts against an unavailable upstream burned in roughly half an hour, and the sweep that
-- enqueued the job deterministically replaced it with the same work.
--
-- THE CURVE. attempts=1 -> 300 s, attempts=2 -> 600 s, cap 3600 s. Half a lease first: the job
-- already waited a full 600 s lease to be noticed, so a materially shorter delay is noise against a
-- one-minute cron. The cap does not bind at `max_attempts = 3` -- it is there so that raising that
-- constant stays bounded, which nobody re-derives when bumping a retry count.
--
-- THE DYING ARM IS UNTOUCHED, deliberately: a terminal row deferred into the future reads as
-- scheduled work that will never run.
--
-- NOT A REMEDY FOR A SUSTAINED OUTAGE. Three attempts are exhausted either way, only more slowly. A
-- persona that cannot run at all should decline at the dispatcher rather than enqueue work nothing
-- can take. This is worth doing regardless: a column every claim reads and nothing writes is a
-- latent defect across all five personas sharing this reaper.

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
