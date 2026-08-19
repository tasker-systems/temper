# WS6 re-home: `temper_next` → `public` (post-flip namespace collapse)

**Date:** 2026-06-25
**Status:** Design — approved, pending spec review
**Author:** Pete Taylor (with Claude)
**Supersedes (operationally):** the rename-promote step in `docs/guides/ws6-flip-runbook.md` and `docs/guides/ws6-endgame-collapse-runbook.md`

## Context

The WS6 flip is **already done and live on prod**. PR #166 (`2fc0412`) collapsed
temper-api to a single substrate backend over the canonical schema, and production
was cut over on ~2026-06-24 via a **search-path flip**:

```sql
ALTER DATABASE neondb SET search_path TO temper_next, public;
```

The collapsed code (its explicit `SET search_path` stripped) resolves the canonical
tables in `temper_next` and the `vector` / `uuid_generate_v7` objects via the `public`
fallback. This was the Neon-native cutover because the original runbook's
**rename-promote** step (rename `temper_next` → `public`) hit a **Neon
extension-ownership wall**: `neondb_owner` cannot relocate the `vector` extension out
of `public`.

The search-path flip is correct but leaves prod in a **split state**: live data in
`temper_next`, dead legacy tables in `public`, and a database-level `search_path`
default carrying the indirection. This spec defines the **re-home** that collapses
that split — moving the canonical objects into `public` and retiring `temper_next` —
without ever moving an extension.

### Why a re-home (not a rename) works

The extension-ownership wall only blocks moving an extension *out of* `public`. The
re-home target **is** `public`, and both extensions **already live there**, so they
never move. We drop the legacy `public` objects and relocate the canonical objects
into `public` per-object via `ALTER … SET SCHEMA public`.

## Ground truth (prod `main` branch, inspected 2026-06-25)

```
DB default search_path : temper_next, public           (the flip hack — to be reverted)
schemas                : neon_auth, public, temper_next
extensions             : vector (public, 0.8.0), pg_uuidv7 (public, 1.6), plpgsql (pg_catalog)
sequences              : 0 in both schemas  (all IDs are UUIDv7 — nothing to relocate)
```

| Object | `public` (legacy) | `temper_next` (canonical/live) |
|--------|-------------------|--------------------------------|
| Tables | 28 | 35 |
| Enums | 8 | 8 |
| Functions | 150 (incl. extension-owned) | 67 |
| `_sqlx_migrations` | ✅ 46 rows (`20260330000001`–`20260623000001`) | — |

**Table overlap:** of the 28 legacy `public` tables, **18 share names** with canonical
(live data is in `temper_next`; these collide on a move) and **10 are public-only**:

```
_sqlx_migrations        ← KEEP (rewrite its rows; not a data table)
kb_backend_selection    ← dead (one-backend collapse)
kb_device_sync_state    ← dead (out of canonical scope, zero live refs)
kb_doc_types            ← dead (doctype concept → kb_properties; PR #159 dropped the cross-namespace lookup)
kb_resource_edges       ← dead (→ kb_edges)
kb_resource_manifests   ← dead
kb_resource_revisions   ← dead (→ kb_block_revisions)
kb_resource_search_index ← dead (FTS retired; zero live refs)
kb_scopes               ← dead
kb_team_resources       ← dead
```

So **27 legacy `kb_` tables** are dropped (18 collide + 9 dead leftovers) and
`_sqlx_migrations` is preserved-and-rewritten. **28 − 27 − 1 = 0** unaccounted.

**Deadness verification (code grounding):** every non-`.sqlx`-cache reference to the
10 public-only tables is either a comment in the canonical migrations
(`-- Was kb_resource_edges`), a canonical *successor* (`kb_edges`, `kb_properties`),
or a generated type name. `kb_resource_search_index` and `kb_device_sync_state` have
**zero live `.rs` references** — the FTS-index and device-sync features are retired in
the collapsed app. The canonical schema header itself lists both as deliberately
out of scope.

## Design

### Posture: one-shot operational transition, `migrations/` untouched

`migrations/` is already the pristine 3-file canonical baseline
(`20260624000001_canonical_schema.sql`, `…02_canonical_functions.sql`,
`…03_canonical_seed.sql`) and is **namespace-free by construction** — every statement
resolves against the connection's `search_path`, so a fresh DB lands the full
substrate in `public`. A fresh system never has a `temper_next` to move or a legacy
`public` to drop.

Therefore the re-home is **not** a sqlx migration. It is:

- `scripts/ws6-rehome-public.sql` — the idempotent one-shot transition.
- `docs/guides/ws6-rehome-to-public-runbook.md` — the operator runbook (rehearse →
  backup → go/no-go → execute → verify → rollback).

The two stale runbooks are marked **superseded** with a pointer to the new one; they
are not rewritten.

### The DDL, in order

All steps are idempotent / guarded so the script can be re-run on a partially-applied
branch without error.

**Step 1 — Drop legacy `public` data tables (27).**
`DROP TABLE IF EXISTS public.<t> CASCADE` for each of the 27 legacy `kb_` tables.
CASCADE clears legacy FKs, indexes, views, and triggers (e.g. the legacy FTS trigger
on `public.kb_resources`). **`public._sqlx_migrations` is NOT dropped.**

**Step 2 — Drop legacy `public` enums (8) and legacy `public` functions, excluding
extension-owned objects.**
- Functions: a DO block iterates `pg_proc` in `public` and drops each **except**
  those owned by an extension (`pg_depend` with `deptype = 'e'`). This protects every
  `vector` / `pg_uuidv7` function while removing the ~legacy temper functions, freeing
  the names for the canonical functions.
- Enums: `DROP TYPE IF EXISTS public.<enum>` for the 8 legacy enums (safe once the
  tables that used them are gone), freeing the names for the canonical enums.

**Step 3 — Re-home canonical objects `temper_next` → `public`.**
- Tables: `ALTER TABLE temper_next.<t> SET SCHEMA public` for all 35. FKs between them
  remain valid (resolved by name within the now-shared schema).
- Functions: DO block over `pg_proc` in `temper_next` → `ALTER FUNCTION … SET SCHEMA
  public` using full signatures from `pg_get_function_identity_arguments`.
- Enums: `ALTER TYPE temper_next.<enum> SET SCHEMA public` for all 8.
- **Body-qualification gate:** before authoring, grep `canonical_functions.sql` for
  `temper_next.` / `public.` literals to confirm the function bodies are unqualified
  (namespace-free) and resolve correctly under `search_path=public` post-move.

**Step 4 — Reconcile `public._sqlx_migrations`.**
- `TRUNCATE public._sqlx_migrations` (drops the 46 legacy lineage rows).
- Insert the **3 canonical baseline rows** (`20260624000001`–`…03`) with
  **real sqlx-computed checksums**. The checksums are derived **once** from an actual
  `sqlx migrate run` against an empty scratch DB (the migration files are committed
  and immutable, so the checksums are stable), then embedded as literal `INSERT`
  values in the script. They are **not** hand-faked.
- **Validation:** `sqlx migrate info` against the re-homed DB must report all 3
  applied, zero pending, no checksum mismatch.

**Step 5 — Revert the search-path default.**
`ALTER DATABASE neondb SET search_path TO public;` (removes the flip hack; the bare
default is `"$user", public`, and `public` suffices since everything now lives there).

**Step 6 — Drop the empty namespace.**
`DROP SCHEMA temper_next CASCADE;` (now empty — guarded so a re-run is a no-op).

### Parity deliverable: `migrations/` ≡ post-re-home `public`

The user requirement is that `migrations/` be the **full, production-correct,
unified-public, from-scratch** representation. Validation:

1. `sqlx migrate run` the 3 canonical files into a **fresh empty Neon branch**.
2. `pg_dump --schema-only` both that branch's `public` and the post-re-home prod
   `public`.
3. Normalize (strip `_sqlx_migrations`, comments, ordering noise) and `diff`.
4. **Any drift is reconciled into `migrations/`** — the from-scratch truth defines the
   shape; if prod ended up with anything `migrations/` doesn't reproduce, that is a
   `migrations/` bug to fix (or a documented, justified prod-only artifact).

Expected drift surfaces: prod-only operational residue the canonical files
intentionally omit. Each diff line is adjudicated, not blanket-accepted.

### Execution sequencing (go/no-go gated)

1. Spec + plan committed.
2. **Rehearse** on a throwaway Neon branch off `main` (`ws6-rehome-rehearsal-2026-06-25`):
   run the full `scripts/ws6-rehome-public.sql`, then verify:
   - read floor: visible resources = **1264**
   - vector chunks = **14840**
   - a real `unified_search` query returns rows
   - an access-gated write probe succeeds
   - `sqlx migrate info` clean
3. **Fresh backup branch** of prod (`ws6-rehome-backup-2026-06-25`).
4. **GO/NO-GO checkpoint → user approval.**
5. Execute the script on prod `main`.
6. **Post-verify** (same five probes) on prod.

### Rollback

- **Pre-execution:** the backup branch is a full point-in-time copy; restore prod from
  it if needed.
- **Search-path safety net:** if a regression appears after the default is reverted but
  before `temper_next` is dropped, re-applying `SET search_path TO temper_next, public`
  restores the prior resolution. Step 6 (drop `temper_next`) is the point of no cheap
  return — it runs last, only after the post-verify probes pass.

## Out of scope

**Rejected / not done here (load-bearing):**
- **Re-home as a sqlx migration.** Rejected: it would carry prod-only cleanup logic
  (`IF temper_next EXISTS …`) in the from-scratch lineage forever, doing nothing on
  every fresh install. The one-shot script keeps the lineage clean.
- **Moving extensions.** Never attempted — the target is `public`, where they already
  live. Moving an extension is the exact operation Neon blocks.
- **Preserving `kb_resource_search_index` / `kb_device_sync_state`.** Dropped, not
  migrated — zero live refs; FTS-index and device-sync are retired features.

**Deferred (separate sessions):**
- **F1 follow-up PR** — readback identity-key injection (CLI `find_task`/`load_tasks`
  + vault filenames), search fixes (context filter / empty-param / unified combine),
  create-guard coverage, F6 `@me` projection dir.
- **CD gating** — decoupling temper-cloud prod deploy from raw `main` merges. Owned by
  the sibling session (`preview/jct/decouple-deploy-from-release`).

## Open verification items (resolved during implementation, not blockers)

- Confirm no `temper_next` table carries a trigger referencing a legacy `public`
  object (would break on the legacy-drop). Expected none — canonical is namespace-free
  and FTS triggers were legacy-only.
- Confirm the function-body grep gate (Step 3) finds no schema-qualified literals.
- Capture the three real sqlx checksums from a clean `sqlx migrate run` before
  authoring Step 4's `INSERT`s.
