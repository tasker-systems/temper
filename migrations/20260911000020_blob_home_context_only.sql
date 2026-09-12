-- The blob-home exclusion (ruled 2026-09-11 with Pete; the decision lives in temper:
-- "Blobs home in contexts only — the cogmap-homed arm is examined-and-deliberately-
-- excluded"): a blob homes in a context. A map is a distilled, telos-driven view OVER
-- resources — the corpus reference path is map → resource → blob — and a blob homed IN a
-- map is data stored in a lens. This completes the posture the relate door already
-- carries (blob↔cogmap and blob↔blob relations refused there, per the delete-act goal's
-- closure): blobs and maps are fully disjoint, and blob custody always resolves through
-- contexts. The goal's named-open ("cogmap-homed homes have no custodial resolver —
-- unstriking until a ruling names the home's custodian") is thereby settled as EXCLUDED,
-- not ruled: no custodian is ever needed, because no row can exist.
--
-- Landed while the class is PROVABLY EMPTY — no surface has ever committed a blob into
-- a cogmap (community: never; enterprise: the blob feature is not rolled out, delete
-- gates that rollout), so the constraint validates over a table with nothing to reject.
-- Home columns survive the strike (the D5.2 emptied shape nulls pathname/type/bytes and
-- keeps hash/home/owner), so the CHECK covers struck rows too.
--
-- Deliberately untouched, as defense-in-depth: the read floor's kb_cogmaps arm
-- (20260906000010's widened floors) and the erasure sweep's 'independent_obligation:
-- home governed by a team or map' outcome (20260911000010's v1 vocabulary, byte-pinned
-- by erasure_fence_service::classify_blob_outcome) — both stay honest should this
-- constraint ever be consciously lifted.
--
-- Additive: one named CHECK, no data touched (the 20260804000020 class).
ALTER TABLE kb_blobs
    ADD CONSTRAINT kb_blobs_home_context_only
    CHECK (home_table = 'kb_contexts');

COMMENT ON CONSTRAINT kb_blobs_home_context_only ON kb_blobs IS
'the blob-home exclusion (ruled 2026-09-11): a blob homes in a context — a map is a '
'distilled view over resources, not a data store, so a cogmap is never a blob''s home. '
'A POSITIVE invariant, not a cogmap-shaped hole: any future home kind must consciously '
'confront this constraint. Covers struck rows too (the D5.2 emptied shape keeps home '
'columns). The read floor''s cogmap arm and the erasure sweep''s '
'''independent_obligation'' outcome remain as defense-in-depth.';

SELECT declare_migration(
    20260911000020,
    'additive',
    'the blob-home exclusion: kb_blobs.home_table is constrained to kb_contexts — the '
    'cogmap-homed arm of the delete-act goal (01a07684) settles as '
    'examined-and-deliberately-excluded instead of a custodian ruling. Landed while the '
    'class is provably empty (no surface has ever committed a blob into a cogmap; the '
    'blob feature is pre-enterprise-rollout), so the check validates over nothing. The '
    'read floor''s cogmap arm and the erasure sweep''s independent_obligation outcome '
    'stay, deliberately, as defense-in-depth.'
);
