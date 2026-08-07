---
name: sqlx-query-cache
description: How compile-time-checked SQL works in this repo and which .sqlx cache to regenerate after changing SQL or schema. Use when adding or editing a sqlx query! macro, changing a migration, or when an offline build fails on a missing/stale query cache entry.
---

# SQL query checking and the `.sqlx` caches

Production SQL queries use `sqlx::query!()` / `sqlx::query_as!()` / `sqlx::query_scalar!()`
macros for compile-time verification against the actual schema. The exception class is
**exactly one reason** — a `$n::vector` bind the macros cannot type — and it lives in
`crates/temper-substrate/src/readback/mod.rs`, whose module note is the authority on which
reads are in it. Read the count there rather than here: this sentence named
`unified_search` in `search_service.rs` until 2026-08-06, and **both halves were wrong** —
that function was retired with the blended search mechanism, and it had never lived in
that file. Trivial test-fixture lookups may use runtime `sqlx::query()`; substantive test
queries keep macros, cached per-crate (below).

> A runtime read that cannot state the `::vector` reason is drift, not an exception. One
> (`search_exact`) had accumulated by 2026-08-06 and was converted to a macro rather than
> documented, because a read outside the cache is absent from the record the schema/binary
> change detector reads — which is the same reasoning that clawed back sixteen such reads
> on 2026-07-30.

- **Local dev:** Set `DATABASE_URL` — macros check against the live database. Note
  `cargo make` tasks force `SQLX_OFFLINE=true`, so `cargo make check` is the honest local
  probe of the committed caches.
- **CI builds:** `SQLX_OFFLINE=true` with committed `.sqlx/` cache for test jobs; the
  `code-quality` clippy job compiles against a **live** DB, so it will NOT catch a missing
  cache entry — only offline `cargo make check` does.
- **After changing any SQL:** Regenerate the workspace cache with
  `cargo sqlx prepare --workspace -- --all-features`
- **Test-target macro queries** (temper-services' service queries, the e2e suite) are NOT
  captured by the workspace ritual — plain `cargo sqlx prepare` skips test targets, and
  adding `--all-targets` to the *workspace* invocation does not fix it (measured: the root
  cache is unchanged and every test-target entry is still missing). They live in per-crate
  caches regenerated with `--all-targets`: `cargo make prepare-services`
  (`crates/temper-services/.sqlx`) and `cargo make prepare-e2e` (`tests/e2e/.sqlx`). Run
  the matching task after changing test SQL or schema it touches. After a merge that moves
  service code between crates, run the ritual in order:
  `cargo sqlx prepare --workspace -- --all-features` → `cargo make prepare-services`
  (per-crate last).

  Each `prepare` **rewrites its cache directory wholesale** — it prunes entries no longer
  emitted, so orphans clean themselves up; no manual pruning is needed. The corollary is
  that a per-crate cache silently rots whenever a *lib* query's signature changes and only
  the workspace ritual is run (macro resolution falls back to the workspace root `.sqlx`,
  so nothing fails — the stale entries just sit there until the next per-crate `prepare`
  sweeps them). Expect an unrelated-looking pile of `.sqlx` churn on the first run after
  such a drift, and check that each pruned entry has a same-query replacement rather than
  assuming the diff is noise.

  > **temper-api has no per-crate cache, deliberately.** It once did; the queries it
  > existed for moved into temper-services during that extraction, leaving a directory
  > whose every entry was already duplicated in the workspace root. temper-api's test
  > targets now contain **no** `query!`-family macros at all, so the workspace cache covers
  > it entirely. Running a per-crate `prepare` there also emitted the whole *dependency
  > closure* — 255 files against 11 committed — so the task was a trap that invited 244
  > files of noise into a diff. Do not recreate the directory or the task. Verify with a
  > cold `cargo clean -p temper-api && SQLX_OFFLINE=true cargo check -p temper-api
  > --all-targets --features test-db`.
- **Tests always run against a real database** (Docker Postgres locally, CI database in
  GitHub Actions)
