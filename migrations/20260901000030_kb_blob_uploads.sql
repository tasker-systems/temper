-- kb_blob_uploads + kb_blob_upload_segments: the staging half of segmented blob upload
-- (spec: binary blobs, 2026-09-01, D7 — segmented upload follows the segmented-ingest
-- precedent, begin/append/finalize over the same content-addressed target).
--
-- ── Pre-ledger, by design ──────────────────────────────────────────────────────
-- These rows are TRANSPORT state: bytes staged across several HTTP requests between
-- begin and finalize. They never ride events — blob_commit refuses any bytes argument
-- outright (20260901000010: hash-not-bytes enforced absolutely) — so this table pair has
-- no projector and deliberately NO replay story: the ledger's business begins at
-- finalize, when the assembled whole first has a hash and kb_blobs gets its row. The
-- substrate's other tables are event-projected or deterministically materialized; this
-- is the one table pair that is neither, and that exclusion is the contract, not an
-- omission (staged sessions must also be invisible to replay's byte-diff, which reads
-- only its enumerated tables).
--
-- A staged session is not a blob, not a resource, not an edge: nothing in the graph
-- surfaces can see it, and its only gate is owner-equality on the session row — never
-- blob_readable_by_profile, which is the read gate for COMMITTED blobs (20260901000020
-- names that predicate for blob read surfaces; a staged session is caller-private until
-- finalized, the ingest precedent of list_blocks gating).
--
-- ── Lifecycle ──────────────────────────────────────────────────────────────────
-- Segments are immutable once landed: the idempotent-append key is (upload_id, seq,
-- segment_hash), and a differing hash at an occupied seq is a conflict, not a supersede
-- (the assembled whole must stay unambiguous). The pair dies at finalize (success);
-- every finalize FAILURE keeps it (resumable), and abandonment is left to a TTL reaper
-- — declared as a hole, not silently cleaned (decision 2026-09-01, keep-and-declare).
--
-- No DEFAULT-vs-identity question here, unlike kb_blobs: a staging session is not a
-- ledger entity, so the server mints the id freely.

CREATE TABLE kb_blob_uploads (
    id               UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    owner_profile_id UUID NOT NULL REFERENCES kb_profiles(id),
    -- The home the assembled blob will commit INTO. Standing is checked at begin (fail
    -- fast) AND re-checked at finalize before the provider put — standing can change
    -- mid-upload, and the put is the write that needs it. No FK on the anchor, the same
    -- shape kb_blob_homes keeps (20260901000010) — the anchor lives in another table.
    home_table       VARCHAR(64) NOT NULL CHECK (home_table IN ('kb_contexts', 'kb_cogmaps')),
    home_id          UUID NOT NULL,
    -- The media type the blob will commit under. The allowlist is NOT checked here:
    -- the SQL wrapper is the sole allowlist authority, at finalize (D9).
    content_type     TEXT NOT NULL,
    created          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated          TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_kb_blob_uploads_owner ON kb_blob_uploads(owner_profile_id);

CREATE TABLE kb_blob_upload_segments (
    upload_id    UUID  NOT NULL REFERENCES kb_blob_uploads(id) ON DELETE CASCADE,
    seq          INT   NOT NULL,
    bytes        BYTEA NOT NULL,
    -- Bare sha256 hex of this segment's raw bytes — the idempotent-append identity.
    segment_hash TEXT  NOT NULL,
    PRIMARY KEY (upload_id, seq)
);

SELECT declare_migration(
    20260901000030,
    'additive',
    'kb_blob_uploads + kb_blob_upload_segments (D7 segmented upload staging): pre-ledger transport state — bytes never ride events, no projector, deliberately outside replay''s diff set; caller-private via owner-equality on the session row until finalized (never blob_readable_by_profile, which gates COMMITTED blobs); segments immutable (idempotent append keyed on upload_id+seq+segment_hash, differing hash is a conflict), the pair dies at finalize and every finalize failure keeps it resumable — TTL reaper declared as a hole. Design: temper-artifacts specs/2026-09-01-binary-blobs-design.md.'
);
