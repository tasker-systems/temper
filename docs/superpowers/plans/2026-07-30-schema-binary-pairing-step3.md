# Schema/Binary Pairing — Step 3: Declaration, Cross-Check, and the Macro Allow-List

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a migration state whether it is safe to apply ahead of its binary, make CI check that
claim against the compiler's own record of the wire contract, and make that record faithful by
removing the 56 runtime queries it cannot see.

**Architecture:** Two arcs. **Arc A** converts the 56 reasonless `sqlx::query` call sites to macros
and then pins the remaining 46 exceptions with a reasoned allow-list. **Arc B** adds a classification
a migration writes into the database itself, plus a CI job that diffs the `.sqlx` caches against the
merge base and fails when the wire contract moves without a migration admitting it.

**Tech Stack:** Rust (sqlx 0.8.6 macros), PostgreSQL (migrations + one SQL function), bash (CI guard
tests), Python 3 (the call-site enumerator, already shipped), cargo-make, GitHub Actions.

**Source spec:** [docs/superpowers/specs/2026-07-30-schema-binary-pairing-design.md](../specs/2026-07-30-schema-binary-pairing-design.md)
§1, §2, §6 — goal `019fb35b-c64e-7cd2-a7c0-aa117d1ab1a7`.
**Input classification:** [docs/development/sqlx-macro-exception-classification.md](../../development/sqlx-macro-exception-classification.md).

---

## Land this as two PRs, not one

Arc A and Arc B are different intents and should be split on that, per repo convention:

- **Arc A — "the macro is the rule"** (spec §6). Self-contained; useful even if Arc B never lands.
- **Arc B — "a migration declares its class, and CI checks the claim"** (spec §1 + §2).

**Order is not a preference.** The classification's conclusion is that *conversion must precede
enforcement*: Arc B's wire-diff reads the `.sqlx` caches, and 56 production queries are absent from
them until Arc A converts them. Landing Arc B first would ship a detector that reads as coverage over
a corpus it cannot see — the precise failure spec §6 exists to prevent.

---

## Global Constraints

Copied from the spec and repo convention. Every task's requirements implicitly include this section.

- **Additive-only on `main`.** Arc B adds migrations; each must be additive by its own definition.
- **A shipped migration can never be edited.** `sqlx` compares checksums of applied migrations
  (`sqlx-core-0.8.6/src/migrate/migrator.rs:175`) and refuses on mismatch. The 148 existing
  migrations therefore get their classification from a **new backfill migration**, never by
  retrofitting their files.
- **CI guard tests are pure bash**, live at `.github/scripts/test-*.sh`, and get one step each in the
  `guard-tests` job of `.github/workflows/code-quality.yml` (that job has no cargo, which is what
  keeps it fast).
- **Run `cargo make check` before every commit.** The pre-commit hook runs fmt, clippy, docs,
  OpenAPI, tsc and biome.
- **Regenerate the `.sqlx` caches after any query change**, and read the `sqlx-query-cache` skill
  first — the workspace ritual does not cover test-target queries, and there are **three** caches:
  `./.sqlx` (290 entries), `./crates/temper-services/.sqlx` (377), `./tests/e2e/.sqlx` (10).
- **Never print an environment variable's value** in any script that runs in CI. Names only.
- **Evidence, not assertion.** Where a step asserts a fact, it gives the command that establishes it.

## How to read the tags on each step

Every step declares its relationship to what is already on disk, so the judgment is auditable up
front rather than discovered at implementation time:

- **CONFORM** — honor an existing load-bearing constraint. Cites the disk thing.
- **EXTEND** — build beyond an existing affordance. Cites the spec section authorizing it.
- **AMEND** — deliberately change an existing thing. Cites both.

**This plan deliberately contains almost no code bodies.** The predecessor plan
(`2026-07-30-schema-binary-pairing-observability.md`) authored a shell script body and an "expected:
all assertions passed" line for a script that, when finally run, **failed its own test** — the body
shipped and the correct prose beside it did not. Where a body appears below it is quoted from disk or
from a command actually executed, and says which. Anything else is the implementer's to write against
the real API.

---

# ARC A — The macro is the rule

## Task A1: Convert `readback/mod.rs` and correct its header

The largest cluster and the clearest one: 19 non-macro calls, of which **16 have no reason**. Its
header states the reason for the exemption outright, and the reason is consistency.

**Files:**
- Modify: `crates/temper-substrate/src/readback/mod.rs` (16 call sites; header at `:16-18` and `:1-5`)
- Regenerate: `./.sqlx` (these are temper-substrate queries; confirm which cache absorbs them)

**Interfaces:**
- Consumes: nothing.
- Produces: ~16 new `.sqlx` cache entries. Task A3's allow-list count drops by 16. No API change.

**Quoted invariant, carried verbatim from the classification:**
> *"Conversion is not a one-line swap where the untyped `Row` API is used. Many of these read columns
> via `row.get("origin_uri")`. A macro returns an anonymous struct with typed fields, so the call site
> changes too, and nullability that `Row::get` papered over becomes explicit."*

- [ ] **Step 1: Enumerate this file's sites and confirm which are in scope** — *CONFORM*, to
      `scripts/classify-sqlx-calls.py`, which is the authority for the counts.

```bash
python3 scripts/classify-sqlx-calls.py | grep readback
rg -n 'sqlx::query' crates/temper-substrate/src/readback/mod.rs
rg -n '::vector' crates/temper-substrate/src/readback/mod.rs
```

The three `::vector` sites (`:868`, `:1383`, `:1434` as of this writing — **re-derive, do not trust
these line numbers**) stay runtime and become allow-list entries in Task A3. Everything else converts.

- [ ] **Step 2: Convert one site and prove the shape compiles before doing fifteen more** —
      *AMEND*: the disk thing is the runtime call; spec §6 authorizes replacing it
      (*"Any call site that turns out to have no reason should become a macro"*).

Pick the simplest scalar site. The conversion shape below is **not invented** — it was compiled
against the live dev database during the classification (`cargo check -p temper-substrate`, exit 0):

```rust
// from → to, for a plain scalar. Verified to compile 2026-07-30.
// let id: Uuid = sqlx::query("SELECT id FROM kb_profiles WHERE id = $1")
//     .bind(prod_profile).fetch_one(pool).await?.get("id");
let id = sqlx::query_scalar!("SELECT id FROM kb_profiles WHERE id = $1", prod_profile)
    .fetch_one(pool)
    .await?;
```

Two facts established by that same probe, both of which will come up in this file:
- A **SQL-function call** (`SELECT cogmap_readable_by_profile($1, $2)`) compiles, and types as
  `Option<bool>` — sqlx treats function returns as nullable. Expect to handle the `Option`.
- A **multi-column join through a set-returning function** (`resources_visible_to($1)`) compiles.

- [ ] **Step 3: Verify that one site compiles** — the macros need a live database, not the offline
      cache, because the entries do not exist yet.

```bash
SQLX_OFFLINE=false cargo check -p temper-substrate
```

Prediction, not observation: this passes. If it does not, the site has a reason the classification
missed — **stop and record it** rather than forcing the conversion; that is a finding, not an
obstacle.

- [ ] **Step 4: Convert the remaining sites in this file, in small commits**

Group by function, not by line number. After each group, re-run Step 3's command.

- [ ] **Step 5: Correct the module header** — *AMEND*: the disk thing is `readback/mod.rs:16-18`,
      quoted below; the authorization is that it will be false once Step 4 lands.

The header currently says (verbatim, `:16-18`):

```
//! Most reads are runtime `sqlx::query` (the pgvector `::vector` cast forces runtime; the rest follow
//! for consistency). The SQL is UNQUALIFIED (`kb_*` / `resources_visible_to`) — there is one schema
```

Rewrite the first sentence so it describes what is then true: the `::vector` sites are runtime and
named; everything else is a macro. **Two further claims in the same header are already stale and
should be fixed in this edit:**

1. It calls the module *"read-only parity tooling"* whose purpose is assertion against production
   reads. It is now a **production dependency** — verify before rewording:
   ```bash
   rg -n 'readback::' crates/temper-services/src/services/
   ```
   (At time of writing: `citation_audit_service.rs:129,136` and `evidential_standing_service.rs:38`.)
2. It implies the `::vector` cast covers most of the module. It covers 3 of 19.

- [ ] **Step 6: Regenerate the caches, then verify offline** — *CONFORM* to the `sqlx-query-cache`
      skill. **Read it first**; do not improvise the ritual.

```bash
cargo make check   # gates the offline build against the regenerated cache
python3 scripts/classify-sqlx-calls.py | head -3   # non-macro count must have dropped by ~16
```

- [ ] **Step 7: Commit**

Commit message should carry *why* these were exempt (the header's own "for consistency") rather than
just what changed.

## Task A2: Convert the remaining 40 sites

**Files:** the ~12 files listed under *no technical reason visible* by the enumerator. Re-derive the
list; do not transcribe it from here, because Task A1 changes it.

**Interfaces:**
- Consumes: Task A1's demonstration that the three dominant shapes compile.
- Produces: the non-macro production count falls to **46**, which is exactly Task A3's allow-list size.

- [ ] **Step 1: Re-derive the working list** — *CONFORM* to the enumerator.

```bash
python3 scripts/classify-sqlx-calls.py /tmp/runtime.json
```

- [ ] **Step 2: Convert file by file, one commit per file**, running
      `SQLX_OFFLINE=false cargo check -p <crate>` after each.

File-sized batches are the unit because a reviewer can reject one file's conversion while accepting
another's, and because a failed conversion in one file is then trivially bisectable.

- [ ] **Step 3: Handle `region_clocks.rs:139` deliberately** — *AMEND*; it is the one site that looks
      like a legitimate exception and is not.

It selects between two static literals by anchor (quoted from disk, `region_clocks.rs:132-137`):

```rust
let sql = match anchor {
    HomeAnchor::Context(_) => {
        "SELECT shape_materialized_event_id FROM kb_contexts WHERE id = $1"
    }
    HomeAnchor::Cogmap(_) => "SELECT shape_materialized_event_id FROM kb_cogmaps WHERE id = $1",
};
```

Two `query_scalar!` calls inside the match arms put both statements in the cache. The enum is closed,
so this stays exhaustive. Keep the existing comment's insight — *"the table name cannot be bound as a
parameter"* — and extend it to say why that does not imply runtime here.

- [ ] **Step 4: Regenerate all affected caches and verify** — *CONFORM*, `sqlx-query-cache` skill.

```bash
cargo make check
python3 scripts/classify-sqlx-calls.py | head -3
```

The non-macro production count should now read **46**. If it reads anything else, the difference is
the finding — reconcile it against the classification doc before proceeding.

## Task A3: The allow-list, its reasons, and the check that enforces it

**Files:**
- Create: `.github/scripts/audit-sqlx-macro-exceptions.sh` (or extend the Python enumerator — see Step 1)
- Create: `.github/scripts/test-audit-sqlx-macro-exceptions.sh` (bash guard test)
- Modify: `.github/workflows/code-quality.yml` — one step in the `guard-tests` job
- Modify: `docs/development/code-quality-best-practices.md:165` — the prose rule becomes enforced

**Interfaces:**
- Consumes: Tasks A1–A2 (the count must be 46 before this can be seeded).
- Produces: a baseline-pinned enumeration. Arc B's wire-diff may then claim the `.sqlx` caches cover
  every production query except a named, reasoned 46.

**Quoted invariant, carried verbatim from the spec (§6):**
> *"each allow-list entry carries its **reason** — `vector-cast`, `dynamic-columns`, `dynamic-table`.
> A reason turns an exception into a declaration; a bare exemption list decays into a place to put
> things."*

- [ ] **Step 1: Decide the language, and follow the prior art either way** — *CONFORM* to
      `.github/scripts/audit-grant-sinks.sh`, which is the established shape for exactly this
      (a baseline-pinned enumeration of security-relevant call sites).

Read it before writing anything. Its interface, quoted from `audit-grant-sinks.sh:23-28`:

```
#   .github/scripts/audit-grant-sinks.sh            # verify against the baseline (CI mode)
#   .github/scripts/audit-grant-sinks.sh --list     # just print the current sinks
#   UPDATE_BASELINE=1 .github/scripts/audit-grant-sinks.sh   # rewrite the baseline after review
#
# Exit 0 = set unchanged. Exit 1 = a sink was added/removed/moved-file — review required.
```

Match that interface. The open question is bash-vs-Python: `scripts/classify-sqlx-calls.py` already
does the hard part correctly (brace-matched `cfg(test)` spans, the `sqlx::` path requirement), and
reimplementing that in bash would be a second spelling of one predicate — the drift this repo's
conventions forbid. **Recommendation: the checker shells out to the existing Python enumerator** and
does the baseline comparison itself. Confirm Python 3 is available in the `guard-tests` job before
committing to this.

- [ ] **Step 2: Write the guard test first** — *CONFORM* to the guard-test convention.

It must assert at minimum that: an added non-macro call outside the allow-list fails; a call with a
recorded reason passes; a call inside a `#[cfg(test)]` module is ignored; and — the one that catches
the enumerator itself — a `.query(` that is **not** sqlx (reqwest's `RequestBuilder::query`) does not
count. That last case is not hypothetical: a path-less pattern reported 7 "sqlx calls" in
`temper-client`, a crate with no sqlx dependency.

Follow `audit-grant-sinks.sh`'s harness, which points its scan at a fixture directory via an
overridable variable (`MIGRATIONS_DIR="${MIGRATIONS_DIR:-migrations}"`, `audit-grant-sinks.sh:34`).
Give the new checker the equivalent seam so the test never scans the real tree.

- [ ] **Step 3: Run the guard test and watch it fail.** The checker does not exist yet.

- [ ] **Step 4: Write the checker, seeded at 46 entries — never by transcribing the current state**

**This is the load-bearing instruction of the whole task.** Seeding from "whatever is there today"
would bless the 56 reasonless sites that Tasks A1–A2 exist to remove, and a baseline that blesses the
thing it was built to prevent is worse than no baseline. If the count is not 46, Arc A is not done.

The four reasons and their members are enumerated in
[the classification](../../development/sqlx-macro-exception-classification.md#the-legitimate-exceptions-enumerated-with-their-reasons):
36 `dynamic-table` (`replay.rs`, one **class** entry, not 36 lines), 7 `vector-cast`, 3 `dynamic-sql`.
The spec names three reasons; the classification found the ORDER BY case wants naming too. **Decide
whether `dynamic-order-by` is its own reason or folds into `dynamic-sql`** — it has exactly one
member, `substrate_read.rs:265`, and it is the case the existing prose rule already names.

- [ ] **Step 5: Run the guard test until it passes**

- [ ] **Step 6: Wire it into CI** — *CONFORM*: `.github/workflows/code-quality.yml`, `guard-tests`
      job. Add one step after the last existing `Guard test —` step. Match the surrounding
      indentation (six spaces for `- name:`, eight for `run:`).

- [ ] **Step 7: Make the prose rule point at its enforcer** — *AMEND*
      `docs/development/code-quality-best-practices.md:165`, which currently reads (verbatim):

```
  `query_as!()` / `query_scalar!()`. Runtime `query_as` is acceptable only where a `::vector`
```

Spec §6's whole premise is that *"the rule already exists and is unenforced, which is the same defect
class as the outage itself."* Once the checker lands, the rule has an enforcer and should name it.

- [ ] **Step 8: `cargo make check`, then commit**

---

# ARC B — A migration declares its class, and CI checks the claim

> **Do not start Arc B until Arc A's Step A2/4 reports 46.** Arc B's detector reads the `.sqlx`
> caches; before Arc A those caches are missing 56 production queries.

## Task B1: The classification, written by the migration itself

**Files:**
- Create: `migrations/<ts>_migration_classification.sql` — the table and the declaring function
- Create/modify: a developer-facing note on how to declare (likely `DEPLOYING.md`, which already owns
  the additive-vs-not vocabulary)

**Interfaces:**
- Consumes: nothing.
- Produces: the declaring function and its table. Tasks B2–B3 both depend on the exact function name
  and signature chosen here — **fix them in this task and cite them in the others.**

**Quoted invariants, carried verbatim from spec §1:**
> *"A migration that says nothing is not thereby safe — the absent statement must be as loud as a
> wrong one, which means CI fails on a migration with no declaration at all."*
>
> *"The declaration must be readable by a binary that does **not have that migration**, so it cannot
> live only in a file header: `MIGRATOR` embeds only the migrations its binary carries. It is written
> into the database by the migration itself."*

- [ ] **Step 1: Confirm the constraint that shapes this whole task** — *CONFORM*.

```bash
rg -n 'checksum' ~/.cargo/registry/src/*/sqlx-core-0.8.6/src/migrate/migrator.rs
```

`migrator.rs:175` compares `migration.checksum != applied_migration.checksum`. **Shipped migrations
cannot be edited**, which is why B2 is a backfill and not 148 file edits.

- [ ] **Step 2: Note the convention that already exists, unenforced** — *CONFORM*.

```bash
rg -l -i '^-- *ADDITIVE:' migrations/*.sql | wc -l
rg -o -i '^-- *ADDITIVE:' migrations/*.sql | sed 's/.*:-- */-- /' | sort | uniq -c
```

Observed 2026-07-30: **8** migrations carry an informal marker, in two spellings — 6 `-- Additive:`
and 2 `-- ADDITIVE:` — read by nothing. This is spec §6's observation applying to §1: the declaration
idea is already here and already inert. The new mechanism should supersede it, and Task B2's review
pass should treat those 8 markers as **evidence to check, never as the answer** — they were never
validated by anything.

> ⚠️ **OPEN DECISION — the frame owner's, not the implementer's.** Spec §1 is `[decided]` on the
> *what* (declare; silence fails; it lives in the database) and silent on the *how*. The shape below
> is a **recommendation, tagged EXTEND against spec §1**, and should be confirmed before building:
>
> **One source, not two.** The declaration *is* a `SELECT declare_migration(<version>, '<class>', '<reason>');`
> call inside the migration. CI greps `migrations/*.sql` for it — so the file is checkable without a
> database — and applying the migration writes the row, so it propagates to every target. A separate
> header comment would be a second spelling that can drift from the INSERT; this has one.
>
> A helper function (rather than a raw INSERT per migration) means the table's shape can change later
> without editing shipped migrations — which, per Step 1, is impossible anyway.
>
> **The version argument must equal the filename's timestamp**, which gives CI a second, free check:
> a copy-pasted declaration naming the wrong migration is caught mechanically.
>
> **The class vocabulary is not yet settled, and measurement says so.** Observed 2026-07-30,
> `rg -o -i 'additive|shape-breaking|destructive' DEPLOYING.md`: **13** uses of *additive*, **2** of
> *destructive*, and **zero** of *shape-breaking* — the term the spec's own cross-check table (§2) is
> written in. So `DEPLOYING.md` and the spec do not currently share a vocabulary, and picking one is
> part of this step rather than a lookup. Whatever is chosen, the two documents should end up
> agreeing, since the check's failure message will quote it back at whoever trips it.

- [ ] **Step 3: Write the migration** creating the table and the function. Additive by construction
      (new table, new function). Follow the repo's existing migration header style — read the newest
      migration for the house voice: `head -20 migrations/20260730000010_facet_inner_key_grain.sql`.

- [ ] **Step 4: Apply it locally and confirm the function behaves**

```bash
cargo make db-migrate
psql "$DATABASE_URL" -c "\df declare_migration"
```

- [ ] **Step 5: Regenerate `.sqlx` if any Rust query touches the new table** (none should, yet), then
      `cargo make check` and commit.

## Task B2: Classify the 148 existing migrations

> **This task is discovery, and the plan does not pretend otherwise.** The exact split is **not
> known**, and no number in this plan should be treated as one. The only measurement taken so far is
> a keyword heuristic, and *a verb is not a verdict*.

**Files:**
- Create: `migrations/<ts>_backfill_migration_classification.sql`
- Create: a working classification table (scratch, not committed) to review against

**Interfaces:**
- Consumes: Task B1's `declare_migration` signature — cite it, do not re-derive it.
- Produces: a classified row for all 148, so Task B3's check has no legacy exemption to carve out.

- [ ] **Step 1: Take the heuristic pass as a starting point, and label it as such**

```bash
rg -l -i 'DROP FUNCTION|DROP COLUMN|DROP TABLE|ALTER COLUMN .* TYPE|RENAME (COLUMN|TO)|DROP VIEW' migrations/*.sql
```

Observed 2026-07-30: **33 of 148** match. That is a candidate set for human review, not a
classification. Note it disagrees with the register's *"24 break callable shape across 21 PRs"* —
different questions (any breaking verb vs. callable-signature change), and **neither has been
validated**.

- [ ] **Step 2: Review the 33 by hand.** For each, the question is spec §3's definition, quoted:
      *"additive is **defined** as safe with any binary in either direction."* A migration is
      additive only if both the old binary against the new schema and the new binary against the old
      schema are fine.

- [ ] **Step 3: Spot-check the presumed-additive remainder.** Do not classify 115 migrations as
      additive because a grep did not match them — that is inferring coverage from absence, which
      this goal's own register forbids. Sample enough to establish the heuristic's false-negative
      rate, and **record the sample size and what it found** in the migration's header.

- [ ] **Step 4: Write the backfill migration**, one `declare_migration` call per version.

- [ ] **Step 5: Verify every applied migration now has a classification**

```bash
cargo make db-migrate
psql "$DATABASE_URL" -c "SELECT count(*) FROM _sqlx_migrations m LEFT JOIN <table> c USING (version) WHERE c.version IS NULL;"
```

Expected: `0`. (Substitute B1's real table name.)

- [ ] **Step 6: `cargo make check` and commit**

## Task B3: CI fails a migration that declares nothing

**Files:**
- Create: `.github/scripts/audit-migration-declarations.sh`
- Create: `.github/scripts/test-audit-migration-declarations.sh`
- Modify: `.github/workflows/code-quality.yml` — `guard-tests` job

**Interfaces:**
- Consumes: Task B1's function name; Task B2's guarantee that no legacy migration is unclassified.
- Produces: the "silence fails" half of spec §1. Task B5 consumes the parsed declarations.

- [ ] **Step 1: Write the guard test first.** It must cover: a migration with a valid declaration
      passes; a migration with **no** declaration fails; a declaration whose version argument does not
      match the filename fails; an unknown class token fails. Use a fixture directory seam, as
      `audit-grant-sinks.sh:34` does.

- [ ] **Step 2: Run it, watch it fail** (the checker does not exist).

- [ ] **Step 3: Write the checker.** Pure bash, no database — it parses `migrations/*.sql`. This is
      what makes it runnable in the fast `guard-tests` job.

- [ ] **Step 4: Run the guard test until green.**

- [ ] **Step 5: Wire into CI** — same job, same indentation as its neighbours.

- [ ] **Step 6: `cargo make check` and commit**

## Task B4: Compute the wire diff

**Files:**
- Create: `.github/scripts/sqlx-wire-diff.sh` (name is the implementer's; keep the `sqlx-` prefix)
- Create: `.github/scripts/test-sqlx-wire-diff.sh`
- Modify: `.github/workflows/code-quality.yml` — **note the `fetch-depth` constraint below**

**Interfaces:**
- Consumes: Arc A (the caches must be faithful).
- Produces: a machine-readable verdict — did this PR move the wire contract, and which entries moved.
  Task B5 consumes it.

**Quoted invariant, carried verbatim from spec §2:**
> *"Diff the `.sqlx` caches against the merge-base. A change to `describe.columns[].type_info` **or**
> `describe.parameters.Left[]` means this PR moves the wire contract. Requires `fetch-depth: 0`,
> which today only the `detect-scope` job carries."*

- [ ] **Step 1: Confirm the cache shape the diff reads** — *CONFORM*. Observed 2026-07-30:

```
top-level keys: ['db_name', 'query', 'describe', 'hash']
describe keys : ['columns', 'parameters', 'nullable']
columns[0]    : {"ordinal": 0, "name": "handle", "type_info": "Text"}
parameters    : {"Left": ["Text", "Text"]}
```

- [ ] **Step 2: Confirm there are THREE caches, not one** — *CONFORM*. A diff over `./.sqlx` alone
      silently ignores the majority of entries:

```bash
find . -name '.sqlx' -type d -not -path './target/*'
```

Observed: `./.sqlx` (290), `./crates/temper-services/.sqlx` (377), `./tests/e2e/.sqlx` (10). Decide
explicitly whether `tests/e2e` is in scope — spec §6 scopes the *allow-list* to production paths, and
the same reasoning arguably applies here. **Record the decision either way**; a silently-omitted
cache is the failure mode this whole goal is about.

- [ ] **Step 3: Resolve the merge base the way that actually works in CI** — *CONFORM*, and note the
      hard-won constraint.

`fetch-depth: 0` appears in exactly two places (`rg -n 'fetch-depth' .github/workflows/`):
`ci.yml:43` (the `detect-scope` job) and `release-tag.yml:28`. The `code-quality` jobs do **not**
carry it, so a naive `git merge-base origin/main HEAD` there resolves nothing.

> ⚠️ **Learn from Arc-adjacent experience.** The predecessor task assumed a git base was available in
> a build environment and shipped twice before measuring: `VERCEL_GIT_PREVIOUS_SHA` was unset exactly
> when it mattered, and then `merge-base` could not resolve at all against a shallow clone, because a
> shallow boundary commit records no parents. **Establish what this job's checkout actually provides
> before designing against it** — add `fetch-depth: 0` to the job deliberately, or run the diff in
> `detect-scope` which already has it.

- [ ] **Step 4: Write the guard test first**, with fixture cache files rather than the repo's real
      ones. It must distinguish: a changed `type_info` (wire move), a changed
      `parameters.Left[]` (wire move), a **new** entry (not a move), a deleted entry (decide and
      document), and a changed `query` text with identical types (**not** a wire move).

- [ ] **Step 5: Run it, watch it fail. Step 6: Write the differ. Step 7: Green.**

- [ ] **Step 8: Wire into CI, `cargo make check`, commit.**

## Task B5: The asymmetric cross-check

**Files:**
- Modify: the Task B4 script, or a thin new one that consumes both it and Task B3's parser
- Modify: its guard test

**Interfaces:**
- Consumes: Task B3 (declarations) and Task B4 (wire diff).
- Produces: the verdict that closes
  `a-classification-is-checkable-against-the-migration-it-describes`.

**Quoted invariant, carried verbatim from spec §2 — the asymmetry is the design, not an oversight:**

| situation | verdict |
|---|---|
| wire-diff non-empty, no migration declares shape-breaking | **fail**, naming the query files and the migration |
| migration declares shape-breaking, wire-diff empty | **pass, noted** — a break can be invisible to the cache; failing here trains under-declaration |
| migration added with no declaration | **fail** |

- [ ] **Step 1: Write the guard test first — one case per row of that table, plus the boring case**
      (no migration, no wire diff → pass). The middle row is the one most likely to be
      "simplified" into a failure by a well-meaning implementer; the test is what prevents that.

- [ ] **Step 2: Run it, watch it fail. Step 3: Implement. Step 4: Green.**

- [ ] **Step 5: Verify against the outage that motivated all of this** — the strongest available
      test, and it uses real history rather than a fixture.

Migration `20260730000010` (PR #576) changed `facet_set`/`property_set` from `RETURNS uuid` to
`RETURNS uuid[]`, and the `.sqlx` diff in that same commit was a 3-line `"type_info": "Uuid"` →
`"UuidArray"`. Run the cross-check against that commit range and confirm it **fails** — under this
plan the migration would have had to declare shape-breaking, and the wire diff is non-empty either
way.

If it does not fail, the mechanism does not catch the case it was built for, and that is a stop
condition rather than a tuning problem.

- [ ] **Step 6: `cargo make check` and commit**

---

## Self-Review

**Spec coverage.** §6 → Tasks A1–A3. §1 (declaration, silence fails) → B1–B3. §2 (change detection,
cross-check) → B4–B5. §2's "consistency" half needs no task: it is already implicit, since
`cargo make check`'s clippy sweep compiles against the cache under `SQLX_OFFLINE=true` and a stale
entry breaks the build — which is *why* the cache diff is a reliable signal. Spec §3/§4/§5 are steps
4, 2 and 5 of the sequencing and out of scope here.

**Placeholder scan.** No `TBD`/`TODO`. Two items are deliberately *open* rather than placeheld, and
both are marked ⚠️ with who owns them: the declaration mechanism (B1 Step 2, frame owner's call) and
the `dynamic-order-by`-vs-`dynamic-sql` naming (A3 Step 4). The 148-migration split is explicitly
labelled unknown in B2 rather than guessed.

**Type consistency.** The one cross-task identifier is B1's declaring function, and B2/B3 are told to
cite B1 rather than re-derive it — which is why B1's Interfaces block says to fix the name there.
Counts are consistent: 102 non-macro today, 56 converted by A1+A2, 46 seeding A3's allow-list.

**What this plan does not carry, on purpose.** No shell or SQL bodies for the four new checkers. The
predecessor plan authored one and it was wrong in a way that only running it could reveal; every
checker here instead gets its **interface** (from `audit-grant-sinks.sh`), its **test cases**, and a
test-first sequence. The one code block quoted as a conversion example was compiled before being
written down, and says so.
