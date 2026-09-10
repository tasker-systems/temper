-- The erasure byte-delete fence (spec 2026-08-31 D3/D5 + the delete-act design's substrate
-- contract, task 01a0577c Beat 4). `blob_delete` releases bytes post-commit ("a provider call
-- cannot join the transaction", 20260906000010) and the release is named in the
-- `principal_erased` payload — so the release-to-provider-delete window is open by construction,
-- healed on re-upload and watched, per the substrate contract: "A byte-deleting build MUST run
-- that fence or its equivalent — retry plus age alerting."
--
-- DERIVE, DON'T REMEMBER: pending deletes seed FROM the `principal_erased` payload's per-target
-- strike verdicts ("erased; released=true; pathname=…", 20260909000025:286-288) — specific
-- pathnames, never provider enumeration (no `list` on BlobStore). First-due is the EVENT's
-- occurred_at, so the age measures how long bytes have been gone-from-the-server while still
-- held at the provider. `erasure_delete_seed` is idempotent per (erasure_event_id, pathname):
-- replaying the derivation re-seeds nothing.
--
-- WHY A DEDICATED TABLE, NOT kb_workflow_jobs: that queue's scope CHECK
-- (`ck_workflow_jobs_one_scope`: exactly one of cogmap/resource/context, each FK-walled) has no
-- slot a delete can occupy, its single-flight grain is (scope, persona, dispatch_type) where the
-- delete's idempotence grain is the content-addressed PATHNAME, and its clock is enqueue time
-- where the fence needs the event's. Using it would amend a five-persona shared table four ways
-- to avoid one small one. The MECHANICS are conformed instead: SKIP LOCKED claim with
-- attempts-increment-at-claim (20260705000001), and the bounded backoff curve
-- least(300 * 2^(attempts-1), 3600) on the retry arm with the dying arm undeferred
-- (20260828000030).
--
-- Additive: a new table and new functions only; nothing existing is altered.

CREATE TABLE kb_erasure_blob_deletes (
    id              uuid PRIMARY KEY DEFAULT uuid_generate_v7(),
    erasure_event_id uuid NOT NULL REFERENCES kb_events(id),
    content_hash    text NOT NULL,
    pathname        text NOT NULL,
    first_due_at    timestamptz NOT NULL,
    status          text NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending','in_progress','waiting_for_retry','done','dead')),
    attempts        int NOT NULL DEFAULT 0,
    max_attempts    int NOT NULL DEFAULT 5,
    resolution      text,
    last_error      text,
    last_error_at   timestamptz,
    next_attempt_at timestamptz NOT NULL DEFAULT now(),
    lease_expires_at timestamptz,
    enqueued_at     timestamptz NOT NULL DEFAULT now(),
    completed_at    timestamptz
);

COMMENT ON TABLE kb_erasure_blob_deletes IS
    'The erasure byte-delete fence: one durable retry state per strike-derived provider delete, '
    'seeded from principal_erased payload verdicts, drained through BlobStore::delete.';

-- One ACTIVE delete per pathname: the bytes are one content-addressed object, so sibling
-- strikes of one hash collapse into the first seed (ON CONFLICT DO NOTHING below). A done or
-- dead row never blocks a later strike of the same pathname — a re-commit + re-strike seeds fresh.
CREATE UNIQUE INDEX uq_erasure_deletes_active_pathname
    ON kb_erasure_blob_deletes (pathname)
    WHERE status IN ('pending', 'in_progress', 'waiting_for_retry');

-- The derivation's idempotence: each released strike seeds AT MOST ONCE ever, so the drain can
-- re-derive from the ledger every tick without re-arming completed work.
CREATE UNIQUE INDEX uq_erasure_deletes_strike
    ON kb_erasure_blob_deletes (erasure_event_id, pathname);

-- Claim scan support, the idx_workflow_jobs_claimable shape.
CREATE INDEX idx_erasure_deletes_claimable
    ON kb_erasure_blob_deletes (next_attempt_at)
    WHERE status IN ('pending', 'waiting_for_retry');

-- Seed one strike-derived delete. Returns the row id, or NULL when this strike was already
-- seeded (the (event, pathname) key) or an active delete for the pathname already stands
-- (a sibling strike of the same hash): one delete per object is the whole grain.
CREATE FUNCTION erasure_delete_seed(
    p_erasure_event uuid, p_hash text, p_pathname text, p_first_due timestamptz
) RETURNS uuid LANGUAGE sql AS $$
    INSERT INTO kb_erasure_blob_deletes (erasure_event_id, content_hash, pathname, first_due_at)
    VALUES (p_erasure_event, p_hash, p_pathname, p_first_due)
    ON CONFLICT DO NOTHING
    RETURNING id;
$$;

-- Claim up to p_limit due deletes: SKIP LOCKED, oldest-first, attempts incremented in SQL,
-- leased for the drain's pass — the workflow_job_claim shape (20260705000001).
CREATE FUNCTION erasure_delete_claim(p_limit int, p_lease_seconds int)
RETURNS TABLE(id uuid, content_hash text, pathname text, attempts int)
LANGUAGE sql AS $$
    UPDATE kb_erasure_blob_deletes j
       SET status = 'in_progress',
           lease_expires_at = now() + make_interval(secs => p_lease_seconds),
           attempts = j.attempts + 1
     WHERE j.id IN (
         SELECT c.id
           FROM kb_erasure_blob_deletes c
          WHERE c.status IN ('pending', 'waiting_for_retry')
            AND c.next_attempt_at <= now()
          ORDER BY c.first_due_at, c.enqueued_at
          LIMIT p_limit
          FOR UPDATE SKIP LOCKED
     )
    RETURNING j.id, j.content_hash, j.pathname, j.attempts;
$$;

-- Record success. p_resolution distinguishes bytes actually struck from the honest skip: a hash
-- a live row re-holds at drain time is NOT deleted (the re-commit re-put those bytes — striking
-- them would delete a live row's object).
CREATE FUNCTION erasure_delete_complete(p_ids uuid[], p_resolution text) RETURNS int
LANGUAGE sql AS $$
    WITH done AS (
        UPDATE kb_erasure_blob_deletes
           SET status = 'done', completed_at = now(), lease_expires_at = NULL,
               resolution = p_resolution
         WHERE id = ANY(p_ids) AND status = 'in_progress'
        RETURNING id
    ) SELECT count(*)::int FROM done;
$$;

-- Record failure: retry with the 20260828000030 curve, dead at max_attempts. The dying arm
-- stays undeferred — a terminal row must not read as scheduled.
CREATE FUNCTION erasure_delete_fail(p_ids uuid[], p_error text) RETURNS int
LANGUAGE sql AS $$
    WITH failed AS (
        UPDATE kb_erasure_blob_deletes j
           SET status = CASE WHEN j.attempts >= j.max_attempts THEN 'dead' ELSE 'waiting_for_retry' END,
               last_error = p_error,
               last_error_at = now(),
               lease_expires_at = NULL,
               completed_at = CASE WHEN j.attempts >= j.max_attempts THEN now() ELSE NULL END,
               next_attempt_at = CASE
                   WHEN j.attempts >= j.max_attempts THEN j.next_attempt_at
                   ELSE now() + make_interval(secs => least(300 * power(2, j.attempts - 1), 3600))
               END
         WHERE id = ANY(p_ids) AND status = 'in_progress'
        RETURNING id
    ) SELECT count(*)::int FROM failed;
$$;

-- Reap leases the drain died mid-pass on: back to the retry ladder (or dead at max). The
-- workflow_job_reap shape (20260705000001 + the 20260828000030 backoff).
CREATE FUNCTION erasure_delete_reap(p_error text DEFAULT 'lease expired') RETURNS int
LANGUAGE sql AS $$
    WITH expired AS (
        SELECT id, attempts, max_attempts
          FROM kb_erasure_blob_deletes
         WHERE status = 'in_progress'
           AND lease_expires_at < now()
         FOR UPDATE SKIP LOCKED
    ), failed AS (
        UPDATE kb_erasure_blob_deletes j
           SET status = CASE WHEN e.attempts >= e.max_attempts THEN 'dead' ELSE 'waiting_for_retry' END,
               last_error = p_error,
               last_error_at = now(),
               lease_expires_at = NULL,
               completed_at = CASE WHEN e.attempts >= e.max_attempts THEN now() ELSE NULL END,
               next_attempt_at = CASE
                   WHEN e.attempts >= e.max_attempts THEN j.next_attempt_at
                   ELSE now() + make_interval(secs => least(300 * power(2, e.attempts - 1), 3600))
               END
          FROM expired e
         WHERE j.id = e.id
        RETURNING j.id
    )
    SELECT count(*)::int FROM failed;
$$;

SELECT declare_migration(
    20260909000040,
    'additive',
    'The erasure byte-delete fence: kb_erasure_blob_deletes plus seed/claim/complete/fail/reap primitives. The substrate contract (2026-09-06 delete-act design) requires a byte-deleting build to run a retry-plus-age-alerting fence over blob_delete''s post-commit release window, and the release verdicts live in the principal_erased payload, so pending deletes are DERIVED from those payloads (specific pathnames, never provider enumeration) with first-due at the event''s occurred_at. A dedicated table rather than kb_workflow_jobs: that queue''s ck_workflow_jobs_one_scope demands exactly one of cogmap/resource/context (all FK-walled), its single-flight grain is (scope, persona, dispatch_type) where the delete''s grain is the content-addressed pathname, and its clock is enqueue time where the fence needs the event''s -- conforming its scope would amend a five-persona shared table four ways. The mechanics are conformed instead: SKIP LOCKED claim with attempts-increment-at-claim (20260705000001) and the bounded backoff least(300 * 2^(attempts-1), 3600) with an undeferred dying arm (20260828000030). Idempotence is two unique indexes: one active delete per pathname (sibling strikes of one hash collapse) and one seed per (erasure_event, pathname) so re-deriving from the ledger never re-arms completed work. Additive: new table, indexes and functions only.'
);
