# Blob uploads

**For users, operators, and integrators.** How files become blobs — the surfaces, the
staged lifecycle, the limits, and what happens to uploads that never finish. For what
happens after a blob exists, see [Deleting a blob](./blob-delete-and-erasure.md) and
[Erasure and the terms of use](./erasure-and-terms-of-use.md).

## What a blob is

A blob is a binary file — an image, a PDF, a diagram — stored outside the database at a
content-addressed path, with a `kb_blobs` row carrying its hash, media type, and home. A
blob homes in **one context** — a cognitive map is never a blob home. Whoever can author
into that context can commit there, and the blob inherits the context's visibility. Hashes
are unique per home, so committing identical bytes twice in one home resolves to the same
content, while the same bytes in someone else's home are their own row with their own
lifecycle.

## Committing a file

Every write surface reaches the same doors:

- **CLI** — `temper blob put --home <context> <file>` (or `-` for stdin). This is the
  surface that decides single-shot versus segmented for you: at or under the
  single-request threshold (4 MB by default) the file commits in one multipart call; above
  it, the upload segments automatically — begin, append each chunk (the server hashes the
  bytes it receives per segment), finalize with the whole-file checksum as the integrity
  check. A failed segment leaves the upload resumable, not restarted.
- **HTTP** — `POST /api/blobs` for the single-shot multipart commit, and the staged
  begin/append/finalize doors for segmented uploads. The commit response carries the blob
  id and, where the home is a personal context, the estate-scope disclosure (below).
- **MCP** — `blob_manage` commits and relates; `blob_read` streams a blob's bytes back and
  lists what you can read.

## Limits and configuration

| Limit | Default | Knob |
|---|---|---|
| Per-blob size cap | 64 MB | `BLOB_MAX_BYTES` |
| Single-request threshold | 4 MB | `BLOB_SINGLE_REQUEST_MAX_BYTES` |
| Media types admitted | png, jpeg, webp, svg, gif, pdf | `BLOB_CONTENT_TYPE_ALLOWLIST` |
| Staged-upload lifetime | 24 hours | `BLOB_UPLOAD_STAGING_TTL_SECONDS` |

Operators can close the blob doors entirely with `BLOB_ENABLED=false` — an explicit opt-out
that wins even with credentials present; the endpoints then refuse naming the knob and the
MCP tools are not advertised. The full knob reference, including the storage credentials,
is in [self-hosting Temper](../playbooks/self-host-temper.md).

## Abandoned uploads

An upload that stalls between begin and finalize is staged state, and staging is swept, not
hoarded: a reaper deletes staged uploads untouched past the staged-upload lifetime (24
hours by default) and records every sweep. Finishing the upload before the lifetime passes
needs no special handling — finalize simply retires the staged rows.

## Custody, deletion, and erasure

Committing a blob gives you a row you hold custody of, not a claim on the bytes' history:

- **You can delete it** — `DELETE /api/blobs/{id}` (the delete act, exposed through the
  API and the generated clients) strikes the row under the custody gate and releases the
  bytes, but only when no live record carries the same content hash; identical bytes
  elsewhere are someone else's row and never your casualty. See
  [Deleting a blob](./blob-delete-and-erasure.md).
- **Your erasure strikes what you hold** — every live blob row homed in your governed
  contexts is struck with the estate, whoever uploaded it. This is the line crossed by
  writing into someone else's personal context: files uploaded there die with that
  context's estate, and the commit response names that disclosure.
- **Shared homes keep their files** — a blob you uploaded into a team context stays when
  you are gone; your erasure names it in the record rather than striking it, and your
  attribution breaks (the pseudonym break) while the team keeps the bytes.
