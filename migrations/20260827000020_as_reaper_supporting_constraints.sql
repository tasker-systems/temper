-- The retention sweep can run without scanning, and recorded replay evidence cannot be cascaded away.
--
-- Supporting schema for the AS retention sweep (TMPR-56,
-- crates/temper-services/src/services/as_reap_service.rs). The sweep is the FIRST thing in the
-- system that has ever deleted from these tables, and deleting from a table exercises constraints
-- that only a delete can reach. Three of them were load-bearing and unbuilt.

-- ============================================================================
-- 1. The self-FK's missing index — the reason a capped sweep was not a bounded sweep.
-- ----------------------------------------------------------------------------
-- `rotated_to` (20260701000006:75) references this same table with no index behind it, so every
-- DELETE fired the RI check `SELECT 1 FROM kb_oauth_refresh_tokens WHERE $1 = rotated_to` once per
-- deleted row -- and with no index that is a full sequential scan of the parent table PER ROW.
--
-- Measured on a 200k-row clone, deleting one 5,000-row batch:
--
--     Trigger for constraint kb_oauth_refresh_tokens_rotated_to_fkey: time=21842.835 calls=5000
--     Trigger for constraint kb_oauth_refresh_replays_token_id_fkey:  time=15.990   calls=5000
--     Execution Time: 21879.881 ms
--
-- 21.8 of the 21.9 seconds is that one trigger. The replays cascade, whose referencing column is
-- its own table's PRIMARY KEY and therefore indexed, costs 16 ms for the identical 5,000 calls --
-- the two side by side are what identify the index and not the delete as the cause. With this
-- index the same statement runs its trigger in 17 ms.
--
-- The cost is O(rows_deleted x table_size), which is why the sweep's per-run row cap did not bound
-- it: capping rows does not cap work when each row costs a table scan. Left unbuilt, a full capped
-- run against a table grown since 20260701000006 exceeds `api/internal`'s 300s maxDuration
-- (vercel.json) and is killed mid-sweep every night.
--
-- PARTIAL, and that is the whole reason it is affordable. `rotated_to` is unwritten by design
-- (20260826000140 says so: `rotated_at` carries what it was declared for), so every row's value is
-- NULL and the partial index is EMPTY -- 8 kB against 1368 kB for the unqualified index, measured.
-- It costs nothing while the column stays unwritten and starts working the moment it does not.
-- `$1 = rotated_to` is strict, so no NULL row can satisfy the RI check and the planner may use it;
-- verified by EXPLAIN ANALYZE rather than assumed.
CREATE INDEX idx_kb_oauth_refresh_tokens_rotated_to
    ON kb_oauth_refresh_tokens (rotated_to)
 WHERE rotated_to IS NOT NULL;

-- ============================================================================
-- 2. That same self-FK is NO ACTION, which would wedge the sweep permanently.
-- ----------------------------------------------------------------------------
-- The sweep deletes in unordered batches and holds evidence-bearing rows out FOREVER (see 3). So
-- if `A.rotated_to = B` and B is reapable while A is pinned -- or merely lands in a different
-- batch -- the DELETE raises 23503 and the whole run fails. It would fail on every subsequent run
-- too, because the pinned row never leaves. Reproduced against a clone.
--
-- Latent today and not tomorrow: nothing writes the column, so the trap is armed only for whoever
-- implements the chain-topology capability 20260826000140 says the column is "kept" for. Rather
-- than leave the sweep depending on a property no constraint enforces, SET NULL makes the descent
-- link give way to the retention floor. That is the right precedence: the link is a convenience
-- for reading chain shape, and the row it points at is one the sweep has already established is
-- past every floor that could make it interesting.
ALTER TABLE kb_oauth_refresh_tokens
    DROP CONSTRAINT kb_oauth_refresh_tokens_rotated_to_fkey,
    ADD  CONSTRAINT kb_oauth_refresh_tokens_rotated_to_fkey
         FOREIGN KEY (rotated_to) REFERENCES kb_oauth_refresh_tokens(id)
         ON DELETE SET NULL;

-- ============================================================================
-- 3. Replay evidence stops being something a retention job can destroy.
-- ----------------------------------------------------------------------------
-- `kb_oauth_refresh_replays.token_id` was ON DELETE CASCADE (20260826000140:75), so deleting a
-- token deleted any replay recorded against it -- the row an operator reads through
-- `vw_oauth_refresh_replays` when asking whether a credential was copied.
--
-- The sweep already refuses to select such a row (`NOT EXISTS`, as_reap_service.rs), and that is
-- NOT sufficient, which is the finding this constraint exists for. Under READ COMMITTED the
-- subquery is evaluated against the statement's snapshot; if the AS records a replay while the
-- sweep is blocked on that row's FK KEY SHARE lock, the reaper resumes against its ORIGINAL
-- snapshot, deletes the token, and the cascade destroys evidence committed a moment earlier.
-- Reproduced: the delete succeeded and both the token and the just-committed replay row were gone.
--
-- RESTRICT moves the guarantee from a predicate that races to a constraint that cannot. The
-- `NOT EXISTS` arm stays as the normal path -- it is what keeps the sweep from ever reaching this
-- constraint -- so RESTRICT fires only in the race, fails that one batch, and self-heals: the next
-- run's `NOT EXISTS` sees the committed evidence and skips the row for good.
--
-- Nothing else deletes from `kb_oauth_refresh_tokens`, so this narrows no existing path. The two
-- administrative revokers and the AS's own rotation all UPDATE `revoked_at`; none deletes a row.
--
-- ONE COUPLING TO KNOW ABOUT BEFORE BUILDING AN ERASURE PATH. `kb_profiles` cascades into
-- `kb_oauth_refresh_tokens` via its `profile_id`, so deleting a profile deletes its tokens -- and
-- would now meet this RESTRICT if one of them carried evidence that did NOT also cascade. It does
-- not today, and the reason is worth writing down rather than rediscovering: `recordRefreshReplay`
-- takes the replay's `profile_id` from the token row it is recording against, so the two always
-- agree. An owned token's evidence is owned too and cascades beside it; an unowned token's
-- evidence is unowned, and an unowned token is not reachable from a profile delete at all. The
-- pair is therefore always deleted together or not at all.
--
-- That invariant is held by one line of TypeScript and by no constraint. A future erasure path
-- that writes `kb_oauth_refresh_replays` directly, or that backfills `profile_id` onto tokens
-- without their replays, breaks it and will meet a 23001 on the profile delete. The fix then is
-- for erasure to delete the evidence explicitly, which is the honest shape anyway: forgetting a
-- person is a deliberate act and should say so, not arrive as a side effect of a cascade.
ALTER TABLE kb_oauth_refresh_replays
    DROP CONSTRAINT kb_oauth_refresh_replays_token_id_fkey,
    ADD  CONSTRAINT kb_oauth_refresh_replays_token_id_fkey
         FOREIGN KEY (token_id) REFERENCES kb_oauth_refresh_tokens(id)
         ON DELETE RESTRICT;

COMMENT ON CONSTRAINT kb_oauth_refresh_replays_token_id_fkey ON kb_oauth_refresh_replays IS
  'RESTRICT, not CASCADE: a recorded replay is evidence that a credential was copied, and no '
  'retention job may destroy it. The AS retention sweep also filters these rows out by predicate; '
  'that filter races under READ COMMITTED and this constraint does not.';

-- ============================================================================
-- 4. The two tables the sweep filters that had no index for it.
-- ----------------------------------------------------------------------------
-- `kb_saml_replay` has shipped `idx_kb_saml_replay_expires` since 20260701000006 with no consumer;
-- the other two never got the equivalent. It does not bite while a backlog exists (the sweep's
-- LIMIT stops early on a table where most rows qualify) -- it bites in STEADY STATE, where nothing
-- qualifies, the LIMIT is never satisfied, and each nightly run makes a full pass over both tables
-- to delete nothing. `EXPLAIN` with `enable_seqscan = off` confirms no index alternative existed.
CREATE INDEX idx_kb_oauth_flow_expires ON kb_oauth_flow (expires_at);

-- Composite because the sweep requires BOTH deadlines past their margin. Leading column is
-- `expires_at` to match the other two tables' access shape.
CREATE INDEX idx_kb_oauth_refresh_tokens_expires
    ON kb_oauth_refresh_tokens (expires_at, chain_expires_at);

SELECT declare_migration(
    20260827000020,
    'additive',
    'Supporting schema for the AS retention sweep (TMPR-56): three indexes and two foreign-key redefinitions on the Authorization Server tables, all reachable only by a DELETE, which nothing performed against these tables until the sweep existed. (1) A PARTIAL index on kb_oauth_refresh_tokens(rotated_to) WHERE rotated_to IS NOT NULL. The self-FK introduced in 20260701000006 has no index behind it, so each deleted row fired an RI check that sequentially scanned the whole parent table: measured at 21,842 ms of a 21,879 ms statement for one 5,000-row batch on a 200k-row clone, against 16 ms for the replays cascade whose column is indexed, and 17 ms with this index present. Partial rather than plain because rotated_to is unwritten by design, so the index is empty -- 8 kB against 1368 kB -- and costs nothing until the column acquires a writer. (2) kb_oauth_refresh_tokens_rotated_to_fkey redefined from NO ACTION to ON DELETE SET NULL: batched deletion plus the sweep permanent exclusion of evidence-bearing rows means a descent link can straddle a batch boundary and raise 23503, aborting that run and every subsequent one since the offending row never leaves. Latent while nothing writes the column, armed for whoever implements the chain-topology capability it is kept for. (3) kb_oauth_refresh_replays_token_id_fkey redefined from ON DELETE CASCADE to ON DELETE RESTRICT so recorded replay evidence cannot be destroyed by a retention job. The sweep already excludes those rows by predicate, but that predicate races under READ COMMITTED -- a replay committed while the sweep blocks on the row FK KEY SHARE lock is deleted anyway, because the reaper resumes against its original snapshot; reproduced against a clone. RESTRICT makes it a constraint rather than a filter, fires only in the race, and self-heals because the next run predicate sees the committed evidence. Narrows no existing path: nothing else deletes from kb_oauth_refresh_tokens, and both administrative revokers and the AS own rotation UPDATE revoked_at rather than deleting. (4) Indexes on kb_oauth_flow(expires_at) and kb_oauth_refresh_tokens(expires_at, chain_expires_at), which had no index for the column the sweep filters on -- harmless while a backlog exists, but a full table pass every night in steady state. Additive in effect: two CREATE INDEX plus one partial, two constraint redefinitions that only widen what a DELETE may do (SET NULL and RESTRICT both replace behaviour reachable solely by a delete that no shipped binary performs), and one COMMENT. No column, type or function signature changes, and a lagging binary names none of these objects.'
);
