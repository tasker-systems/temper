# Cloud-Only Vault: Deprecating the Local Vault, Sync Engine, and Local Indexing

**Status:** Draft for plan-writing
**Date:** 2026-05-21
**Goal:** `path-to-alpha`
**Predecessor specs (informing this design):**
- `2026-05-01-cloud-first-reframe-and-manifest-redefinition-design.md`
- `2026-05-07-wave1-phase3-dbbackend-design.md` (the `DbBackend` foundation)
- `2026-05-10-cloud-only-sync-and-find-resource-design.md` (local-mode sync fixes — largely superseded here)
- `2026-05-18-wave1-phase5-surface-dispatch-unification-design.md`

## Summary

Temper currently runs in two modes. In **local mode** the on-disk vault is the
authoritative store: the CLI writes markdown files, tracks them in a per-device
manifest, and reconciles with the cloud through a manifest-based three-way merge
(`temper sync`). In **cloud mode** the cloud is authoritative and writes route
through the API. The two modes are selected at runtime by `TEMPER_VAULT_STATE`.

This design **deprecates local mode entirely.** Temper becomes cloud-first and
cloud-only: the cloud is the single source of truth for every resource, and
every write goes through the API. The on-disk vault is demoted from an
authoritative store to a **read-only local projection** — a materialized copy
of cloud state, refreshed on demand, that exists purely so `ripgrep`/`find` and
human browsing work over a real directory tree.

The redesign removes ~11,000+ lines of local-vault machinery: the `VaultState`
mode switch, the `Manifest` and its IO, the ~2,900-line sync engine, the
three-way merge, the ~5,400-line `VaultBackend`, and local HNSW vector
indexing. It keeps `CloudBackend`, the `Backend` trait, `DbBackend`, the API
services, and `temper-client` — the cloud half is already built.

There is **no migration and no deprecation window.** The cloud is assumed
current (sync has kept it so), the cutover is a clean break, and the work is
sequenced across multiple PRs only so that each lands green.

## Motivation

The two-mode architecture has carried a permanent tax:

- **The sync engine is the most complex subsystem in the codebase** — a
  three-way state machine (`Clean`/`LocalModified`/`RemoteModified`/`Conflict`/
  `Pending`/`LocallyMissing`), paragraph-granularity diffing, and conflict
  annotation — and it exists solely to reconcile a local authoritative store
  that a cloud-only product does not need.
- **Every write path is doubled.** `VaultBackend` and `DbBackend` both
  implement the `Backend` trait; per-doctype write handlers, translators, and
  tests exist twice. Phase 3/4/5 of Wave 1 spent significant effort keeping the
  two halves consistent.
- **~95% of tests run local-mode only**, so cloud-mode regressions are
  under-covered while local-mode machinery — the part being removed — is
  over-covered.
- **The local manifest is per-device and fragile.** Recovery flows
  (`LocallyMissing`, `sync refresh` vs `sync run`) exist only to paper over the
  fact that a device's local view can diverge from the cloud.

A knowledge base whose value proposition is *cross-session, cross-device
continuity for AI agents* is a cloud product. The local vault's only durable
value is grep-ability — and that is fully served by a read-only projection
without any of the reconciliation machinery.

## Scope

### In scope

- Remove the `VaultState` enum, `TEMPER_VAULT_STATE`, `VaultState::from_env()`,
  `backend_select`'s dispatcher, `surface_for_state`, and `Surface::CliLocalVault`.
  The CLI unconditionally uses `CloudBackend`.
- Remove `VaultBackend` (`crates/temper-cli/src/vault_backend/`) in full.
- Remove the `Manifest` family (`ManifestEntry`, `ManifestEntryState`), the
  `temper-core` manifest/sync module, and `manifest_io`.
- Remove the sync engine (`crates/temper-cli/src/actions/sync.rs`), the
  `sync` command (`commands/sync_cmd.rs`), the `push` command, the three-way
  merge (`crates/temper-ingest/src/merge.rs`), the `similar` dependency if
  unused elsewhere, the `temper-client` `SyncClient`, and the `/api/sync/*`
  endpoints and their handlers/services.
- Remove local HNSW vector indexing and the local graph-build commands
  (`temper graph build` and any sibling commands that build or query a
  *local* index/graph).
- Introduce a `projection` module in `temper-cli`: full-context pull,
  per-resource refresh, file pruning, and a per-context staleness cursor.
- Add `temper pull <context>` (full-context projection materialization).
- Make `temper resource show <slug>` write the fetched resource to its
  canonical projection path (per-resource refresh + the *read* step of the
  read-before-write discipline).
- Add a lightweight "latest event id for context" query (client method;
  reuse `list_events` ordering+limit, or a dedicated endpoint if too heavy).
- Wire a non-blocking staleness warning into context-touching commands.
- Collapse the two-tier `add`/`import` model: `add` ingests a file/URL into
  the cloud vault with no local registration.
- Adjust `init` (no local vault scaffold), `doctor` (auth/connectivity checks
  instead of manifest health), and `status` (projection staleness instead of
  sync state).
- Update `SKILL.md` and the skill files to encode the read-before-write
  discipline; update `CLAUDE.md`.
- Migrate the test suite off local mode; add projection tests.

### Out of scope

- **The replacement for graph-build.** Identifying and developing concepts and
  documenting relationships across a context is a workflow that needs its own
  redesign — the current graph-build pattern is acknowledged as suboptimal.
  This spec removes the local-index-bound commands. **The ability to do
  graph-build at all is intentionally lost during the transition period**;
  restoring it (cloud-side, in a better shape) is a named follow-on, not a
  deliverable here.
- **Optimistic-concurrency / compare-and-swap on `update`.** The read-before-write
  discipline guarantees you edited from current server state; it does not
  guard against a concurrent writer between your `show` and your `update`. No
  merge, no diff, no version guard — last write wins. This is a deliberate,
  user-confirmed tradeoff.
- **Offline writes.** Cloud-only means writes require network and auth. No
  offline write queue.
- **Renaming the `vault.path` config field.** It is retained as the projection
  root to avoid churn; a rename is a cosmetic follow-up.
- **Multi-device projection coordination.** Each device pulls independently;
  there is no per-device state to coordinate because the projection is
  derivative.

## Architecture

### 1. Cloud-only CLI

The CLI becomes a pure cloud client. The `Backend` trait
(`temper_core::operations::Backend`) is retained as the dispatch contract, but
the CLI keeps **only `CloudBackend`**. Every write follows one path:

```
CLI command → CloudBackend → temper-client → API → DbBackend → Postgres
```

There is no local file IO on the write path. `backend_select::build_backend()`
collapses from a `match VaultState` dispatcher to an unconditional
`CloudBackend` constructor. `CloudBackend` requires an authenticated client;
the `client: None` unauthenticated-degradation path that `VaultBackend`
tolerated is removed — auth is mandatory for writes.

**Removed (with line-count estimates from the architecture survey):**

| Component | Location | ~Lines |
|---|---|---|
| `VaultState` + `from_env` + env var | `temper-core/src/types/config.rs` | ~50 |
| `backend_select` dispatcher | `temper-cli/src/backend_select.rs` | partial |
| `Surface::CliLocalVault` + `surface_for_state` | `temper-core` / `temper-cli` | small |
| `VaultBackend` module | `temper-cli/src/vault_backend/` | ~5,400 |
| `Manifest` family + `manifest_io` | `temper-core` sync module + `temper-cli` | ~230 |
| Sync engine | `temper-cli/src/actions/sync.rs` | ~2,900 |
| `sync` command | `temper-cli/src/commands/sync_cmd.rs` | ~250 |
| Three-way merge | `temper-ingest/src/merge.rs` | ~200 |
| `SyncClient` + `/api/sync/*` | `temper-client` + `temper-api` | moderate |
| Local HNSW index + `graph build` | indexing module + `commands/` | moderate |

**Kept / repurposed:**

- `CloudBackend`, the `Backend` trait, `DbBackend`, all API services — unchanged.
- `temper-client` resource/search/context/profile/events clients — unchanged;
  the sole write path.
- `temper_core::vault::Vault` — the path-layout type. Repurposed as the
  **read-side** projection path builder (`doc_file`, `rel_path`). Its file-path
  *construction* survives; it no longer feeds an authoritative write.
- `Frontmatter` parse/serialize, doc-type schemas, `apply_doc_type_defaults`,
  the frontmatter-assembly helpers — unchanged.
- `lookup.rs` — **repurposed.** Its manifest dependency is dropped. Slug→resource
  resolution moves server-side via `ResourceClient::resolve_by_uri`; the module
  retains only projection-path computation (where on disk a resource's file
  belongs).

### 2. The local projection

The on-disk vault becomes a **read-only projection** of cloud state. "Read-only"
is a *convention*, not a filesystem permission — projection files are written
with normal write permissions (making the binary toggle `0444` on every pull is
more defensiveness than it is worth). The projection is read-only in the sense
that **it is not a write-back surface**: editing a projection file changes
nothing on the server unless an explicit `temper resource update` is run.

The projection lives at the existing `vault.path` (`~/projects/kb-vault` by
default) with the existing layout: `{owner}/{context}/{doc_type}/{slug}.md`.
Cursors live in `.temper/projection/<context>.json`.

**Two refresh granularities:**

- **Full-context** — `temper pull <context>` materializes the entire context
  tree. For when you want `ripgrep`/`find` coverage over a whole context.
- **Per-resource** — `temper resource show <slug>` fetches one resource and
  writes that one file. This is what cloud agents use; they work a single
  resource and never need the whole tree.

**Staleness cursor.** Each pulled context records one cursor sidecar:

```jsonc
// .temper/projection/<context>.json
{ "last_event_id": "<event-id>", "pulled_at": "<rfc3339>" }
```

This is the *entire* projection state — one cursor per context, nothing
per-resource. There is no manifest, no per-file state machine, no hashes.

**Staleness check** is a cheap non-blocking pre-flight. Commands that touch a
context compare the stored `last_event_id` against the server's latest event id
for that context (one call — `list_events` ordered descending, `limit=1`, or a
dedicated lightweight endpoint). If the server is ahead, the command prints one
warning line and proceeds:

```
⚠ projection for '<context>' is stale — run `temper pull <context>` to refresh
```

A single-resource `show` is fresh by definition (it just fetched), so it never
warns. Offline → the check is skipped silently with a debug log.

### 3. Write path and the read-before-write discipline

All writes route through `CloudBackend`; no local file IO. On a successful
`create`/`update`/`delete`, the CLI rewrites (or removes) **that one
projection file** so the local copy immediately reflects new server state.

`temper resource show <slug>` fetches the authoritative copy from the cloud and
writes it to its canonical projection path. This is simultaneously (a) the
per-resource projection refresh and (b) the *read* step of the discipline.

The **read-before-write discipline** — the cloud analogue of Claude Code's
read-before-edit rule — is encoded in `SKILL.md` and the skill files:

1. `temper resource show <slug>` — fetch latest from cloud, write the
   projection file.
2. Edit that file.
3. `cat <file> | temper resource update <slug> [flags]` — push the modification.

The guarantee: step 1 always fetches fresh, so you are provably editing from
current server state, and your modification is intentional rather than a blind
overwrite of an unknown base. There is no merge and no concurrency guard — if
another writer changed the resource between your `show` and your `update`, your
update wins. This is the accepted tradeoff (see Out of Scope).

### 4. Command surface

| Command | Change |
|---|---|
| `temper sync` (`run`/`status`/`refresh`/`reset`) | **Removed** |
| `temper push` | **Removed** |
| `temper pull <context>` | Repurposed — full-context projection materialization |
| `temper resource show` | Cloud fetch → writes the canonical projection file |
| `temper resource create/update/delete` | Cloud write via `CloudBackend`; rewrites/removes the affected projection file on success |
| `temper add` | Two-tier collapses — ingests a file/URL into the cloud vault; no manifest registration |
| `temper graph build` (+ local graph commands) | **Removed** — see Out of Scope |
| `temper init` | No local-vault scaffold; configures auth + projection directory |
| `temper doctor` | Checks auth + cloud connectivity instead of manifest health |
| `temper status` | Reports projection staleness per pulled context instead of sync state |
| `temper search` | Unchanged — cloud semantic/FTS search via `temper-client` |
| `temper export-token` | Unchanged — local token cache remains; old cloud-mode guard removed |

### 5. Local indexing and graph commands

Local HNSW vector indexing is removed. Semantic search is already cloud-only
via `temper-client`; the local index is dead weight.

`temper graph build` and any command that constructs or queries a *local*
graph/index loses its referent and is removed. **Cloud-side graph stays
intact**: `/api/resources/{id}/edges`, `SearchClient::graph_traverse`, and the
`list_events` relationship deltas — relationship data lives in Postgres.

The workflow for identifying/developing concepts and documenting relationships
across a context is deferred to its own design (Out of Scope). The capability
is intentionally lost during the transition.

## Data Flow

### `temper pull <context>`

```
temper pull <context>
  │
  ├─ resolve context (--context flag or config)
  │
  ├─ client.resources().list({ context, active_only: true })
  │
  ├─ for each resource summary:
  │     client.resources().get(id)              → full document
  │     compute projection path via Vault::doc_file(owner, ctx, doctype, slug)
  │     create_dir_all(parent); write frontmatter + body  (normal perms)
  │
  ├─ prune: for each existing file under <ctx>/ whose temper-id
  │         is absent from the listed set → remove it
  │
  └─ record cursor:
        latest_event_id ← list_events(context, limit=1, desc)
        write .temper/projection/<context>.json { last_event_id, pulled_at }
```

`pull` is idempotent — a full overwrite plus prune. No incremental diff.

### `temper resource show <slug>` (per-resource refresh)

```
temper resource show <slug> --type <T> [--context <ctx>]
  │
  ├─ DocType::from_str(<T>)? at clap boundary
  │
  ├─ client.resources().resolve_by_uri(@me, ctx, doctype, slug)  → id
  │     └─ network/auth failure → clear offline error; not-found → clear 404 error
  │
  ├─ client.resources().get(id)                                  → full document
  │
  ├─ render in requested format (stdout)
  │
  └─ write the document to its canonical projection path
        (per-resource refresh — file is fresh, no staleness warning)
```

### `temper resource update` (the discipline's write step)

```
cat <projection-file> | temper resource update <slug> --type <T> [flags]
  │
  ├─ resolve id via resolve_by_uri
  ├─ build UpdateResource command (body trio from stdin + frontmatter flags)
  ├─ CloudBackend.update_resource → temper-client PATCH /api/resources/{id}
  │     └─ auth checked server-side before any write
  ├─ on success: rewrite the projection file from the returned resource row
  └─ emit DomainEvents (RemoteSynced); no PushDeferred path — writes are synchronous
```

### Staleness pre-flight (context-touching commands)

```
command touching <context>
  │
  ├─ read .temper/projection/<context>.json → stored last_event_id (or absent)
  ├─ client: list_events(context, limit=1, desc) → server latest_event_id
  │     └─ offline → skip silently (debug log)
  ├─ if stored absent OR server ahead → print one ⚠ warning line
  └─ proceed regardless (non-blocking)
```

## Error Handling & UX

- **Writes offline / unauthenticated.** Fail fast, before any work:
  `temper is cloud-only — a network connection and `temper auth login` are
  required to create or modify resources`. No partial state, no queue.
- **Projection reads offline.** `ripgrep`/`find` over already-pulled files work
  fully offline — they are just files. `temper pull` and `temper search`
  require network and report a clear connectivity error.
- **`show` for a missing resource.** Distinguish offline (`couldn't reach the
  server to look up <slug>`) from genuine not-found (`<doctype> not found:
  <slug>`).
- **Stale projection.** Non-blocking `⚠` warning line; the command proceeds.
  The user decides whether to `temper pull`.
- **Editing a projection file without updating.** Not detected and not
  guarded — the discipline lives in `SKILL.md`, not in enforcement. A future
  follow-up could add drift detection, but it is out of scope.
- **Removed-command UX.** `temper sync` / `temper push` / `temper graph build`
  are removed from the clap surface; invoking them yields clap's standard
  unknown-subcommand error. No deprecation shim.

## Testing

The test migration is the largest single chunk of work. ~95% of current
e2e/CLI tests exercise local mode.

- **Deleted with their code.** Tests bound to `VaultBackend`, the manifest,
  the sync engine, the merge, and local indexing are removed alongside the code
  they cover (`sync_test.rs`, `graph_build_test.rs`, the `VaultBackend`
  `tests.rs`/`ctx_tests.rs`, `publish_tail_test.rs`, etc.).
- **Re-pointed to the cloud path.** CLI command tests that exercised
  resource CRUD in local mode move onto the cloud path. The e2e harness already
  spawns a real Axum + Postgres, so the infrastructure exists; `cloud_writes_test.rs`
  is the pattern to generalize.
- **New tests.**

| Test | Layer | Asserts |
|---|---|---|
| `pull_materializes_full_context_tree` | e2e | every active resource in a context appears as a file at its canonical path |
| `pull_prunes_resources_removed_on_server` | e2e | a soft-deleted resource's projection file is removed by the next `pull` |
| `pull_records_context_cursor` | e2e | `.temper/projection/<ctx>.json` written with the server's latest event id |
| `show_writes_canonical_projection_file` | e2e | `resource show` refreshes the single file on disk |
| `staleness_warning_when_server_ahead` | e2e | a write after a `pull` makes the next context command print the `⚠` line |
| `staleness_check_skipped_when_offline` | unit (cli) | offline → no warning, no error, debug log only |
| `write_fails_fast_when_unauthenticated` | unit (cli) | a write with no token errors before any network call |
| `update_rewrites_projection_file_on_success` | e2e | post-`update`, the on-disk file matches the returned server row |

- **Workspace verification** (per `feedback_workspace_test_surfaces_pipeline_bugs`):
  every chunk's verification runs `cargo nextest run --workspace` to surface
  feature-unification surprises.
- **Embed-gated tier.** Chunks touching ingest/embed run
  `--features test-db,test-embed` against `tests/e2e/Cargo.toml` locally to
  match the Embed CI job.

## Decomposition

Multiple PRs, sequenced so each lands green. The new projection is built
*additively first*; the mode switch flips only once the projection exists;
deletions follow once the switch is flipped and the old code is unreachable.

### Chunk 1 — Projection module foundation (additive, dark-launched)

`projection/` module in `temper-cli`: full-context pull writer, file pruning,
cursor sidecar read/write. New `temper pull <context>` command using
`ResourceClient::list` + `get`. No removals — purely additive; coexists with
local mode. Validation: `temper pull` materializes a context against a real
test server; cursor written.

### Chunk 2 — Staleness check + per-resource refresh

Client method for "latest event id for context" (reuse `list_events`
ordering+limit; add a dedicated endpoint only if measured too heavy).
Non-blocking staleness pre-flight wired into context-touching commands.
`temper resource show` writes the canonical projection file. Validation: a
post-pull write triggers the `⚠` line; `show` refreshes one file.

### Chunk 3 — `CloudBackend` unconditional; remove `VaultState`

`backend_select` returns `CloudBackend` unconditionally. Remove `VaultState`,
`from_env`, `TEMPER_VAULT_STATE`, `surface_for_state`, `Surface::CliLocalVault`.
`create`/`update`/`delete` rewrite the affected projection file on success.
Validation: full CRUD works cloud-only; no `TEMPER_VAULT_STATE` references
remain.

### Chunk 4 — Delete `VaultBackend` + manifest

Remove `vault_backend/`, `manifest_io`, the `temper-core` manifest/sync module.
Drop `lookup.rs`'s manifest dependency; resolution is server-side. Validation:
workspace compiles; `VaultBackend` and `Manifest` symbols are gone.

### Chunk 5 — Delete sync engine, `push`, merge

Remove `actions/sync.rs`, `commands/sync_cmd.rs`, `temper push`,
`temper-ingest/src/merge.rs`, the `similar` dependency (if unused), the
`temper-client` `SyncClient`, and the `/api/sync/*` endpoints and their
handlers/services. Validation: workspace compiles; no `/api/sync` route.

### Chunk 6 — Remove local HNSW indexing + local graph-build

Delete the local vector index and `temper graph build` (+ local graph
commands). Accept the intentional capability gap; record the follow-on.
Validation: workspace compiles; cloud-side graph endpoints unaffected.

### Chunk 7 — `add` / `init` / `doctor` / `status` cloud-only cleanup

Collapse the two-tier `add`; `init` drops the local-vault scaffold; `doctor`
checks auth/connectivity; `status` reports projection staleness. Validation:
each command behaves per the Command Surface table.

### Chunk 8 — `SKILL.md`, skill files, `CLAUDE.md`

Encode the read-before-write discipline. Rewrite cloud-mode-vs-local-mode
guidance for a single cloud-only reality. Remove `sync`/`push`/`graph build`
references. Validation: docs reviewed; no stale local-mode instructions.

Test migration is folded into each chunk (a chunk deletes its own tests and
re-points the tests for the surface it changes). A final consolidation pass at
the end of Chunk 7 sweeps for any orphaned local-mode test.

### Named follow-ons (capture as task stubs)

- **Cloud-side concept/relationship workflow** — the replacement for
  graph-build. Its own brainstorm → spec → plan. The capability is intentionally
  absent until it lands.
- **Projection drift detection** — optionally warn when a projection file has
  local edits that were never `update`d.
- **`vault.path` → `projection.path` config rename** — cosmetic.
- **Optimistic-concurrency guard on `update`** — revisit only if last-write-wins
  proves painful in multi-agent use.
- **UUID-named projection files** — the on-disk filename is currently the
  `slug`, which exists only for human/agent semantic legibility. The slug is
  derived from a mutable title, so a rename today implies a path change that
  must be reconciled. Naming the projection file by the resource UUID instead
  would make a file's on-disk identity stable and fully decoupled from its
  title — a slug/title change would never move a file. The tradeoff is grep
  ergonomics: a UUID filename is far less legible than a slug for humans
  scanning a directory (a `slug + short-uuid` hybrid, or a slug-based symlink
  alongside a UUID-named file, are possible middle grounds). This is a
  meaningful change with its own moving parts (projection path computation,
  `resolve_by_uri`, pruning, the read-before-write idiom). Deferred to a
  dedicated phase with its own review — explicitly **not** part of this work.
