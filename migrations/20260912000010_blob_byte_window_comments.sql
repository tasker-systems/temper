-- The byte window's comment correction (task 01a09360-e00a-7d90-858d-f4998dd70b6c). The
-- 20260906000010 strike substrate recorded the post-commit release window as "healed on
-- re-upload, watched by the erasure build's queue fence", and 20260909000040's
-- erasure_delete_complete carried the same premise. The axes sitting (2026-09-12, temper
-- research 01a09360-742b-7540-bde7-6a1bce4c0e9c) proved both halves false against the code:
-- the re-commit does not re-put (it dedups against the ghost row and then refuses at D4's
-- presence gate — the "confusing refusal" the N3 widening removed for struck rows,
-- reintroduced for live ones), and the fence's re-derivation read the ghost as re-occupation
-- and closed the residue done, unalerted. A live row over absent provider bytes was
-- reachable, permanent, and silent.
--
-- The window is CLOSED in code by the same build: the commit path's transaction takes the
-- hash advisory lock FIRST and re-derives byte presence under it — restoring a missing
-- object from the caller's own bytes before the row can go live — and every byte release
-- (the delete door's post-commit release through writes::release_blob_bytes, and the fence
-- drain's batched delete) re-derives released-ness under the same locks and holds them
-- across the provider delete. The ghost is unconstructible; the fence remains the
-- retry-plus-age-alerting backstop for provider failures.
--
-- This migration exists because the two COMMENT statements below live in shipped,
-- checksum-locked migration files that are never edited; a `\d` reader inherits the
-- corrected prose from here, and the diff reader gets the story from this header. The Rust
-- doc comments were corrected in place by the same build.

COMMENT ON FUNCTION blob_delete(text, jsonb, uuid, jsonb, uuid, uuid) IS
'the shared emptying act, parameterized by the calling act''s event type, which must be a
registered DOMAIN type — refused here otherwise, in this wrapper''s voice, like an absent or
already-struck blob. Serializes against commits and sibling strikes on the hash (advisory
lock, taken before the row lock); the refcount is same-transaction and live-rows-only, so
released is exact as of this transaction''s commit. The caller releases the provider bytes
AFTER the commit — the delete door through writes::release_blob_bytes, the erasure arm
through the byte-delete fence''s drain — both re-deriving released-ness under the same
advisory lock and holding it across the provider delete — while the commit path
re-derives, and RESTORES from the caller''s own bytes, presence under that lock before any
row goes live. A live row over absent provider bytes is unconstructible (20260912000010;
task 01a09360-e00a-7d90-858d-f4998dd70b6c). The strike folds no edges.';

COMMENT ON FUNCTION erasure_delete_complete(uuid[], text) IS
'Record success. p_resolution distinguishes bytes actually struck from the honest skip: a
hash a live row re-holds at drain time is NOT deleted — under the drain''s advisory locks
the re-derivation is exact, and a live row''s bytes are present, because the commit path
restores presence under the same lock before the row can go live (20260912000010; task
01a09360-e00a-7d90-858d-f4998dd70b6c). A re-holding hash in kb_erased_content is the ruled
custody-never-bytes posture (20260911000000 retired the re-admission refusal): the skip
records the erasure byte obligation''s lawful end.';

SELECT declare_migration(
    20260912000010,
    'additive',
    'The byte window''s comment correction (task 01a09360-e00a-7d90-858d-f4998dd70b6c): re-stamps the blob_delete and erasure_delete_complete COMMENTs, whose shipped files are checksum-locked. The 20260906000010/20260909000040 prose recorded the post-commit release window as healed on re-upload and watched by the fence; the axes sitting proved both halves false (the re-commit dedups against the ghost and refuses at D4 rather than re-putting; the drain read the ghost as re-occupied and closed the residue done, unalerted). The window is closed in code by the same build — the commit restores byte presence under the hash advisory lock, and byte releases re-derive released-ness under the same locks held across the provider delete — so the corrected comments state the serialization, not the old claim. Additive: two COMMENT statements only.'
);
