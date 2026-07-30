# Schema/Binary Pairing — Observability First Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the three steps of the schema/binary pairing design that need no new machinery — correct the record, turn on the migration canary, and make the running commit answerable — so that every later step is observable when it lands.

**Architecture:** Three independent tasks, each landable alone. Task 1 corrects two repo documents and a temper goal register that currently assert things measurement disproved. Task 2 adds a repo-side `ignoreCommand` so PRs touching `migrations/` get a preview build (production always builds), with a pure-bash guard test in the existing `guard-tests` CI job. Task 3 bakes `VERCEL_GIT_COMMIT_SHA` into the binary via a `build.rs`, following `crates/temper-ingest/build.rs`, and surfaces it on `/api/health`.

**Tech Stack:** Rust (sqlx 0.8.6, utoipa, axum), POSIX sh (Vercel build container), bash (CI guard tests), cargo-make, Vercel + Neon.

**Source spec:** [docs/superpowers/specs/2026-07-30-schema-binary-pairing-design.md](../specs/2026-07-30-schema-binary-pairing-design.md) — goal `019fb35b-c64e-7cd2-a7c0-aa117d1ab1a7`.

## Scope

This plan covers **spec steps 1, 2 and 5**. Spec steps 3 (declaration + wire-diff cross-check + macro allow-list) and 4 (build-phase application of additive migrations) are **deliberately excluded** and each needs its own plan:

- **Step 3** should be planned *after* the ~60 residual non-macro `sqlx::query*` call sites are classified. Writing it now would mean authoring specifics that go stale on contact — the classification is discovery, not implementation.
- **Step 4** changes how schema reaches production and carries the highest risk in the design. It deserves a plan of its own, written against step 3's landed classification.

## Global Constraints

Copied from the spec and from repo convention. Every task's requirements implicitly include this section.

- **Additive-only on `main`.** Nothing in this plan adds a migration; if that changes, the change is not in scope.
- **Anything running in the Vercel build container is POSIX `sh`, not bash.** The build command is spawned via `sh -c`.
- **CI guard tests are pure bash**, live at `.github/scripts/test-*.sh`, and get one step each in the `guard-tests` job of `.github/workflows/code-quality.yml` (that job has no cargo, which is what keeps it fast).
- **Run `cargo make check` before every commit.** The pre-commit hook runs fmt, clippy, docs, OpenAPI, tsc and biome.
- **Never print an environment variable's value** in any script that runs in CI or a build container. Names only.
- **Evidence, not assertion.** Every claim these documents make must be checkable; where a step asserts a fact, it gives the command that establishes it.
- **`unverifiable` is not `wrong`.** Where a build cannot know its commit (a local `cargo build`, a `cargo install`), the recorded value must say *"not recorded"* and never a plausible-looking placeholder.

---

### Task 1: Correct the record

Three artifacts assert things that measurement disproved. This task costs almost nothing and stops the wrong detector propagating into every later step.

**Files:**
- Modify: `DEPLOYING.md:59-66` (the `CREATE OR REPLACE` claim)
- Modify: `docs/upload-lifecycle.md:7` (the topology claim)
- Modify (via CLI, not a file): temper goal `019fb35b-c64e-7cd2-a7c0-aa117d1ab1a7` — two occurrences of the 85-second figure

**Interfaces:**
- Consumes: nothing
- Produces: nothing consumed by later tasks. Independently landable.

- [ ] **Step 1: Establish the claim empirically before editing the doc that gets it wrong**

```bash
psql "postgresql://temper:temper@localhost:5437/temper_development" \
  -c "CREATE OR REPLACE FUNCTION zz_probe(a int) RETURNS uuid   LANGUAGE sql AS \$\$ SELECT gen_random_uuid() \$\$;" \
  -c "CREATE OR REPLACE FUNCTION zz_probe(a int) RETURNS uuid[] LANGUAGE sql AS \$\$ SELECT ARRAY[gen_random_uuid()] \$\$;" \
  -c "DROP FUNCTION IF EXISTS zz_probe(int);"
```

Expected: the first `CREATE FUNCTION` succeeds, the second fails with:

```
ERROR:  cannot change return type of existing function
HINT:  Use DROP FUNCTION zz_probe(integer) first.
```

- [ ] **Step 2: Confirm the corpus agrees — every return-type change is DROP+CREATE**

```bash
rg -c 'DROP FUNCTION' migrations/*.sql | wc -l          # expect 31
rg -l 'CREATE OR REPLACE FUNCTION' migrations/*.sql | wc -l
sed -n '232,236p' migrations/20260730000010_facet_inner_key_grain.sql
```

Expected: the outage migration shows `DROP FUNCTION facet_set(...)` followed by `CREATE FUNCTION ... RETURNS uuid[]` — not a `CREATE OR REPLACE`.

- [ ] **Step 3: Rewrite the DEPLOYING.md paragraph**

Replace the first paragraph of `### A function's return type is a wire contract with the binary` (currently `DEPLOYING.md:61-66`) with:

```markdown
A function signature change reads like an ordinary edit: every caller in the repo is
updated in the same commit, and the non-additive examples beside this one — a rename,
a destructive collapse, a search-path flip — are all table-shaped, so it matches none
of them. It is not additive. The binary decodes that function's result **by type**, so
**the running binary is a caller you did not update**, and the invariant's second
clause — *old code against the new schema* — is what breaks.

**The tell is a `DROP FUNCTION`, not the absence of one.** Postgres refuses to change a
function's return type via `CREATE OR REPLACE` (`ERROR: cannot change return type of
existing function`), so a return-type change **must** be written as `DROP FUNCTION` +
`CREATE FUNCTION`. All 18 return-type changes in this repo's migrations are written that
way; none is a `CREATE OR REPLACE`. Grep the migration for `DROP FUNCTION`, then grep the
callers of what it drops.
```

- [ ] **Step 4: Correct the topology claim in `docs/upload-lifecycle.md:7`**

The line currently reads:

```markdown
temper-cloud runs two runtimes in a single Vercel project at **temperkb.io**:
```

Replace with:

```markdown
temper-cloud runs two runtimes in a single Vercel project. That project is named
**`temper-cloud`** (`prj_ra0MmQYksfePnXvHiTiOGoKigQvY`) and is **not** the project
serving `temperkb.io` — that hostname belongs to the **`temper-ui`** project
(`prj_UFUosi5qWyG7Vz830I0pOUkXyynK`), which reverse-proxies `/api`, `/mcp`, `/oauth`
and `/.well-known` to `API_BASE_URL`. Note also that the Vercel *project* named
`temper-cloud` is a different thing from the TypeScript *package* of the same name:
```

- [ ] **Step 5: Correct both occurrences of the 85-second figure in the goal register**

The register lives in temper, not in the repo, so this is a CLI edit. There are **two**
occurrences, in different sections. Use the show-edit-cat idiom, one resource per call,
with stdin explicit — never inside a redirected loop.

```bash
temper resource show 019fb35b-c64e-7cd2-a7c0-aa117d1ab1a7 \
  | python3 -c "import json,sys; open('/tmp/reg.md','w').write(json.load(sys.stdin)['content'])"
grep -n '85 seconds' /tmp/reg.md      # expect exactly 2 hits
```

Edit `/tmp/reg.md`, replacing both with `47 seconds`, and add the measurement beside the
first one so the number carries its own evidence:

```markdown
`[observed]` PRs #573 and #576 merged **47 seconds** apart
(`git log --first-parent`: committer timestamps 1785417576 → 1785417623), and #576's merge
commit `e917a058` produced **no deployment record of any kind** — not production, not
preview, not cancelled. The earlier figure of ~85 seconds was wrong.
```

Then write it back:

```bash
cat /tmp/reg.md | temper resource update 019fb35b-c64e-7cd2-a7c0-aa117d1ab1a7
```

- [ ] **Step 6: Verify the corrections landed**

```bash
rg -n 'DROP FUNCTION, not the absence' DEPLOYING.md
rg -n 'temper-ui' docs/upload-lifecycle.md
temper resource show 019fb35b-c64e-7cd2-a7c0-aa117d1ab1a7 \
  | python3 -c "import json,sys; c=json.load(sys.stdin)['content']; print('85 seconds:', c.count('85 seconds'), '| 47 seconds:', c.count('47 seconds'))"
```

Expected: both greps hit; the register reports `85 seconds: 0 | 47 seconds: 2`.

- [ ] **Step 7: Commit**

```bash
cargo make check
git add DEPLOYING.md docs/upload-lifecycle.md
git commit -m "docs: the tell is a DROP FUNCTION, and temperkb.io is the temper-ui project

DEPLOYING.md's worked example named CREATE OR REPLACE as the trap. Postgres refuses
to change a return type that way, so the outage class is structurally always
DROP FUNCTION + CREATE FUNCTION — which inverts the detector the section was teaching.
All 18 return-type changes in migrations/ are written that way; none is a CREATE OR
REPLACE.

upload-lifecycle.md placed temper-cloud 'at temperkb.io'. That hostname belongs to the
temper-ui project; the Vercel project named temper-cloud builds the root vercel.json
Rust functions and is a different thing again from the TypeScript package of the same
name — the exact conflation that produced a wrong assertion during the outage."
```

---

### Task 2: The canary — preview builds for migration-carrying PRs

Today **every** preview deployment is cancelled in 9–13s by an Ignored Build Step, while every production deployment builds. Neon cuts a preview branch per push for deployments that never build. This task makes migration-carrying PRs build, so the schema change is rehearsed against a real database branch before merge.

**Files:**
- Create: `scripts/vercel-ignore-build.sh` (POSIX sh — runs in Vercel's build container)
- Create: `.github/scripts/test-vercel-ignore-build.sh` (bash guard test)
- Modify: `.github/workflows/code-quality.yml` — one step in the `guard-tests` job
- Modify: `vercel.json` — add `ignoreCommand`

**Interfaces:**
- Consumes: nothing
- Produces: `scripts/vercel-ignore-build.sh`, whose contract is **exit 1 = build, exit 0 = skip** (Vercel's inversion, not ours). Later tasks do not depend on it.

- [ ] **Step 1: Write the failing guard test**

Create `.github/scripts/test-vercel-ignore-build.sh`:

```bash
#!/usr/bin/env bash
# Guard test for scripts/vercel-ignore-build.sh.
#
# Vercel's Ignored Build Step inverts the usual convention: exit 0 SKIPS the build,
# exit 1 BUILDS it. Every assertion below is written in those terms, because getting
# the polarity backwards would silently stop production from deploying — a far worse
# outcome than the cost this script exists to avoid.
set -euo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/scripts/vercel-ignore-build.sh"
fails=0

expect() { # expect <desc> <expected-exit> <env assignments...>
  local desc="$1" want="$2"; shift 2
  local got=0
  env "$@" sh "$SCRIPT" >/dev/null 2>&1 || got=$?
  if [ "$got" -eq "$want" ]; then
    echo "  ok   — $desc (exit $got)"
  else
    echo "  FAIL — $desc: expected exit $want, got $got"; fails=$((fails+1))
  fi
}

echo "vercel-ignore-build guard test"

# Production must ALWAYS build, whatever changed.
expect "production always builds" 1 VERCEL_ENV=production CHANGED_PATHS=""
expect "production builds even with no migration" 1 VERCEL_ENV=production CHANGED_PATHS="README.md"

# Preview builds only when migrations/ moved.
expect "preview WITH a migration builds"    1 VERCEL_ENV=preview CHANGED_PATHS="migrations/20260730000010_x.sql"
expect "preview with a migration among others builds" 1 VERCEL_ENV=preview CHANGED_PATHS=$'README.md\nmigrations/2026_x.sql'
expect "preview WITHOUT a migration skips"  0 VERCEL_ENV=preview CHANGED_PATHS="README.md"
expect "preview with an empty changeset skips" 0 VERCEL_ENV=preview CHANGED_PATHS=""

# A path merely CONTAINING the word must not count — only the migrations/ directory.
expect "docs mentioning migrations do not trigger" 0 VERCEL_ENV=preview CHANGED_PATHS="docs/migrations-guide.md"

# Fail SAFE: an unknown environment builds rather than silently skipping.
expect "unknown VERCEL_ENV builds" 1 VERCEL_ENV= CHANGED_PATHS="README.md"

if [ "$fails" -gt 0 ]; then echo "FAILED: $fails assertion(s)"; exit 1; fi
echo "all assertions passed"
```

- [ ] **Step 2: Run it to verify it fails**

```bash
bash .github/scripts/test-vercel-ignore-build.sh
```

Expected: FAIL — every assertion errors because `scripts/vercel-ignore-build.sh` does not exist.

- [ ] **Step 3: Write the script**

Create `scripts/vercel-ignore-build.sh`:

```sh
#!/bin/sh
# Vercel Ignored Build Step — decides whether this deployment builds.
#
# POLARITY IS INVERTED AND LOAD-BEARING: exit 0 SKIPS the build, exit 1 BUILDS it.
#
# WHY THIS EXISTS
#   Every preview deployment on this project was previously cancelled in 9-13s while
#   production built in 4-8 min. That is a deliberate cost decision — a Rust preview
#   build is expensive — but it meant the Neon preview branch cut for every push was
#   never exercised. A preview runs the PR's binary against its own database branch,
#   which is precisely the pairing rehearsal the schema/binary goal needs, so PRs that
#   touch migrations/ are worth the build and nothing else is.
#
#   Design: docs/superpowers/specs/2026-07-30-schema-binary-pairing-design.md § 4.
#
# CHANGED_PATHS is injected by the guard test; in a real build it is derived from
# VERCEL_GIT_PREVIOUS_SHA, which Vercel sets in the build environment.
set -u

# Production is never skipped. A cost optimisation must not be able to stop a deploy.
if [ "${VERCEL_ENV:-}" = "production" ]; then
  echo "build: VERCEL_ENV=production"
  exit 1
fi

# Fail safe: anything we do not positively recognise as a preview gets built.
if [ "${VERCEL_ENV:-}" != "preview" ]; then
  echo "build: VERCEL_ENV='${VERCEL_ENV:-}' not recognised — building rather than guessing"
  exit 1
fi

if [ -n "${CHANGED_PATHS:-}" ]; then
  changed="${CHANGED_PATHS}"
elif [ -n "${VERCEL_GIT_PREVIOUS_SHA:-}" ]; then
  changed="$(git diff --name-only "${VERCEL_GIT_PREVIOUS_SHA}" HEAD 2>/dev/null || echo "__UNKNOWN__")"
else
  # No previous SHA (first deployment on a branch): we cannot tell, so build.
  echo "build: no VERCEL_GIT_PREVIOUS_SHA — cannot determine the changeset"
  exit 1
fi

if [ "${changed}" = "__UNKNOWN__" ]; then
  echo "build: could not diff against VERCEL_GIT_PREVIOUS_SHA"
  exit 1
fi

# Only the migrations/ directory counts — not a doc that merely says "migrations".
if printf '%s\n' "${changed}" | grep -q '^migrations/'; then
  echo "build: this changeset touches migrations/ — rehearsing the schema change"
  exit 1
fi

echo "skip: preview with no migration change"
exit 0
```

- [ ] **Step 4: Run the guard test to verify it passes**

```bash
chmod +x scripts/vercel-ignore-build.sh
bash .github/scripts/test-vercel-ignore-build.sh
```

Expected: `all assertions passed` — 8 `ok` lines, exit 0.

- [ ] **Step 5: Wire the guard test into CI**

In `.github/workflows/code-quality.yml`, in the `guard-tests` job, after the
`Guard test — audit-grant-sinks (SQL half)` step, add:

```yaml
      - name: Guard test — vercel-ignore-build (polarity + migration scoping)
        run: bash .github/scripts/test-vercel-ignore-build.sh
```

- [ ] **Step 6: Point vercel.json at the script**

In `vercel.json`, add `ignoreCommand` immediately after `installCommand`:

```json
  "ignoreCommand": "sh scripts/vercel-ignore-build.sh",
```

This **overrides** the dashboard Ignored Build Step for every deployment of this repo,
which is the point: which PRs get a preview build becomes a versioned, reviewable
decision rather than an invisible toggle.

- [ ] **Step 7: Verify the whole thing, then commit**

```bash
bash .github/scripts/test-vercel-ignore-build.sh
python3 -c "import json; c=json.load(open('vercel.json')); print('ignoreCommand:', c['ignoreCommand'])"
cargo make check
git add scripts/vercel-ignore-build.sh .github/scripts/test-vercel-ignore-build.sh \
        .github/workflows/code-quality.yml vercel.json
git commit -m "feat(deploy): build previews for migration-carrying PRs — the canary

Every preview deployment on this project was cancelled in 9-13s by an Ignored Build
Step while production built in 4-8 min, so the Neon preview branch cut for every push
was never exercised. A preview runs the PR's binary against its own database branch,
which is exactly the pairing rehearsal the schema/binary goal needs.

ignoreCommand in vercel.json overrides the dashboard setting, so the decision is
versioned and reviewable. Production always builds — a cost optimisation must never be
able to stop a deploy — and anything not positively recognised as a preview builds too.

The polarity is inverted and load-bearing (exit 0 skips, exit 1 builds), so the guard
test asserts it in both directions, including that a doc merely mentioning migrations
does not trigger a build."
```

**After this lands**, the first PR touching `migrations/` should produce a preview
deployment that reaches `● Ready` rather than `Canceled`. Confirm with
`vercel ls temper-cloud` — that is the canary's first chirp, and until it is observed
this task's benefit is claimed rather than demonstrated.

---

### Task 3: The running commit becomes answerable

`/api/health` reports `version: env!("CARGO_PKG_VERSION")` — `0.1.0`, unchanged since the crate was created, carrying zero deploy identity, behind a comment claiming it "can never drift from the crate's actual version" (true, and precisely why it is useless). `VERCEL_GIT_COMMIT_SHA` is present in the build environment; this task bakes it in.

**Files:**
- Create: `crates/temper-api/build.rs`
- Modify: `crates/temper-api/Cargo.toml` (declare the build script)
- Modify: `crates/temper-core/src/types/api.rs:12-15` (`HealthResponse`)
- Modify: `crates/temper-api/src/handlers/health.rs:15-22`
- Regenerate: `openapi.json`, the temper-rb gem, temper-ts's `schema.ts`

**Interfaces:**
- Consumes: nothing from Tasks 1–2
- Produces: `HealthResponse { status: &'static str, version: &'static str, commit: Option<&'static str> }`, and the compile-time env `TEMPER_BUILD_COMMIT` emitted by `crates/temper-api/build.rs`.

> **`HealthResponse` derives `utoipa::ToSchema` (under the `web-api` feature) but has NO
> ts-rs derive.** So this change restales `openapi.json` — and thence the temper-rb gem and
> temper-ts's `schema.ts` — but does **not** touch the ts-rs tree. `cargo make check` gates
> `openapi-check`, `openapi-rb-drift` and `openapi-ts-drift`. Read the `generated-artifacts`
> skill before Step 5.

- [ ] **Step 1: Write the failing test**

Add to `crates/temper-api/src/handlers/health.rs`, at the end of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_reports_a_commit_slot_that_never_lies() {
        let Json(body) = health_check().await.expect("health check succeeds");

        assert_eq!(body.status, "ok");

        // The commit is Option, not a placeholder string. A build that cannot know its
        // commit — a local `cargo build`, a `cargo install` — must report *absence*,
        // never a plausible-looking value. `unverifiable` is not `wrong`.
        match body.commit {
            None => {}
            Some(sha) => {
                assert!(!sha.is_empty(), "a recorded commit must not be empty");
                assert!(
                    sha.chars().all(|c| c.is_ascii_hexdigit()),
                    "a recorded commit must be a hex sha, got {sha:?}"
                );
            }
        }
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cargo nextest run -p temper-api --lib health_reports_a_commit_slot_that_never_lies
```

Expected: FAIL to compile — `no field 'commit' on type 'HealthResponse'`.

- [ ] **Step 3: Add the field to the shared type**

In `crates/temper-core/src/types/api.rs`, replace the `HealthResponse` struct
(currently lines 12-15) with:

```rust
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    /// The git commit this binary was built from, when the build knew it.
    ///
    /// `None` means *this build did not record one* — a local `cargo build`, a
    /// `cargo install`, any build outside a Vercel deploy. It never means "unknown
    /// commit": absence is reported as absence rather than as a placeholder, so a
    /// reader can never mistake "we cannot tell" for "we checked".
    pub commit: Option<&'static str>,
}
```

- [ ] **Step 4: Write the build script**

Create `crates/temper-api/build.rs`, modelled on `crates/temper-ingest/build.rs:42`
which already bakes a compile-time value this way:

```rust
//! Bakes the git commit this binary was built from into the binary itself.
//!
//! Vercel sets `VERCEL_GIT_COMMIT_SHA` in the build environment (verified against a real
//! build log, 2026-07-30). Outside a Vercel deploy the variable is absent, and that case
//! is reported as absence — never as a placeholder — so `/api/health` can distinguish
//! "this build did not record a commit" from "this build is at commit X".
//!
//! Design: docs/superpowers/specs/2026-07-30-schema-binary-pairing-design.md § 5.

fn main() {
    // Rebuild when the variable appears, changes, or disappears.
    println!("cargo:rerun-if-env-changed=VERCEL_GIT_COMMIT_SHA");

    match std::env::var("VERCEL_GIT_COMMIT_SHA") {
        Ok(sha) if !sha.trim().is_empty() => {
            println!("cargo:rustc-env=TEMPER_BUILD_COMMIT={}", sha.trim());
        }
        _ => {
            // Emit nothing. `option_env!` then resolves to `None`, which is the honest
            // answer; emitting a sentinel string here would make absence indistinguishable
            // from a commit literally named that.
        }
    }
}
```

- [ ] **Step 5: Declare the build script explicitly**

In `crates/temper-api/Cargo.toml`, in the `[package]` section (it currently has no `build`
key, and two `[[bin]]` targets: `temper-api` and `emit-openapi`), add:

```toml
build = "build.rs"
```

Cargo auto-detects a `build.rs` in the package root, so this line is **not required** — it
is here so the build script is visible to someone reading the manifest rather than only to
someone listing the directory. Do not treat its absence as a bug if you see it elsewhere.

- [ ] **Step 6: Read the baked value in the handler**

In `crates/temper-api/src/handlers/health.rs`, replace the body of `health_check`
(currently lines 15-22) with:

```rust
pub async fn health_check() -> ApiResult<Json<HealthResponse>> {
    Ok(Json(HealthResponse {
        status: "ok",
        // Sourced from Cargo at compile time. NOTE: this is the temper-api crate's own
        // version (0.1.0, unchanged since the crate was created), not a deploy identity —
        // `commit` below is the field that answers "what is running here".
        version: env!("CARGO_PKG_VERSION"),
        // `option_env!` resolves at compile time to the value build.rs emitted, or `None`
        // when the build did not know its commit.
        commit: option_env!("TEMPER_BUILD_COMMIT"),
    }))
}
```

- [ ] **Step 7: Run the test to verify it passes**

```bash
cargo nextest run -p temper-api --lib health_reports_a_commit_slot_that_never_lies
```

Expected: PASS. Locally `commit` is `None` (no `VERCEL_GIT_COMMIT_SHA`), which the test
accepts as the honest answer.

- [ ] **Step 8: Prove the baking actually works**

A test that passes with `None` would pass just as well if the plumbing were dead. Force
the variable and confirm the value lands.

> The endpoint is `/api/health` and the port defaults to `3000`. Don't go looking for
> either in `routes.rs` — the route is registered as `routes!(handlers::health::health_check)`
> and the **path comes from the `#[utoipa::path(get, path = "/api/health", …)]` attribute**
> on the handler; the port is `lookup("PORT") … unwrap_or(3000)` in the services config.

```bash
VERCEL_GIT_COMMIT_SHA=$(git rev-parse HEAD) cargo run -p temper-api --bin temper-api 2>/dev/null &
sleep 8 && curl -s localhost:3000/api/health | python3 -m json.tool ; kill %1
```

Expected: `"commit": "<the sha you passed>"`. If it is `null`, the build script did not
re-run — `touch crates/temper-api/build.rs` and retry.

- [ ] **Step 9: Regenerate the router-derived artifacts**

```bash
cargo make openapi
git status --porcelain openapi.json clients/temper-rb clients/temper-ts
```

Expected: `openapi.json` gains the `commit` property on the `HealthResponse` schema, and
the gem plus `schema.ts` regenerate to match. All three are committed artifacts.

- [ ] **Step 10: Verify the gates, then commit**

```bash
cargo make check
git add crates/temper-api/build.rs crates/temper-api/Cargo.toml \
        crates/temper-core/src/types/api.rs crates/temper-api/src/handlers/health.rs \
        openapi.json clients/temper-rb clients/temper-ts
git commit -m "feat(api): /api/health reports the commit the binary was built from

The endpoint reported version: env!(\"CARGO_PKG_VERSION\") — 0.1.0, unchanged since the
crate was created — behind a comment saying it can never drift from the crate's actual
version. True, and precisely why it carried zero deploy identity: a merge is not a
deploy, and nothing could answer what was actually running.

VERCEL_GIT_COMMIT_SHA is present in the Vercel build environment (verified against a
real build log). build.rs bakes it via cargo:rustc-env, following the pattern
crates/temper-ingest/build.rs already uses for the embedding model's sha256.

commit is Option and absence is reported as absence. A build that cannot know its
commit — a local cargo build, a cargo install — reports null, never a placeholder, so
'we cannot tell' can never be read as 'we checked'.

HealthResponse is in the OpenAPI contract, so openapi.json and its two generated
consumers (the temper-rb gem, temper-ts's schema.ts) are regenerated here."
```

---

## Self-Review

**Spec coverage.** Spec step 1 → Task 1. Step 2 → Task 2. Step 5 → Task 3. Steps 3 and 4
are explicitly out of scope, with reasons, under *Scope* above. No spec requirement inside
this plan's scope lacks a task.

**Placeholder scan.** No `TBD`, `TODO`, "add error handling", or "similar to Task N". Every
code step carries the actual content. Every command carries its expected output.

**Type consistency.** `HealthResponse` is defined once (Task 3 Step 3) with three fields and
referenced with exactly those names in Steps 1, 6 and 9. The compile-time env is
`TEMPER_BUILD_COMMIT` in both `build.rs` (Step 4, emitting) and `health.rs` (Step 6,
reading). The ignore script's contract — exit 1 builds, exit 0 skips — is stated identically
in the test (Task 2 Step 1), the script (Step 3) and the interfaces block.

**One thing deliberately asymmetric.** Task 2's benefit is not demonstrated by its own tests
— a guard test proves the polarity logic, not that Vercel honours it. The task therefore
ends with an observation step against `vercel ls`, and says outright that until that is seen
the benefit is claimed rather than demonstrated.
