-- The erasure act and the block history scrub pin lock_timeout = 0.
--
-- Floored writes now carry a lock bound: the write floor sets lock_timeout for the rest of the
-- write's transaction, and a wait past it answers 503 RESOURCE_BUSY (task
-- 01a0fd12-f4b7-7bd2-81d0-13c0814650d5). The bound is SET LOCAL, per transaction, and the acts
-- take no floor, so today nothing bounds them. That must stay true whatever is set later. The act
-- waits on every writer already holding the resource row; under write load, an act that timed
-- out would roll back every time and never complete, which is a denial of erasure. The scrub is
-- the same act at block grain.
--
-- A function's SET clause applies for the duration of the call and is restored after it, so a
-- lock_timeout set on the pool, the role or the database cannot reach either act. It does nothing
-- for statement_timeout: that timer is armed when the calling statement starts, before the clause
-- applies. No statement_timeout is set anywhere today; if one ever is, these two calls need their
-- own.
--
-- A later CREATE OR REPLACE of either function resets its SET clauses to the ones it names, so it
-- must restate SET lock_timeout = 0 (as resource_erasure_execute already restates its
-- search_path). The witness below fails if it does not.
--
-- Witness: temper-services tests/write_lock_bound_test.rs,
-- the_act_completes_under_a_session_lock_timeout.

ALTER FUNCTION public.resource_erasure_execute(uuid, uuid, uuid, uuid, uuid[])
    SET lock_timeout = 0;

ALTER FUNCTION public.block_history_scrub_execute(uuid, uuid[], uuid, uuid, uuid)
    SET lock_timeout = 0;

SELECT declare_migration(
    20261015100000,
    'additive',
    'ALTER FUNCTION ... SET lock_timeout = 0 on resource_erasure_execute and block_history_scrub_execute. Bodies, signatures, return types, grants and COMMENTs are unchanged. The two acts now run without a lock timeout whatever the session, role or database sets.'
);
