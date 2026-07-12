-- Serialize the resource body_hash recompute tail against concurrent same-resource appends.
--
-- `_recompute_resource_body_hash` is a read-modify-write over a resource's whole visible block set:
-- it aggregates every non-folded block's chunk hashes into a merkle, then UPDATEs kb_resources.body_hash.
-- Every append lands its own block + chunks in its own transaction and then calls this at the tail
-- (block_append → _project_block_created → _project_blocks). `begin_scoped` runs at READ COMMITTED
-- (temper-substrate/src/writes.rs), so two concurrent appends to the SAME resource each aggregate over
-- a snapshot that misses the other's still-uncommitted block, compute a body_hash over an incomplete
-- set, and both UPDATE. The row lock on the UPDATE serializes the writes but does not re-derive the
-- value — the later committer's stale hash wins, and the resource is left hashing a block set that is
-- missing a block. A subsequent resource_finalize then RAISEs (body_hash ≠ the client's merkle over
-- every segment), so segmented ingest fails nondeterministically. This is the precondition for the
-- K>1 upload fan-out in the ingest throughput spike (task 019f57d2): today only a depth-1 pipeline is
-- safe.
--
-- Fix: take a resource row lock BEFORE the aggregate. In READ COMMITTED each statement takes a fresh
-- snapshot, so a blocking lock that waits out an in-flight append lets the *next* statement (the
-- aggregate) see the now-committed block — the recompute runs over a settled set. The lock is only
-- reached at the tail (after the expensive ~170-row chunk inserts), so those still overlap across
-- concurrent appends — only the cheap recompute is ordered. Sequential appends (today's CLI path) take
-- an uncontended lock and are unaffected.
--
-- The lock mode is FOR NO KEY UPDATE, NOT FOR UPDATE. Each append's own block + chunk inserts already
-- hold a FOR KEY SHARE lock on this kb_resources row (the FK from kb_content_blocks/kb_chunks →
-- kb_resources), and KEY SHARE locks are mutually compatible, so two concurrent appends both hold one.
-- FOR UPDATE conflicts with FOR KEY SHARE, so each append escalating to FOR UPDATE would wait on the
-- other's still-held KEY SHARE — a deadlock (empirically confirmed by the bite test; verified, not
-- trusted). FOR NO KEY UPDATE conflicts with itself (so it still serializes the two recompute tails)
-- but is compatible with FOR KEY SHARE (so it escalates cleanly while the sibling holds only its FK
-- key-share) — no cycle. It is also the exact lock the trailing `UPDATE kb_resources SET body_hash`
-- already takes (a non-key-column update), so this only moves that acquisition earlier.
--
-- Additive-safe: CREATE OR REPLACE with the unchanged (uuid, timestamptz) signature — no DROP (a DROP
-- would break migrate-ahead-of-deploy skew, and `main` stays additive-only). Body is copied verbatim
-- from the live definition (migrations/20260624000002_canonical_functions.sql) with ONE added statement
-- at the top; the aggregate/UPDATE below it are unchanged.
CREATE OR REPLACE FUNCTION _recompute_resource_body_hash(p_resource uuid, p_occurred timestamptz)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_resource_hashes text;
BEGIN
    -- Serialize the recompute tail: wait out any concurrent same-resource append still in flight, so
    -- the aggregate SELECT below (a fresh READ COMMITTED snapshot) sees the settled, committed block set.
    -- FOR NO KEY UPDATE (not FOR UPDATE) so it does not conflict with the FK-induced KEY SHARE locks the
    -- concurrent append already holds — see the header note; FOR UPDATE deadlocks here.
    PERFORM 1 FROM kb_resources WHERE id = p_resource FOR NO KEY UPDATE;
    SELECT string_agg(bh, '' ORDER BY seq) INTO v_resource_hashes FROM (
        SELECT b.seq,
               encode(sha256(convert_to(string_agg(ch.content_hash, '' ORDER BY ch.chunk_index), 'UTF8')),
                      'hex') AS bh
        FROM kb_content_blocks b
        JOIN kb_chunks ch ON ch.block_id = b.id AND ch.is_current
        WHERE b.resource_id = p_resource AND NOT b.is_folded
        GROUP BY b.seq
    ) per_block;
    UPDATE kb_resources
        SET body_hash = encode(sha256(convert_to(coalesce(v_resource_hashes, ''), 'UTF8')), 'hex'),
            updated = p_occurred
        WHERE id = p_resource;
END;
$$;
