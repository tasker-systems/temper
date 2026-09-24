# Upload Lifecycle

How binary files flow into Temper: from an authenticated client through the staged-upload
doors to a committed, content-addressed blob — and how committed blobs leave (the delete
act, and erasure). This is the internal companion to the public docs
([Blob uploads](../../docs/concepts/blob-uploads.md),
[Deleting a blob](../../docs/concepts/blob-delete-and-erasure.md)) and to the
self-host configuration ([BLOB_* knobs](../../docs/playbooks/self-host-temper.md)). The
design spec of record is `temper-artifacts:specs/2026-09-01-binary-blobs-design.md`.

> The TypeScript upload/extract/embed pipeline this page once described (the `blob_files`
> table, `/api/upload`, the Vercel durable workflow) was removed; migration
> `20260903000020_kb_blobs.sql` dropped its table. Git history keeps that document.

## The pipeline

**Commit** — a file's bytes become a blob homed in a context (or a map) the caller can
author. One multipart call at or under `BLOB_SINGLE_REQUEST_MAX_BYTES` (4 MB, deliberately
under the platform's request cap); beyond it, the segmented upload: begin, append
(each segment's identity is the server's own sha256 of the bytes it receives), finalize
(the whole-file sha256 echoed by the caller is the integrity check). Every failure leaves
the upload resumable. The same doors serve API, MCP (`blob_read`/`blob_manage`), and CLI
(`temper blob put` segments automatically).

**Storage** — bytes live external to the database at a content-addressed pathname
(`blob_pathname`); the `kb_blobs` row carries the hash, media type, home, and lifecycle.
Hashes are unique per home (owner-scoped uniqueness), so re-committing identical bytes into
a home resolves to the same logical content, while a strike of one row never touches
another principal's identical bytes (custody, never bytes).

**Reaping** — an upload that stalls between begin and finalize is abandoned state, and
abandonment is left to a TTL reaper, never silently cleaned: `blob_reap_service` sweeps
staged uploads untouched past `BLOB_UPLOAD_STAGING_TTL_SECONDS` (default 24 hours) on its
cron, and every reap is recorded. Finalize success retires the staged rows; every failure
keeps them resumable.

**Custody and deletion** — a committed blob is struck by `DELETE /api/blobs/{id}` under the
two-arm custody gate (delete standing over every live relation's resource peer, else the
home custodian); already-struck and unknown ids read the same 404. Released bytes delete
post-commit and are watched by the erasure fence. See
[Deleting a blob](../../docs/concepts/blob-delete-and-erasure.md).

**Erasure** — the act's blob arm is home-pure: every live row homed in the erased governed
contexts is struck with the estate, whoever committed it (disclosed at commit time — the
commit response carries the estate-scope disclosure); rows in ungoverned homes survive,
named in the record. Bytes release only when no live record carries the same hash.

## Configuration

| Knob | Default | Purpose |
|---|---|---|
| `BLOB_ENABLED` | unset (enabled) | `false` closes the blob doors deliberately — fails closed even with credentials present; the MCP tools are not advertised |
| `BLOB_MAX_BYTES` | 64 MB | per-blob cap |
| `BLOB_CONTENT_TYPE_ALLOWLIST` | png, jpeg, webp, svg, gif, pdf | media types the doors admit |
| `BLOB_SINGLE_REQUEST_MAX_BYTES` | 4 MB | above this the CLI segments automatically |
| `BLOB_UPLOAD_STAGING_TTL_SECONDS` | 24 hours | how long a stalled staged upload survives before the reaper sweeps it |

## Named gaps

Open work, stated so absence is never read as coverage:

- **No per-owner aggregate bounds** — nothing caps open staged sessions or total staged
  bytes per principal (task `01a0723e-cfe5-7080-9e0e-9b3323c25080` in the temper vault;
  shipped 2026-09-24 to PR #950 — pending merge).
