# temper-ts Drift Regime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Put `clients/temper-ts` inside the contract-drift regime — a committed, generated `schema.ts` under gates that go red when `openapi.json` changes and the schema doesn't — and give it a minimal typed client so the schema is usable.

**Architecture:** `openapi-typescript` emits one types-only `src/generated/schema.ts` from the repo-root `openapi.json` (68 paths, 161 schemas → ~7,400 lines, ~71ms, Node-only). A generator script is the single source of truth for the invocation and is called from three places that must agree (`cargo make openapi`, the drift check, CI). `openapi-fetch`'s `createClient<paths>` then makes all 68 paths callable with full types, so the only hand-written code is what the contract cannot carry: bearer auth, the `X-Temper-Surface` attribution header, and one re-mint on 401.

**Tech Stack:** TypeScript 5.8 (strict, NodeNext), Node ≥22, vitest 3, `openapi-typescript` 7.13.0 (devDep, exact pin), `openapi-fetch` 0.17.x (the package's first runtime dependency), cargo-make, bash, GitHub Actions.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-07-13-temper-ts-drift-regime-design.md`. Read it before Task 1.
- **`clients/temper-ts` is workspace-isolated.** It is NOT a bun workspace member and NOT a cargo member. Run all npm commands **from inside `clients/temper-ts`** (`cd clients/temper-ts && npm ci`). A root `npm install` inherits the root's bun `overrides` and fails.
- **`openapi-typescript` is pinned EXACTLY** — `"openapi-typescript": "7.13.0"`, no caret. A moving generator makes the drift gate fail on days when nothing changed. (Verified: two runs of 7.13.0 against the real spec are byte-identical, and the emitted header carries no timestamp or version.)
- **Never hand-edit `src/generated/schema.ts`.** It is a product of `openapi.json`, which is a product of the Axum router.
- **The generated file is committed.** That is the whole point — a drift gate needs something to diff against.
- **No `console.log`** in library code.
- Existing tests must stay green: 29 temper-ts tests, 15 steward tests.

---

## File Structure

**Create:**
- `.github/scripts/generate-temper-ts.sh` — the single source of truth for the generator invocation
- `.github/scripts/check-temper-ts-drift.sh` — regenerate + `git diff --exit-code`
- `clients/temper-ts/src/generated/schema.ts` — the emission (generated, committed, never hand-edited)
- `clients/temper-ts/src/auth-fetch.ts` — `createAuthedFetch`: bearer + surface header + one re-mint on 401
- `clients/temper-ts/src/client.ts` — `createTemperClient`: `openapi-fetch` over `paths`
- `clients/temper-ts/tests/auth-fetch.test.ts`
- `clients/temper-ts/tests/client.test.ts`

**Modify:**
- `.github/scripts/detect-ci-scope.sh:155` — add `^openapi\.json$` to the `test-agents-ts` trigger set
- `.github/scripts/test-detect-ci-scope.sh` — new case proving it
- `.github/workflows/test-agents-ts.yml` — a drift step in the `temper-ts` job
- `tools/cargo-make/main.toml` — `openapi-ts` + `openapi-ts-drift` tasks; both wired into `openapi` and `check`
- `clients/temper-ts/package.json` — deps, `./schema` export, `generate` + `drift` scripts
- `clients/temper-ts/src/index.ts` — re-export the schema types and the client

---

## Task 1: The CI trigger set (the gate that would otherwise run nowhere)

This comes **first**. A drift gate that CI never runs is not a gate, and `test-agents-ts` is path-scoped to a set that does not include `openapi.json` — so a Rust DTO change would regenerate the spec and never run the job. Same rot as `tests/contracts/` missing from `test-ruby`'s set. Prove it failing, then fix it.

**Files:**
- Modify: `.github/scripts/test-detect-ci-scope.sh` (append a case, before the summary block at the end)
- Modify: `.github/scripts/detect-ci-scope.sh:146-156`

**Interfaces:**
- Consumes: nothing.
- Produces: `RUN_TEST_AGENTS_TS=true` for an `openapi.json`-only change. Task 3's CI drift step depends on this being true, or it never executes.

- [ ] **Step 1: Write the failing test case**

Append to `.github/scripts/test-detect-ci-scope.sh`, immediately before the final summary/exit block (find it by locating the `echo "Running detect-ci-scope.sh tests..."` header's matching tail — the last `run_test` call in the file; put this after it):

```bash
# --- openapi.json alone must run BOTH SDK jobs: each has a codegen drift gate
# --- against it, and a gate the contract change does not run is not a gate.
run_test "openapi.json change: runs both SDK drift gates" \
    "openapi.json" \
    "DOCS_ONLY=false" \
    "RUN_TEST_RUBY=true" \
    "RUN_TEST_AGENTS_TS=true"
```

- [ ] **Step 2: Run it and watch it fail**

```bash
bash .github/scripts/test-detect-ci-scope.sh
```

Expected: `FAIL: openapi.json change: runs both SDK drift gates` with
`RUN_TEST_AGENTS_TS: expected='true' actual='false'`. `RUN_TEST_RUBY` already passes — that asymmetry *is* the bug.

Do not proceed until you have seen this fail. A test that passes before the fix is testing nothing.

- [ ] **Step 3: Fix the trigger regex**

In `.github/scripts/detect-ci-scope.sh`, the `HAS_AGENTS_TS` block. Change:

```bash
if changes_match '^clients/temper-ts/|^packages/agent-workflows/|^tests/contracts/|^\.github/workflows/test-agents-ts\.yml$|^__force_full_ci__$'; then
```

to:

```bash
if changes_match '^clients/temper-ts/|^packages/agent-workflows/|^tests/contracts/|^openapi\.json$|^\.github/workflows/test-agents-ts\.yml$|^__force_full_ci__$'; then
```

And update the block's comment — it currently claims the contract is in the trigger set, which was only half true (`tests/contracts/` was; `openapi.json` was not). Replace the comment above `HAS_AGENTS_TS=false` with:

```bash
# TypeScript SDK + agent workflows: clients/temper-ts (the TS client) and
# packages/agent-workflows/** (the eve agents that consume it), plus BOTH wire
# contracts they are asserted against.
#
# openapi.json is in this set for the same reason it is in test-ruby's: temper-ts
# commits a generated schema.ts, so a contract change that does not run this job is
# a contract change whose drift gate never fires. tests/contracts/ is the other
# contract (the m2m token request).
#
# crates/** deliberately stays OUT: openapi.json is committed, and openapi-check in
# code-quality already forces a DTO change to land a regenerated spec in the same
# PR. The contract is therefore both sufficient and precise as the trigger key.
#
# Path-scoped for exactly the reason test-ruby is: these projects are inert to
# both cargo (`members = ["crates/*", "tests/e2e"]`) and bun (an explicit
# two-entry `workspaces` list), so no Rust or TS change can reach them except
# through a contract, which is in the trigger set.
```

- [ ] **Step 4: Run the scope tests and watch them all pass**

```bash
bash .github/scripts/test-detect-ci-scope.sh
```

Expected: every case passes, including the new one, and the count is one higher than before (27, if it was 26).

- [ ] **Step 5: Commit**

```bash
git add .github/scripts/detect-ci-scope.sh .github/scripts/test-detect-ci-scope.sh
git commit -m "ci(scope): openapi.json must run test-agents-ts, or the drift gate runs nowhere

test-agents-ts is path-scoped and openapi.json was not in the set. Correct while
temper-ts is inert to the contract; wrong the instant it commits a generated
schema. The block's own comment already claimed 'a contract, which is in the
trigger set' — but only tests/contracts/ was. test-ruby has carried
^openapi.json\$ all along, for exactly this reason.

Proven by a scope-test case that fails before the regex change."
```

---

## Task 2: The generated schema

**Files:**
- Modify: `clients/temper-ts/package.json`
- Create: `.github/scripts/generate-temper-ts.sh`
- Create: `clients/temper-ts/src/generated/schema.ts` (by running the script — never by hand)
- Modify: `clients/temper-ts/src/index.ts`

**Interfaces:**
- Consumes: nothing.
- Produces: `src/generated/schema.ts` exporting `paths`, `components`, `operations`. Task 4 and Task 5 both import `paths` from it. `package.json` gains a `"./schema"` export and a `generate` script.

- [ ] **Step 1: Add the pinned generator and the schema export to `package.json`**

`clients/temper-ts/package.json` — add the `"./schema"` entry to `exports`, the `generate` script, and the exact-pinned devDependency. The exact pin (no caret) is load-bearing: a moving generator makes the drift gate fail on days when nothing changed.

```json
{
  "exports": {
    ".": {
      "types": "./dist/index.d.ts",
      "default": "./dist/index.js"
    },
    "./schema": {
      "types": "./dist/generated/schema.d.ts",
      "default": "./dist/generated/schema.js"
    },
    "./testing": {
      "types": "./dist/testing/index.d.ts",
      "default": "./dist/testing/index.js"
    }
  },
  "scripts": {
    "build": "tsc",
    "generate": "bash ../../.github/scripts/generate-temper-ts.sh",
    "test": "vitest run",
    "typecheck": "tsc --noEmit && tsc -p tsconfig.test.json"
  },
  "devDependencies": {
    "@types/node": "24.x",
    "openapi-typescript": "7.13.0",
    "typescript": "^5.8",
    "vitest": "^3"
  }
}
```

Keep every other field (`name`, `version`, `private`, `description`, `type`, `main`, `types`, `files`, `engines`) exactly as it is. Then install, from inside the package:

```bash
cd clients/temper-ts && npm install
```

- [ ] **Step 2: Write the generator script**

Create `.github/scripts/generate-temper-ts.sh`:

```bash
#!/usr/bin/env bash
#
# Regenerate clients/temper-ts/src/generated/schema.ts from the repo-root openapi.json.
#
# The generated schema is a committed *product of openapi.json* (itself a product of
# the Axum router), so a new field on a response DTO leaves temper-ts stale — the
# same class of drift the openapi-check gate guards for the spec itself, and the same
# one generate-temper-rb.sh guards for the gem.
#
# This script is the single source of truth for the generator invocation. Called from
# three places that must agree, or the drift gate would be checking a different
# artifact than the one it tells you to regenerate:
#   - `cargo make openapi` / `cargo make openapi-ts` (local dev, regen)
#   - `cargo make openapi-ts-drift` → check-temper-ts-drift.sh (local dev, verify)
#   - the temper-ts CI job's drift step (.github/workflows/test-agents-ts.yml)
#
# Unlike the gem's generator this needs neither Docker nor Java — openapi-typescript
# is an npm devDependency, so a Rust dev who changed a DTO regenerates with Node
# alone (~70ms). That is why, unlike openapi-rb-drift, the TS drift gate NEVER skips.
#
# The generator version is pinned EXACTLY in clients/temper-ts/package.json (no
# caret) and locked in package-lock.json: a moving generator makes the drift gate
# fail on days when nothing in this repo changed.
#
# Usage: bash .github/scripts/generate-temper-ts.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SPEC="$REPO_ROOT/openapi.json"
PKG="$REPO_ROOT/clients/temper-ts"
OUT="$PKG/src/generated/schema.ts"

if [ ! -s "$SPEC" ]; then
  echo "ERROR: openapi.json is missing or empty — run: cargo make openapi" >&2
  exit 1
fi

# npm ci (not install) so the LOCKED generator version is what emits — but only when
# the binary is absent, since a wholesale reinstall on every regen would make
# `cargo make check` pay seconds for nothing. temper-ts is workspace-isolated: npm
# MUST run from inside it (a root install inherits the root's bun overrides and fails).
if [ ! -x "$PKG/node_modules/.bin/openapi-typescript" ]; then
  echo "  installing temper-ts devDependencies (pinned openapi-typescript)…" >&2
  (cd "$PKG" && npm ci)
fi

mkdir -p "$(dirname "$OUT")"
(cd "$PKG" && ./node_modules/.bin/openapi-typescript "$SPEC" -o "$OUT")
```

Make it executable:

```bash
chmod +x .github/scripts/generate-temper-ts.sh
```

- [ ] **Step 3: Generate the schema and eyeball it**

```bash
bash .github/scripts/generate-temper-ts.sh
wc -l clients/temper-ts/src/generated/schema.ts
grep -c "" clients/temper-ts/src/generated/schema.ts
head -6 clients/temper-ts/src/generated/schema.ts
grep -n "^export interface paths\|^export interface components\|^export interface operations" clients/temper-ts/src/generated/schema.ts
```

Expected: ~7,400 lines; the header comment `This file was auto-generated by openapi-typescript.` with **no** timestamp or version line (that is what makes the drift gate stable); and all three of `paths`, `components`, `operations` exported.

- [ ] **Step 4: Prove it is deterministic**

Run the generator twice and confirm the working tree is clean the second time — this is the property the entire drift gate rests on:

```bash
bash .github/scripts/generate-temper-ts.sh
git status --short clients/temper-ts/src/generated/schema.ts
```

Expected after the first commit of the file: **no output** (byte-identical re-emission). If this prints a modified file, stop — the gate would flap, and nothing downstream is worth building until it doesn't.

- [ ] **Step 5: Export the schema publicly from `index.ts`**

The schema is a public export from day one — temper-ui's eventual migration off its 103 overlapping ts-rs types is the stated exit, and an export it cannot reach is not a direction. Add to `clients/temper-ts/src/index.ts`, below the existing `credentials.js` export block:

```typescript
/**
 * The wire contract, generated from the repo-root `openapi.json` — itself a product of the Axum
 * router. NEVER hand-edited: `cargo make openapi` regenerates it, and `openapi-ts-drift` (in
 * `cargo make check`, and in CI) fails if the committed copy has fallen behind the contract.
 *
 * Public from day one on purpose. temper-ui types its API surface from ts-rs today, and 103 of
 * those 133 types are ALSO OpenAPI schemas — two TypeScript renderings of the same Rust structs.
 * The exit is temper-ui importing these instead; an export it cannot reach would not be a
 * direction, only an intention.
 */
export type { components, operations, paths } from "./generated/schema.js";
```

- [ ] **Step 6: Typecheck, build, and confirm the schema reaches `dist/`**

```bash
cd clients/temper-ts && npm run typecheck && npm run build && ls dist/generated/
```

Expected: typecheck clean under `strict`; `dist/generated/` contains **both** `schema.js` and `schema.d.ts`. (The emission is `.ts`, not `.d.ts`, precisely so `tsc` carries it into `dist` — a `.d.ts` under `src/` would not be emitted, and the `"./schema"` export would dangle.)

- [ ] **Step 7: Run the existing tests**

```bash
cd clients/temper-ts && npm test
```

Expected: 29 passed. The schema is types-only; nothing at runtime should have moved.

- [ ] **Step 8: Commit**

```bash
git add .github/scripts/generate-temper-ts.sh clients/temper-ts/package.json \
        clients/temper-ts/package-lock.json clients/temper-ts/src/generated/schema.ts \
        clients/temper-ts/src/index.ts
git commit -m "feat(temper-ts): generate the wire contract — schema.ts from openapi.json

openapi-typescript, not openapi-generator: the gem generates an HTTP layer because
Ruby has none worth having, and TypeScript has fetch. One types-only file (7.4k
lines, 71ms, no Docker, no Java) instead of ~170 files of ToJSON boilerplate that
would rot into a second, worse client next to credentials.ts.

Public export from day one — temper-ui's migration off its 103 overlapping ts-rs
types is the stated exit, and an export it cannot reach is not a direction."
```

---

## Task 3: The gates

**Files:**
- Create: `.github/scripts/check-temper-ts-drift.sh`
- Modify: `tools/cargo-make/main.toml` (the `check` chain at :26-34; the `openapi` task at :229; new tasks after `openapi-rb-drift` at :253)
- Modify: `.github/workflows/test-agents-ts.yml` (the `temper-ts` job)

**Interfaces:**
- Consumes: `.github/scripts/generate-temper-ts.sh` (Task 2); `RUN_TEST_AGENTS_TS=true` on an `openapi.json` change (Task 1).
- Produces: `cargo make openapi-ts`, `cargo make openapi-ts-drift`; a CI drift step.

- [ ] **Step 1: Write the drift check script**

Create `.github/scripts/check-temper-ts-drift.sh`:

```bash
#!/usr/bin/env bash
#
# Fail if the committed temper-ts schema drifts from openapi.json.
#
# Regenerates the schema (via generate-temper-ts.sh) and diffs the result against what
# is committed — the local mirror of the temper-ts CI job's drift step.
#
# Unlike check-temper-rb-drift.sh this NEVER skips. That one needs a Docker daemon and
# exits 0 when it is absent (the test-ruby CI job being the never-skipping backstop);
# openapi-typescript needs only Node, so there is no environment in which we would
# rather guess. `cargo make check` therefore gains a gate that is a real gate.
#
# Usage: bash .github/scripts/check-temper-ts-drift.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GENERATED="clients/temper-ts/src/generated/schema.ts"

bash "$REPO_ROOT/.github/scripts/generate-temper-ts.sh"

if ! git -C "$REPO_ROOT" diff --exit-code -- "$GENERATED"; then
  echo >&2
  echo "ERROR: temper-ts's generated schema is out of date with openapi.json." >&2
  echo "       Run: cargo make openapi   (regenerates the spec, the gem, and the schema)" >&2
  echo "       then commit the regenerated $GENERATED" >&2
  exit 1
fi

echo "temper-ts generated schema is up to date with openapi.json"
```

```bash
chmod +x .github/scripts/check-temper-ts-drift.sh
```

Note the same trap that bites the gem gate: the diff is **against git**, not against a fresh build. A schema you have just correctly regenerated still fails while it sits unstaged. Stage it, then re-run.

- [ ] **Step 2: Add the cargo-make tasks**

In `tools/cargo-make/main.toml`, immediately after the `[tasks.openapi-rb-drift]` block, add:

```toml
[tasks.openapi-ts]
description = "Regenerate temper-ts's schema.ts from openapi.json (needs Node)"
script = ["bash ${CARGO_MAKE_WORKING_DIRECTORY}/.github/scripts/generate-temper-ts.sh"]

[tasks.openapi-ts-drift]
description = "Fail if the committed temper-ts schema drifts from openapi.json"
# The local mirror of the temper-ts CI job's drift step, and the sibling of
# openapi-rb-drift. Unlike that one it NEVER skips: openapi-typescript needs only
# Node, so there is no environment in which we would rather guess than check.
script = ["bash ${CARGO_MAKE_WORKING_DIRECTORY}/.github/scripts/check-temper-ts-drift.sh"]
```

- [ ] **Step 3: Wire both gates into `check` and `openapi`**

In `[tasks.check]`, add `"openapi-ts-drift"` to `dependencies`, directly after `"openapi-rb-drift"`:

```toml
dependencies = [
  "rust-fmt-check",
  "rust-clippy",
  "rust-docs",
  "rust-machete",
  "openapi-check",
  "openapi-routes-check",
  "openapi-rb-drift",
  "openapi-ts-drift",
  "ts-check"
]
```

And extend the comment above it — after the sentence ending `(the test-ruby CI job is the never-skipping backstop).` — with:

```
# `openapi-ts-drift` does the same for temper-ts's generated schema.ts, and unlike the
# gem's gate it never skips: openapi-typescript needs only Node.
```

In `[tasks.openapi]`, append the schema regen as a final script line, after the temper-rb line:

```toml
  "bash ${CARGO_MAKE_WORKING_DIRECTORY}/.github/scripts/generate-temper-ts.sh && echo 'temper-ts schema regenerated'"
```

and update that task's `description` to:

```toml
description = "Emit the OpenAPI contract + regenerate the temper-rb gem and temper-ts schema (all products of the router)"
```

- [ ] **Step 4: Prove the gate BITES**

A gate that has never gone red is a gate you are trusting on faith. Falsify it — the probe must violate the invariant the gate asserts:

```bash
cp openapi.json /tmp/openapi.json.bak
python3 - <<'PY'
import json
spec = json.load(open('openapi.json'))
spec['components']['schemas']['ResourceRow']['properties']['drift_probe'] = {'type': 'string'}
json.dump(spec, open('openapi.json', 'w'), indent=2)
PY
cargo make openapi-ts-drift; echo "exit=$?"
```

Expected: a diff showing `drift_probe?: string;` landing in `schema.ts`, then
`ERROR: temper-ts's generated schema is out of date with openapi.json.` and a **non-zero exit**.

Now restore — **from the file copy, never `git checkout`** (that would also revert your in-progress work):

```bash
cp /tmp/openapi.json.bak openapi.json
bash .github/scripts/generate-temper-ts.sh
git status --short openapi.json clients/temper-ts/src/generated/schema.ts
cargo make openapi-ts-drift; echo "exit=$?"
```

Expected: `git status` clean for both files, and the gate now exits **0**.

- [ ] **Step 5: Add the CI drift step**

In `.github/workflows/test-agents-ts.yml`, in the `temper-ts` job, add a step between "Install dependencies" and "Type-check". The job's `defaults.run.working-directory` is `clients/temper-ts`, and the script wants a repo-root-relative path:

```yaml
      # The never-skipping backstop for the generated schema — the role `test-ruby`'s
      # `rake drift` plays for the gem. `detect-ci-scope.sh` runs this job for an
      # openapi.json change precisely so this step fires on the PR that would break it.
      - name: Schema drift (schema.ts vs openapi.json)
        run: bash ../../.github/scripts/check-temper-ts-drift.sh
```

- [ ] **Step 6: Run the full local gate**

```bash
cargo make check
```

Expected: passes, and the output now includes `temper-ts generated schema is up to date with openapi.json`.

- [ ] **Step 7: Commit**

```bash
git add .github/scripts/check-temper-ts-drift.sh tools/cargo-make/main.toml \
        .github/workflows/test-agents-ts.yml
git commit -m "feat(temper-ts): gate the schema — cargo make check + a CI backstop

openapi-ts-drift joins the check chain beside openapi-rb-drift, and unlike that one
it never skips: openapi-typescript needs only Node, so there is no environment in
which we would rather guess than check.

Verified by falsification: adding drift_probe to ResourceRow in openapi.json turns
the gate red; removing it turns it green."
```

---

## Task 4: `createAuthedFetch` — what the contract cannot carry

The generated schema gives every path and type. What it cannot give is who you are. This is the steward's `temperFetch` (`packages/agent-workflows/steward/agent/lib/temper-auth.ts:103`) lifted out of the steward, minus what is genuinely eve's: the env names, the Vercel Connect strategy, and the 5xx cold-start retry.

**Files:**
- Create: `clients/temper-ts/src/auth-fetch.ts`
- Create: `clients/temper-ts/tests/auth-fetch.test.ts`

**Interfaces:**
- Consumes: `Credentials` (`canRefresh`, `token()`, `refresh()`) from `./credentials.js`; `startMockApi`, `startMockIssuer` from `./testing/index.js`.
- Produces:
  ```typescript
  export type FetchLike = (input: Request) => Promise<Response>;
  export interface AuthedFetchOptions {
    credentials: Credentials;
    surface?: "sdk" | "cli";
    fetch?: FetchLike;
  }
  export function createAuthedFetch(opts: AuthedFetchOptions): FetchLike;
  ```
  Task 5 hands the result to `createClient`. **`FetchLike` takes a `Request`, not `(url, init)`** — that is `openapi-fetch`'s `ClientOptions["fetch"]` signature (`(input: Request) => Promise<Response>`), and matching it is what lets the two compose with no adapter.

- [ ] **Step 1: Write the failing tests**

Create `clients/temper-ts/tests/auth-fetch.test.ts`. Note `startMockApi` returns a full endpoint `url`; a `Request` needs only that, so no harness change is required.

```typescript
import { afterEach, describe, expect, it } from "vitest";

import { createAuthedFetch } from "../src/auth-fetch.js";
import { BearerToken, ClientCredentials } from "../src/credentials.js";
import { type MockApi, type MockIssuer, startMockApi, startMockIssuer } from "../src/testing/index.js";

let api: MockApi | undefined;
let issuer: MockIssuer | undefined;

const CLIENT_ID = "tmpr_test";
const CLIENT_SECRET = "s3cr3t";

afterEach(async () => {
  await api?.close();
  await issuer?.close();
  api = undefined;
  issuer = undefined;
});

/**
 * `startMockIssuer` REQUIRES flavor/clientId/clientSecret — there is no zero-arg form. The
 * `temper-as` flavor is the one that matters here: it mints 900s tokens and ignores a
 * request-supplied audience, exactly as the real AS does.
 */
async function startTemperAs(): Promise<MockIssuer> {
  return startMockIssuer({ flavor: "temper-as", clientId: CLIENT_ID, clientSecret: CLIENT_SECRET });
}

function machineCredentials(issuerUrl: string): ClientCredentials {
  return new ClientCredentials({
    tokenUrl: issuerUrl,
    clientId: CLIENT_ID,
    clientSecret: CLIENT_SECRET,
  });
}

describe("createAuthedFetch", () => {
  it("presents a bearer token and the sdk surface header", async () => {
    api = await startMockApi();
    issuer = await startTemperAs();

    const authed = createAuthedFetch({ credentials: machineCredentials(issuer.url) });
    const res = await authed(new Request(api.url));

    expect(res.status).toBe(200);
    expect(api.bearers).toHaveLength(1);
    expect(api.bearers[0]).not.toBe("");
    expect(api.surfaces).toEqual(["sdk"]);
  });

  it("re-mints once on a 401 and retries with the NEW token", async () => {
    api = await startMockApi({ rejectFirst: 1 });
    issuer = await startTemperAs();

    const authed = createAuthedFetch({ credentials: machineCredentials(issuer.url) });
    const res = await authed(new Request(api.url));

    expect(res.status).toBe(200);
    // Two presentations, and the retry carried a DIFFERENT token — a blind replay of the
    // dead one would 401 forever.
    expect(api.bearers).toHaveLength(2);
    expect(api.bearers[0]).not.toBe(api.bearers[1]);
    // `requests` (NOT `mints`) is what MockIssuer records — one mint for the original token,
    // one for the re-mint.
    expect(issuer.requests).toHaveLength(2);
  });

  it("replays a POST body on the retry", async () => {
    api = await startMockApi({ rejectFirst: 1 });
    issuer = await startTemperAs();

    const authed = createAuthedFetch({ credentials: machineCredentials(issuer.url) });
    const res = await authed(
      new Request(api.url, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ title: "hello" }),
      }),
    );

    expect(res.status).toBe(200);
    // The first send CONSUMES the request body. A retry built from the consumed request
    // would send an empty one — silently writing nothing, on the exact path a 401 recovery
    // exists to save.
    expect(api.bodies).toEqual(['{"title":"hello"}', '{"title":"hello"}']);
  });

  it("returns a 401 UNTOUCHED when the strategy cannot mint", async () => {
    api = await startMockApi({ rejectFirst: 1 });

    const authed = createAuthedFetch({ credentials: new BearerToken("static-token") });
    const res = await authed(new Request(api.url));

    // BearerToken.refresh() THROWS. Calling it here would replace temper's real 401 — the
    // answer a human is trying to read, body and all — with "BearerToken cannot refresh".
    expect(res.status).toBe(401);
    expect(await res.json()).toEqual({ error: "unauthorized" });
    expect(api.bearers).toEqual(["static-token"]);
  });

  it("retries exactly once — a 401 that survives a fresh token is a real denial", async () => {
    api = await startMockApi({ rejectFirst: 99 });
    issuer = await startTemperAs();

    const authed = createAuthedFetch({ credentials: machineCredentials(issuer.url) });
    const res = await authed(new Request(api.url));

    expect(res.status).toBe(401);
    expect(api.bearers).toHaveLength(2); // original + one retry, never a third
  });

  it("composes an inner fetch (the steward keeps its cold-start retry)", async () => {
    api = await startMockApi();
    issuer = await startTemperAs();

    const seen: string[] = [];
    const inner = (input: Request): Promise<Response> => {
      seen.push(input.url);
      return fetch(input);
    };

    const authed = createAuthedFetch({ credentials: machineCredentials(issuer.url), fetch: inner });
    const res = await authed(new Request(api.url));

    expect(res.status).toBe(200);
    expect(seen).toEqual([api.url]);
  });
});
```

These tests need two things `MockApi` does not record yet — `surfaces` and `bodies`. (`MockIssuer` already records every mint attempt, as `requests: MintRequest[]` — no change needed there.) Extend `src/testing/mock-api.ts`:

```typescript
export interface MockApi {
  url: string;
  /** Every bearer token presented, in order. */
  bearers: string[];
  /** Every `X-Temper-Surface` presented, in order — the attribution header. */
  surfaces: string[];
  /** Every request body received, in order. Proves a retry REPLAYED the body it was given. */
  bodies: string[];
  close(): Promise<void>;
}
```

and in `startMockApi`, alongside the existing `bearers.push(...)`, collect the surface header and the body (the handler must now read the request stream before responding):

```typescript
  const surfaces: string[] = [];
  const bodies: string[] = [];

  const server: Server = createServer((req, res) => {
    const header = req.headers.authorization ?? "";
    bearers.push(header.replace(/^Bearer /, ""));
    surfaces.push((req.headers["x-temper-surface"] as string | undefined) ?? "");
    seen += 1;

    const chunks: Buffer[] = [];
    req.on("data", (chunk: Buffer) => chunks.push(chunk));
    req.on("end", () => {
      if (chunks.length > 0) {
        bodies.push(Buffer.concat(chunks).toString("utf8"));
      }

      if (seen <= rejectFirst) {
        res.writeHead(401, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: "unauthorized" }));
        return;
      }

      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ ok: true }));
    });
  });
```

Return `surfaces` and `bodies` from the returned object alongside `bearers`.

- [ ] **Step 2: Run the tests and watch them fail**

```bash
cd clients/temper-ts && npx vitest run tests/auth-fetch.test.ts
```

Expected: FAIL — `Failed to resolve import "../src/auth-fetch.js"`. The module does not exist yet.

- [ ] **Step 3: Implement `createAuthedFetch`**

Create `clients/temper-ts/src/auth-fetch.ts`:

```typescript
import type { Credentials } from "./credentials.js";

/**
 * `openapi-fetch`'s `ClientOptions["fetch"]` — a `Request` in, a `Response` out. NOT the
 * `(url, init)` shape of global `fetch`. Matching it exactly is what lets an authed fetch
 * drop straight into `createClient` with no adapter, and what lets a caller compose one
 * (the steward wraps its 5xx cold-start retry this way).
 */
export type FetchLike = (input: Request) => Promise<Response>;

/**
 * The attribution marker, sent on every request. It names the KIND of surface, never the
 * client's language — the gem sends the same `sdk` (clients/temper-rb/lib/temper/connection.rb).
 * Provenance, never authorization: the server's `{sdk, cli}` allowlist degrades anything else
 * to `web`.
 */
export type Surface = "sdk" | "cli";

export interface AuthedFetchOptions {
  credentials: Credentials;
  /** Default `sdk`. */
  surface?: Surface;
  /** The fetch to wrap. Default: global `fetch`. */
  fetch?: FetchLike;
}

/**
 * `fetch` against temper, authenticated, with a single re-mint on 401.
 *
 * The 401 branch is not belt-and-braces. A caller resolves ONE token and then holds it across N
 * parallel requests, so a token that dies mid-flight takes them all down and nothing recovers;
 * refresh-ahead-of-expiry cannot help, because the token was live when it was checked. Temper's
 * own AS mints 900-second tokens by default, which makes outliving one ordinary rather than
 * exotic. `ClientCredentials.refresh()` coalesces concurrent callers onto ONE mint, so N
 * simultaneous 401s buy one token, not N.
 *
 * Exactly ONE retry: a 401 that survives a fresh token is a real authorization failure — a
 * revoked credential, missing reach — and retrying it would only bury the error.
 *
 * A strategy that cannot mint gets its 401 back UNTOUCHED. `BearerToken.refresh()` throws, and
 * throwing here would replace temper's real answer — the response body a human is trying to
 * read — with a message about the client's own plumbing.
 */
export function createAuthedFetch(opts: AuthedFetchOptions): FetchLike {
  const { credentials } = opts;
  const surface: Surface = opts.surface ?? "sdk";
  const inner: FetchLike = opts.fetch ?? ((input) => fetch(input));

  const authorize = async (request: Request, token: string): Promise<Request> => {
    const headers = new Headers(request.headers);
    headers.set("authorization", `Bearer ${token}`);
    headers.set("x-temper-surface", surface);
    return new Request(request, { headers });
  };

  return async (input: Request): Promise<Response> => {
    // Clone BEFORE the send. Sending consumes the body, and a retry built from the consumed
    // request would carry an empty one — silently writing nothing, on the exact path the 401
    // recovery exists to save.
    const pristine = input.clone();

    const response = await inner(await authorize(input, await credentials.token()));
    if (response.status !== 401 || !credentials.canRefresh) {
      return response;
    }

    const refreshed = await credentials.refresh();
    return inner(await authorize(pristine, refreshed.token));
  };
}
```

- [ ] **Step 4: Run the tests and watch them pass**

```bash
cd clients/temper-ts && npx vitest run tests/auth-fetch.test.ts && npm run typecheck
```

Expected: 6 passed; typecheck clean.

- [ ] **Step 5: Run the whole suite** — the harness changed, so the existing tests must be re-proven

```bash
cd clients/temper-ts && npm test
```

Expected: 35 passed (29 existing + 6 new). If any of the original 29 broke, the `mock-api.ts` body-reading change is the suspect — the handler now responds from inside `req.on("end")`.

- [ ] **Step 6: Commit**

```bash
git add clients/temper-ts/src/auth-fetch.ts clients/temper-ts/tests/auth-fetch.test.ts \
        clients/temper-ts/src/testing/mock-api.ts
git commit -m "feat(temper-ts): createAuthedFetch — bearer, surface, one re-mint on 401

The steward's temperFetch, lifted out of the steward and stripped of what is eve's
(env names, Vercel Connect, the 5xx cold-start retry). It takes an inner fetch, so
the steward keeps that retry by composing rather than by owning a second copy.

The body-replay case is the one that would have shipped broken: sending CONSUMES a
Request's body, so a retry built from the consumed request writes nothing at all —
silently, on the exact path a 401 recovery exists to save. Clone before the send."
```

---

## Task 5: `createTemperClient` — the schema, callable

**Files:**
- Create: `clients/temper-ts/src/client.ts`
- Create: `clients/temper-ts/tests/client.test.ts`
- Modify: `clients/temper-ts/package.json` (the first runtime dependency)
- Modify: `clients/temper-ts/src/index.ts`

**Interfaces:**
- Consumes: `paths` from `./generated/schema.js` (Task 2); `createAuthedFetch`, `Surface` from `./auth-fetch.js` (Task 4).
- Produces:
  ```typescript
  export interface TemperClientOptions {
    baseUrl: string;
    credentials: Credentials;
    surface?: Surface;
    fetch?: FetchLike;
  }
  export function createTemperClient(opts: TemperClientOptions): Client<paths>;
  ```

- [ ] **Step 1: Add the runtime dependency**

temper-ts has had **zero** runtime dependencies. This is the first, and the steward bundles it into a serverless function — so it is stated, not slid in. `openapi-fetch` is ~6kb with no dependencies of its own.

In `clients/temper-ts/package.json`, add a `dependencies` block (there is none today) between `scripts` and `devDependencies`:

```json
  "dependencies": {
    "openapi-fetch": "^0.17.0"
  },
```

```bash
cd clients/temper-ts && npm install
```

- [ ] **Step 2: Write the failing test**

Create `clients/temper-ts/tests/client.test.ts`. `startMockApi` answers `{ok: true}` on any path and its `url` carries a path, so the client's `baseUrl` is that URL's **origin**:

```typescript
import { afterEach, describe, expect, it } from "vitest";

import { createTemperClient } from "../src/client.js";
import { ClientCredentials } from "../src/credentials.js";
import { type MockApi, type MockIssuer, startMockApi, startMockIssuer } from "../src/testing/index.js";

let api: MockApi | undefined;
let issuer: MockIssuer | undefined;

const CLIENT_ID = "tmpr_test";
const CLIENT_SECRET = "s3cr3t";

afterEach(async () => {
  await api?.close();
  await issuer?.close();
  api = undefined;
  issuer = undefined;
});

/** `startMockIssuer` requires flavor/clientId/clientSecret — there is no zero-arg form. */
async function startTemperAs(): Promise<MockIssuer> {
  return startMockIssuer({ flavor: "temper-as", clientId: CLIENT_ID, clientSecret: CLIENT_SECRET });
}

function machineCredentials(issuerUrl: string): ClientCredentials {
  return new ClientCredentials({
    tokenUrl: issuerUrl,
    clientId: CLIENT_ID,
    clientSecret: CLIENT_SECRET,
  });
}

describe("createTemperClient", () => {
  it("calls a contract path with auth and the surface header", async () => {
    api = await startMockApi();
    issuer = await startTemperAs();

    const client = createTemperClient({
      baseUrl: new URL(api.url).origin,
      credentials: machineCredentials(issuer.url),
    });

    // `/api/health` is a real path in the contract — this line does not compile if the
    // generated schema does not carry it, which is the point of generating it.
    const { response } = await client.GET("/api/health");

    expect(response.status).toBe(200);
    expect(api.bearers).toHaveLength(1);
    expect(api.surfaces).toEqual(["sdk"]);
  });

  it("carries the 401 re-mint through to a contract call", async () => {
    api = await startMockApi({ rejectFirst: 1 });
    issuer = await startTemperAs();

    const client = createTemperClient({
      baseUrl: new URL(api.url).origin,
      credentials: machineCredentials(issuer.url),
    });

    const { response } = await client.GET("/api/health");

    expect(response.status).toBe(200);
    expect(api.bearers).toHaveLength(2);
    expect(api.bearers[0]).not.toBe(api.bearers[1]);
  });
});
```

- [ ] **Step 3: Run it and watch it fail**

```bash
cd clients/temper-ts && npx vitest run tests/client.test.ts
```

Expected: FAIL — `Failed to resolve import "../src/client.js"`.

- [ ] **Step 4: Implement the client**

Create `clients/temper-ts/src/client.ts`:

```typescript
import createClient, { type Client } from "openapi-fetch";

import { createAuthedFetch, type FetchLike, type Surface } from "./auth-fetch.js";
import type { Credentials } from "./credentials.js";
import type { paths } from "./generated/schema.js";

export interface TemperClientOptions {
  /** The instance origin — e.g. `https://temperkb.io`. No trailing path. */
  baseUrl: string;
  credentials: Credentials;
  /** Default `sdk`. */
  surface?: Surface;
  /** The fetch to wrap — compose here to keep a caller-specific retry. Default: global `fetch`. */
  fetch?: FetchLike;
}

/**
 * A fully typed client over every path in the contract.
 *
 * There are deliberately NO per-endpoint methods here. The gem hand-writes `Resources`,
 * `Contexts`, `CognitiveMaps` because Ruby has no type inference — someone must write
 * `create(title:, context:)` by hand. TypeScript infers: `createClient<paths>` types every path,
 * its params, and its responses PER STATUS, straight off the generated schema. A hand-written
 * `resources.create()` would be a second, worse spelling of something already correct — and a
 * place for the two to drift.
 *
 * Errors are the contract's too. `openapi-fetch` does not throw; it returns
 * `{ data, error, response }` with `error` typed from the spec's own error responses. The gem
 * needs `errors.rb` because Faraday raises. Inventing a hierarchy alongside a typed one would be
 * inventing drift.
 */
export function createTemperClient(opts: TemperClientOptions): Client<paths> {
  return createClient<paths>({
    baseUrl: opts.baseUrl,
    fetch: createAuthedFetch({
      credentials: opts.credentials,
      surface: opts.surface,
      fetch: opts.fetch,
    }),
  });
}
```

- [ ] **Step 5: Run the tests and watch them pass**

```bash
cd clients/temper-ts && npx vitest run tests/client.test.ts && npm run typecheck
```

Expected: 2 passed; typecheck clean. If `client.GET("/api/health")` fails to typecheck, the schema is not being resolved — check the `paths` import, not the test.

- [ ] **Step 6: Export the client publicly**

Add to `clients/temper-ts/src/index.ts`, below the schema re-export:

```typescript
export { createAuthedFetch, type AuthedFetchOptions, type FetchLike, type Surface } from "./auth-fetch.js";
export { createTemperClient, type TemperClientOptions } from "./client.js";
```

- [ ] **Step 7: Full verification**

```bash
cd clients/temper-ts && npm test && npm run typecheck && npm run build && ls dist/
cd ../../packages/agent-workflows/steward && npm install && npm test
```

Expected: 37 temper-ts tests passing (29 + 6 + 2); build emits `client.js`, `auth-fetch.js`, `generated/schema.js`; the steward's 15 tests still pass — it takes temper-ts as a `file:` dep, so a broken export surfaces here.

Then, from the repo root:

```bash
cargo make check
```

Expected: green, including `temper-ts generated schema is up to date with openapi.json`.

- [ ] **Step 8: Commit**

```bash
git add clients/temper-ts/src/client.ts clients/temper-ts/tests/client.test.ts \
        clients/temper-ts/src/index.ts clients/temper-ts/package.json \
        clients/temper-ts/package-lock.json
git commit -m "feat(temper-ts): createTemperClient — every contract path, typed

openapi-fetch's createClient<paths> over the generated schema. No per-endpoint
methods: the gem hand-writes Resources/Contexts/CognitiveMaps because Ruby cannot
infer, and TypeScript can. No error hierarchy: openapi-fetch returns errors typed
per-status from the spec, and inventing one alongside it would be inventing drift.

First runtime dependency the package has ever had (openapi-fetch, ~6kb, zero deps);
the steward bundles this into a serverless function, so it is stated, not slid in."
```

---

## Task 6: Documentation

**Files:**
- Modify: `CLAUDE.md` (the `clients/` description — check whether one exists; if temper-ts is described only under `packages/agent-workflows`, correct that)
- Modify: `clients/temper-ts/README.md` (create if absent)

- [ ] **Step 1: Document the regime in `CLAUDE.md`**

Find the section describing `temper-rb`'s OpenAPI/drift relationship (search for `openapi-rb-drift`). Extend it so the TypeScript sibling is described in the same breath — a reader must not have to discover that one client is gated and the other is not. State: `schema.ts` is generated from `openapi.json` by `cargo make openapi`; `openapi-ts-drift` gates it and (unlike the gem's) never skips; the CI backstop is `test-agents-ts`; and `detect-ci-scope.sh` carries `openapi.json` in that job's trigger set for exactly that reason.

- [ ] **Step 2: Write `clients/temper-ts/README.md`**

Cover: what the package is (auth + generated contract + minimal client); that `schema.ts` is generated and never hand-edited; how to regenerate (`cargo make openapi`); a `createTemperClient` usage example with `ClientCredentials`; and the temper-ui exit (it will import `temper-ts/schema` and retire its overlapping ts-rs types).

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md clients/temper-ts/README.md
git commit -m "docs(temper-ts): the schema is generated, gated, and never hand-edited"
```

---

## Self-Review Notes

**Spec coverage.** D1 (openapi-typescript) → Task 2. D2 (public export) → Task 2 Step 5. D3 (pin in lockfile, invocation in script) → Task 2 Steps 1–2. D4 (minimal client, no error hierarchy, the stated runtime dep) → Tasks 4–5. D5 (gates + the trigger-set fix) → Tasks 1 and 3. Verification items 1–5 from the spec → Task 1 Step 2 (scope test fails first), Task 3 Step 4 (the gate bites), Task 4 Step 1 (the 401 tests, including the untouched-401 case), Task 2 Step 6 (`schema.ts` typechecks and reaches `dist/`), Task 5 Step 7 (29 + 15 stay green).

**Verified before writing, not assumed.** Every symbol in this plan's code blocks was checked against the real source or a real run:

- `openapi-fetch`'s `fetch` option is `(input: Request) => Promise<Response>` — **not** `(url, init)`. `FetchLike` matches it exactly so the two compose with no adapter.
- `new Request(pristine, { headers })` really does replay a **consumed** POST body on Node 26 (no `duplex` error) — run, not assumed. This is the whole basis of the retry.
- `openapi-typescript@7.13.0` is deterministic: two runs against the real spec are byte-identical, and the emitted header carries no timestamp or version. Without that, the drift gate would flap.
- `/api/health` is a real `get` path in `openapi.json` (so `client.GET("/api/health")` typechecks).
- The gem really does send `X-Temper-Surface: sdk` (`clients/temper-rb/lib/temper/connection.rb:54`), so the TS client's marker matches rather than inventing a second spelling.
- `detect-ci-scope.sh`'s `test-agents-ts` regex omits `openapi.json` while `test-ruby`'s includes it — the asymmetry Task 1 fixes.
- `startMockIssuer(opts)` **requires** `flavor` / `clientId` / `clientSecret` (there is no zero-arg form), and `MockIssuer` records mints as **`requests: MintRequest[]`**, not `mints`. Both were wrong in the first draft of this plan and are corrected above.
