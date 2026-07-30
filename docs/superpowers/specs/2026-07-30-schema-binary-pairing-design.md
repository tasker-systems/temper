# The schema and the binary that speaks it never silently disagree

`[decided — 2026-07-30, Pete]` for the design below. `[observed]` for everything under *Evidence*.
Anything unmarked is inferred.

Goal: `019fb35b-c64e-7cd2-a7c0-aa117d1ab1a7`. Grounded in the production outage of 2026-07-30
(PR #576, ~40 minutes of failing writes). Companion prose: [DEPLOYING.md](../../../DEPLOYING.md).

---

## What this decides, in one paragraph

Every migration declares whether it is safe to apply ahead of the binary that pairs with it. CI
checks that declaration against the compiler's own record of the binary↔schema wire contract — the
committed `.sqlx` cache — and fails on contradiction or on silence. A migration that declares itself
**additive** is applied by the deploy itself, during the build phase, by a binary that contains it;
anything else stays operator-gated exactly as today. PRs that touch `migrations/` get a preview
build, so the schema change is rehearsed against a real database branch before it reaches production.

The classification is not a warning label. It is the routing decision for whether the deploy is
allowed to apply the migration.

---

## The journey, and the two premises that were wrong

The outage was diagnosed the same day and written up in `DEPLOYING.md`. Two of that write-up's
load-bearing claims did not survive contact with the system.

**`CREATE OR REPLACE FUNCTION` cannot change a return type.** `DEPLOYING.md` says the edit "reads
like an additive edit — nothing is dropped", and treats `CREATE OR REPLACE` as the trap. Postgres
refuses the operation outright, so the outage class is *structurally never* a `CREATE OR REPLACE`;
it must be written as `DROP FUNCTION` + `CREATE FUNCTION`, and it was. That inverts the detector:
the tell is not that nothing was dropped, it is that **a `DROP FUNCTION` is present**. Correcting
`DEPLOYING.md` is part of this work.

**The detect signal already existed, committed, in the same commit as the migration.** The break is
a three-line diff in the `.sqlx` offline cache — `"type_info": "Uuid"` → `"UuidArray"` — recorded by
the compiler, requiring no database to read, and consumed by nothing. Most of this design is
arranging for something to read it.

A third thing was true and unexamined: **a merge is not a deploy**, and for PR #576 no deployment of
any kind was created. That is why the window stayed open for forty minutes rather than the minute or
two an operator would normally hold it.

---

## Evidence

Everything in this section was executed, not recalled.

### The outage class leaves a mandatory fingerprint

```
$ psql … -c "CREATE OR REPLACE FUNCTION zz_probe(a int) RETURNS uuid  …"
  CREATE FUNCTION
$ psql … -c "CREATE OR REPLACE FUNCTION zz_probe(a int) RETURNS uuid[] …"
  ERROR:  cannot change return type of existing function
  HINT:  Use DROP FUNCTION zz_probe(integer) first.
```

Across all 148 migrations there are **18 return-type changes in 13 files, and all 18 are written as
DROP+CREATE. Zero are `CREATE OR REPLACE`.** The outage migration is one of them
(`migrations/20260730000010_facet_inner_key_grain.sql:232-236`).

### The wire contract moved, in the cache, in the same commit

```
$ git diff 4a729a7e e917a058 -- .sqlx
  .sqlx/query-177087d2….json  |  2 +-      SELECT facet_set($1,$2,$3,$4,$5)
  .sqlx/query-670f5f95….json  |  2 +-      SELECT property_set($1,$2,$3,$4,$5)
  .sqlx/query-cc4a0def….json  |  2 +-      SELECT facet_set($1,$2)
  -        "type_info": "Uuid"
  +        "type_info": "UuidArray"
```

`4a729a7e` is the binary that was **running**; `e917a058` is the one that **never deployed**.

The cache also records `describe.parameters.Left[]`, so an **argument-list** change surfaces there
too. A gate that watches only `columns[].type_info` sees half the class.

### Migration census — what the ceremony would cost

| | count |
|---|---|
| total migrations | 148 (all created 2026-06-24 → 2026-07-30, post-#166 squash) |
| purely additive | **109 (73.6%)** |
| non-additive | 39 (26.4%) |
| callable-shape-breaking | **24, across 21 PRs** |
| migrations per migration-carrying PR | median 1, mean 1.34, max 4 |
| migration-carrying PRs that also changed Rust | **100 of 106** |

The last row is why a blanket "each migration its own PR" rule is expensive: it does not mainly cost
migration-splitting, it costs *schema-from-code* splitting on ~94% of migration PRs. Scoping the
ceremony to the shape-breaking class costs 21 PRs over 36 days instead.

### Production schema state

```
rows | min_version    | max_version    | all_success
 148 | 20260624000001 | 20260730000010 | t
```

Set membership against `migrations/` on disk is an **exact match in both directions** (`comm` finds
zero on either side), and `sqlx migrate info` reports all 148 `/installed` with no checksum
mismatch. The 2026-06 cutover truncated and re-baselined `_sqlx_migrations`, but it has since
accumulated the full canonical set — it is a trustworthy comparison base today.

### The counterfactual

| | migrations known | max version |
|---|---|---|
| binary at `4a729a7e` (**what was running**) | 147 | `20260728000010` |
| binary at `e917a058` (**never deployed**) | 148 | `20260730000010` |
| **production database** | **148** | **`20260730000010`** |

The database was ahead of the running binary by exactly one migration — detectable by comparing two
lists of integers, with no schema introspection at all. `MIGRATOR` is already declared in three
crates (`temper-api/src/lib.rs:15`, `temper-services/src/lib.rs:36`,
`temper-substrate/src/lib.rs:35`) and nothing in production calls `.iter()` or `.run()`.

### A merge is not a deploy — confirmed, not inferred

`temper-cloud` production deployments bracketing the outage:

| deploy created | commit | PR |
|---|---|---|
| 1785417585 | `4a729a7ef37929f976f0dcbfdf1d42e9692dac70` | #573 |
| **— none —** | **`e917a058` (merged 1785417623)** | **#576** |
| 1785419137 | `e34d0f4774b3f2a651deeac320666e4d899d8552` | #579 |

`e917a058` appears in no deployment record — not production, not preview, not cancelled. The gap
between the two merges is **47 seconds** (`git log --first-parent`: 1785417576 → 1785417623), not the
85 recorded in the goal register; the register should be corrected.

### Deployment topology — one recorded defect closed

`temperkb.io` is the **`temper-ui`** project (`prj_UFUosi5qWyG7Vz830I0pOUkXyynK`, domains include
`temperkb.io`). The Vercel project named `temper-cloud` (`prj_ra0MmQYksfePnXvHiTiOGoKigQvY`) builds
the root `vercel.json` Rust functions and is a different thing from the TypeScript *package* of the
same name — the trap the outage-time assertion fell into. `docs/upload-lifecycle.md:7` states the
opposite and is wrong.

### The preview environment exists and has never run

Every PR gets a Neon branch (`preview/<git-branch>`), cut within ~a minute of push, forked
copy-on-write from `main` — verified at 148 migrations, `max=20260730000010`, identical to
production. So a preview runs **the PR's new binary against `main`'s schema**, which is exactly one
half of the pairing invariant, rehearsed on every push.

Except it does not run at all:

| environment | status, across the entire visible history | duration |
|---|---|---|
| Production | ● Ready (every one) | 4–8 min |
| **Preview** | **Canceled (every one)** | **9–13 s** |

An Ignored Build Step skips every preview. The rehearsal infrastructure is provisioned and inert.

### Build-phase feasibility — measured on a real build

A throwaway branch with `"ignoreCommand": "exit 1"` forced the first preview build this project has
ever run. From its log:

```
Running "exit 1"
Running "vercel build"
Running "install" command: `cd packages/temper-cloud && bun install`...
===== TEMPER_BUILD_PROBE_START =====
  cargo:  PRESENT at /rust/bin/cargo        cargo 1.92.0
  rustc:  PRESENT at /rust/bin/rustc
  rustup: PRESENT at /rust/bin/rustup
  PATH: …:/uv/python/bin:/rust/bin:/usr/local/sbin:…
  DATABASE_URL:          SET
  DATABASE_URL_UNPOOLED: SET
  SQLX_OFFLINE:          SET
  VERCEL_GIT_COMMIT_SHA: SET
  target/ exists: yes
Restored build cache from previous deployment (2zobrgqpD5V4qXyVjwjPus9tXjoQ)
```

Four things follow, and two of them contradict what reading `vercel/vercel@main` predicted:

1. **A `buildCommand` runs on a `framework: null` project whose only outputs are `api/*.rs`**, and
   it runs **before** the Rust builders — matching `sortBuilders` (`@vercel/static-build` priority 0,
   `@vercel/rust` priority 1).
2. **Cargo is on `PATH`.** Source-reading predicted otherwise, because `@vercel/rust` installs its
   toolchain into a scoped env and never mutates `process.env.PATH`. That reading was correct about
   the *builder* and wrong about the *image*: Rust ships pre-installed at `/rust/bin`, with
   `CARGO_HOME`/`RUSTUP_HOME` in the build env. (`~/.cargo exists: no` is the tell.)
3. **`DATABASE_URL_UNPOOLED` is present at build time on a preview** — the direct connection sqlx's
   advisory-lock migrator needs.
4. Preview builds **restore the production build cache**, so they are cheaper than 4–8 min suggests.

`ignoreCommand` is a `vercel.json` property that **overrides the project setting** — *"The build
continues if the command exits with code 1, and is ignored if it exits with 0."* So which PRs get a
preview build becomes a versioned, reviewable decision rather than a dashboard toggle.

### sqlx's own safety properties

`Migrator` defaults to `locking: true` and takes a Postgres advisory lock for exclusive access
(`sqlx-core-0.8.6/src/migrate/migrator.rs:53,146`), so concurrent builds serialise rather than
corrupt. Each migration's DDL and its `_sqlx_migrations` insert run in one transaction, so an
interrupted migration rolls back whole — there is no stuck-failed-migration state unless a migration
opts out with `-- no-transaction`, and none of ours does.

---

## The design

### 1. A migration states its classification, and silence is a failure

Every migration declares whether it is safe to apply ahead of its binary. **A migration that says
nothing is not thereby safe** — the absent statement must be as loud as a wrong one, which means CI
fails on a migration with no declaration at all.

The declaration must be readable by a binary that does **not have that migration**, so it cannot
live only in a file header: `MIGRATOR` embeds only the migrations its binary carries. It is written
into the database by the migration itself, and therefore propagates to every target automatically.

### 2. CI checks the claim against the compiler's record

Two distinct jobs, neither substituting for the other:

- **Change detection.** Diff the `.sqlx` caches against the merge-base. A change to
  `describe.columns[].type_info` **or** `describe.parameters.Left[]` means this PR moves the wire
  contract. Requires `fetch-depth: 0`, which today only the `detect-scope` job carries.
- **Consistency.** The workspace cache is already implicitly verified by every offline build —
  `cargo make check`'s clippy sweep compiles against it under `SQLX_OFFLINE=true`, so a stale entry
  breaks the build. This is why the cache diff is a *reliable* signal: its absence would not compile.

The cross-check is deliberately **asymmetric**:

| situation | verdict |
|---|---|
| wire-diff non-empty, no migration declares shape-breaking | **fail**, naming the query files and the migration |
| migration declares shape-breaking, wire-diff empty | **pass, noted** — a break can be invisible to the cache; failing here trains under-declaration |
| migration added with no declaration | **fail** |

### 3. The deploy applies additive migrations; nothing else

A migration declaring itself **additive** is applied during the build phase, by a binary that
contains it. This is exact rather than a hedge: *additive* is **defined** as safe with any binary in
either direction, so:

- it cannot create a mismatch, by construction;
- `vercel rollback` stays safe — an additive migration left in place under a rolled-back binary is
  harmless, which is precisely what the additive-only invariant asserts;
- the build-command-before-Rust-build ordering stops mattering. A compile failure after an applied
  migration is harmless for this class.

**Shape-breaking migrations are never auto-applied.** They remain operator-gated cutover, per
`DEPLOYING.md`. That is the 26% where a human belongs and where rollback needs thought anyway.

**The build applies the pending set only while every member of it is additive, and halts at the
first shape-breaking migration rather than running past it.** This is not a detail — `MIGRATOR.run()`
applies *all* pending migrations, so a naive call would apply an operator-gated migration as a side
effect of some later, unrelated, additive deploy. Halting is a refusal, not an error: the build says
which migration stopped it and that an operator must take it, and whether that fails the deploy or
merely warns is deliberately left to step 4's implementation, because it depends on whether the
halted migration's binary is the one being deployed.

This runs in **both** environments and means different things in each. On a **preview** it applies
the PR's own migration to the PR's own Neon branch — the rehearsal. On **production** it applies the
additive backlog at deploy time, which is the manual step it replaces.

Migrations are applied by a small Rust binary calling the existing `MIGRATOR`, not by a shell
invocation of `sqlx-cli` and not by a TypeScript re-implementation. Checksums, ledger format and
advisory locking stay in sqlx's own semantics; reimplementing the ledger across a language boundary
is the exact drift this goal exists to eliminate.

### 4. Preview builds are the canary

`ignoreCommand` in `vercel.json` turns preview builds **on for PRs that touch `migrations/`**, and
leaves them off otherwise. `[decided — 2026-07-30, Pete]`

This is the only clause that gets *rehearsal* rather than *detection*: the PR's migration is applied
to its own Neon branch and the PR's binary is built and run against it, before merge. A preview of
PR #576 would have broken there. Cost lands on ~30% of merges, on inherited build cache.

### 5. The running commit becomes answerable

`VERCEL_GIT_COMMIT_SHA` is present at build time. `/api/health` currently reports
`version: env!("CARGO_PKG_VERSION")` — `0.1.0`, unchanged since the crate was created, carrying zero
deploy identity, behind a comment claiming it "can never drift from the crate's actual version"
(true, and precisely why it is useless). Prior art for baking a build-time value and refusing on
mismatch is `crates/temper-ingest/build.rs`.

---

## Rejected, and why

Recorded so they are not re-proposed.

**A cold-start / boot-time schema check.** During the outage **reads were healthy throughout** — only
writes through `facet_set`/`property_set` failed. A boot-time refusal would have converted a
writes-only outage into a total one. The refusal would have been worse than the failure.

**A cron executor** (applying migrations from the existing `/api/embed/warm` path). Mechanically
sound — infrastructure-invoked, authenticated, `maxDuration: 300`, holds `AppState` — and it has the
appealing property that schema can never get ahead of a binary that is doing the migrating. Rejected
because it **cannot fail a deploy** (by the time it runs the binary is already live), and because
Vercel crons are production-only, so it buys no rehearsal.

**`cargo sqlx prepare --check` as the consistency gate.** Build-warmth dependent: run from a crate
directory it captures queries from every path dependency that recompiles in that build, then reports
them "missing" from the per-crate cache. CI is always cold, so it would red on nearly every run.
Observed directly — `cargo make prepare-e2e` produced zero changes while `--check` claimed drift.

**An offline-compile gate** (`SQLX_OFFLINE=true cargo check -p X --features Y --all-targets`) as a
per-crate drift detector. Passes vacuously whenever nothing recompiles, and does not bite on
per-crate mirror staleness because the mirrors are not read. Verified by reverting
`crates/temper-services/.sqlx` to its stale state, forcing a genuine 9.15s recompile, and watching it
compile clean.

**Squawk**, the Rust raw-SQL migration linter. `[research-derived, not reproduced here]` None of its
40 rules touches `CREATE FUNCTION`, `RETURNS`, or `DROP FUNCTION`. Run over this corpus it emits 664
findings, 2 of them on the outage migration, both about lock timeouts. It cannot see our failure
class.

---

## Declared holes

Named, not silently absent.

- **No witness may be authored until its mechanism exists.** With nothing built, "fails against
  current state" is satisfied vacuously by anything.
- **Non-macro `sqlx::query()` calls produce no cache entry**, so their wire contracts are invisible
  to the diff.
- **A function body change that alters semantics without changing its signature** is invisible to
  every part of this design. 45 of the 109 additive migrations redefine a function in place with an
  unchanged signature; they are classified additive here and that may be wrong in specific cases.
- **The refusal face is not designed here.** Its urgency drops sharply once the deploy applies its
  own additive schema, because the unbounded failure direction largely disappears. The material
  exists (`MIGRATOR` in three crates; `AppState` already carries a `tokio::sync::OnceCell`), and
  `create_app`/`create_internal_app` are synchronous (`routes.rs:392`, `:467`), so any probe there
  needs a signature change.
- **Rollback across a shape-breaking migration** remains unspecified, deliberately. `kb_events` is
  append-only and projection rebuild is the established repair shape; a rollback story invented here
  would not be grounded.
- **Enterprise self-hosted cadence** is named as an actor and left unexamined.
- **Whether `main()` runs once per instance or per invocation** on Vercel's Rust runtime is
  unverified. It does not bind this design, which no longer places anything at boot, but it would
  bind any future refusal face.
- **Neon preview branches are cut but never reaped.** Several remain `ready` for long-merged
  branches. A standing cost, independent of this work.

---

## Sequencing

Each step is independently landable and independently useful.

1. **Correct the record.** `DEPLOYING.md`'s `CREATE OR REPLACE` claim, `docs/upload-lifecycle.md:7`'s
   topology claim, and the goal register's 85-seconds figure. Cheapest step, and it stops the wrong
   detector propagating.
2. **Preview builds for migration-carrying PRs.** One `ignoreCommand` in `vercel.json`. Buys the
   canary immediately, before any of the machinery below exists.
3. **The declaration + the CI cross-check.** Classification stated, wire-diff computed, contradiction
   and silence both fail.
4. **Build-phase application of additive migrations**, routed by the declaration.
5. **The running commit on `/api/health`**, from `VERCEL_GIT_COMMIT_SHA`.

Steps 1 and 2 close no clause on their own but make every later step observable. Step 3 is where
`a-classification-is-checkable-against-the-migration-it-describes` is satisfied. Step 4 is where
`a-shape-mutating-or-destructive-change-never-reaches-an-environment-whose-binary-predates-it` stops
depending on an operator remembering.
