# WS6 re-home parity report: `migrations/` ≡ post-re-home `public`

**Date:** 2026-06-25
**Verdict:** ✅ **Equivalent.** `migrations/` is a faithful from-scratch representation of the
production canonical schema. No `migrations/` changes required.

## Method

- **Reference (from-scratch truth):** a clean `cargo sqlx migrate run --source migrations` into
  an empty local `public` (PostgreSQL 18), then `pg_dump --schema-only --schema=public`.
- **Production canonical:** `pg_dump --schema-only --schema=temper_next` of prod `main`
  (PostgreSQL 17), text-rewritten `temper_next.` → `public.` (the `SET SCHEMA` re-home changes
  the namespace, not the shape, so prod `temper_next` today == prod `public` post-re-home).
- Both normalized (strip comments / `SET` / blank lines, one statement per line, whitespace
  squeezed, sorted) and `diff`ed.

Normalized sizes: migrations 582 stmts, prod 579 stmts. Eight diff hunks, all adjudicated below.

## Adjudication

Every difference falls into one of four buckets; none is a structural drift.

### Bucket A — dump-scope artifact (not a schema difference)

- **`public._sqlx_migrations` table + `_sqlx_migrations_pkey`** appear only in the migrations
  dump. The local reference `public` holds the sqlx ledger; the prod dump is of `temper_next`,
  which never held it (the ledger lives in prod `public`). After the re-home, prod `public`
  carries the reconciled 3-row ledger (verified in rehearsal via `sqlx migrate info`). **Not drift.**

### Bucket B — PostgreSQL 17/18 portability (expected, by design)

- **`uuid_generate_v7()`** appears as a standalone `CREATE FUNCTION … SELECT uuidv7()` only in
  the migrations (PG18) dump. On PG17 prod the generator is supplied by the `pg_uuidv7`
  **extension**, so `pg_dump` folds it into the extension rather than emitting a standalone
  function. This is exactly the 17/18 split the canonical schema's `DO $uuid_compat$` block
  encodes (extension on PG17, native-`uuidv7()` alias on PG18). **Not drift.**

### Bucket C — `pg_dump` cross-version deparse cosmetics (semantically identical)

- **CHECK constraints using `= ANY (ARRAY[...])`** on 7 tables (`kb_cogmap_region_members`,
  `kb_contexts`, `kb_edges`, `kb_events`, `kb_properties`, `kb_resource_access`,
  `kb_resource_homes`) render differently:
  - PG18: `ANY ((ARRAY['kb_resources'::character varying, …])::text[])`
  - PG17: `ANY (ARRAY[('kb_resources'::character varying)::text, …])`
  Same predicate, different deparse (whole-array cast vs per-element cast). **Not drift.**

### Bucket D — cosmetic column ordering (functionally equivalent, accepted)

- **`kb_profiles` column order.** Both have the identical 7 columns with identical types,
  defaults, and nullability; only the position of `created` differs:
  - migrations: `… system_access, email, preferences, created`
  - prod:       `… system_access, created, email, preferences`
  `migrations/` was edited (email/preferences relocated) after prod's `temper_next` was built.
  All column access is name-based (`sqlx::query!`, named binds) — no positional `SELECT *`
  indexing or column-list-free `COPY` in the app — so the ordering is immaterial to behavior.
  **Accepted, not reconciled:** rewriting the canonical migration to reorder columns would be
  churn for zero functional gain and cannot retroactively reorder the live prod table anyway
  (column order is fixed at table creation). The from-scratch build remains fully functional.

## What matched exactly

- **All 66 business functions** — the only function diff was `uuid_generate_v7` (Bucket B).
- **Every table, column, type, enum, constraint, and index** — no missing or extra objects;
  the only `<`/`>`-only lines are Buckets A and B.

## Conclusion

`migrations/` reproduces the production canonical substrate in `public` for a fresh system,
modulo PG-version deparse noise and one cosmetic column ordering. It satisfies the
"full, correct-with-production, unified-public, from-scratch" requirement. **No reconciliation
commit needed.**
