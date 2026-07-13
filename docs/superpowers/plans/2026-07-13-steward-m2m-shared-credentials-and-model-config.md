# Steward M2M Shared Credentials + Config-Driven Model — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Seed `clients/temper-ts` with a credentials module + faithful mock issuer, have the steward compose it (optional audience, re-mint on 401), extend the shared M2M wire contract with a response section, and make the steward's model selection env-driven with an AI-Gateway fallback list.

**Architecture:** The M2M mint moves out of the steward into `clients/temper-ts` — the credentials layer of the TypeScript SDK, mirroring `Temper::Credentials` in the gem (which was itself *ported from* the steward, and fixed two bugs the steward still has). Faithfulness is proven by a mock issuer built from `tests/contracts/m2m-token-request.json` — the same file the **real** AS is asserted against — so a mock that drifts from the AS breaks the AS's own test first. The steward consumes temper-ts through an npm `file:` dependency, a deliberate bridge until temper-ts publishes.

**Tech Stack:** TypeScript (ESM, NodeNext), vitest, `node:http` for the mocks, eve (agent framework), Vercel AI Gateway, npm (isolated projects — **not** bun workspace members).

**Spec:** [docs/superpowers/specs/2026-07-13-steward-m2m-shared-credentials-and-model-config-design.md](../specs/2026-07-13-steward-m2m-shared-credentials-and-model-config-design.md)

## Global Constraints

- **Package name is `temper-ts`** — parity with `temper-rb` (and the coming `temper-py`). A `file:` dep resolves by path but the dependency *key* must match the package's `name`.
- **`clients/temper-ts` and `packages/agent-workflows/steward` are NOT bun workspace members.** The root `package.json` `workspaces` list is exactly `["packages/temper-cloud", "packages/temper-ui"]`. Run all npm commands **from inside** each project directory. A root `npm install` inherits the root's bun `overrides` and fails.
- **`temper-ts` has ZERO runtime dependencies.** It uses only `fetch`, `URLSearchParams`, and `node:http` (the latter in the `./testing` subpath only). Dev deps: `typescript@^5.8`, `vitest@^3`, `@types/node@24.x`.
- **The token request is `application/x-www-form-urlencoded`.** RFC 6749 §4. A JSON body is refused by temper's AS. Never send JSON.
- **`audience` is optional.** Auth0 requires it; temper's AS ignores a request-supplied audience entirely. A `tmpr_` credential omits it. Never send an empty-string audience — omit the key.
- **Token expiry is cached ABSOLUTE (ms since epoch), never relative,** with a **60 000 ms** skew. A duration cannot survive being cached.
- **Never emit `console.log` in temper-ts.** The steward's existing `console.log`/`console.error` in schedules is established and stays.
- **All comments explain WHY, not what.** Match the density and voice of the existing steward files.

---

## Task 1: Probe the deploy shape before anchoring anything on it

The riskiest assumption in the design: that Vercel's **"Include source files outside of the Root Directory in the Build Step"** plus eve's bundler can resolve an npm `file:` dependency pointing outside the project root (`packages/agent-workflows/steward`). Prove it with a throwaway-sized package **before** writing the real module. If it fails, the fallback is publishing `temper-ts@0.0.x` to npm and taking a normal dependency — and we want to know that now, not after the module exists.

**Files:**
- Create: `clients/temper-ts/package.json`
- Create: `clients/temper-ts/tsconfig.json`
- Create: `clients/temper-ts/src/index.ts`
- Create: `clients/temper-ts/.gitignore`
- Modify: `packages/agent-workflows/steward/package.json`
- Modify: `packages/agent-workflows/steward/agent/schedules/steward.ts:51`

**Interfaces:**
- Consumes: nothing.
- Produces: `TEMPER_TS_VERSION: string` exported from `temper-ts`. The package's `dist/` build output, its `exports` map, and the steward's `file:` dependency + `build:dep` script — all later tasks build on these.

- [ ] **Step 1: Create the package manifest**

`clients/temper-ts/package.json`. `private: true` blocks *publishing* only — a `file:` dependency installs a private package fine. The `./testing` export exists from the start so later tasks add files rather than reshape the manifest.

```json
{
  "name": "temper-ts",
  "version": "0.0.0",
  "private": true,
  "description": "TypeScript client for the Temper knowledge base API.",
  "type": "module",
  "main": "./dist/index.js",
  "types": "./dist/index.d.ts",
  "exports": {
    ".": {
      "types": "./dist/index.d.ts",
      "default": "./dist/index.js"
    },
    "./testing": {
      "types": "./dist/testing/index.d.ts",
      "default": "./dist/testing/index.js"
    }
  },
  "files": ["dist"],
  "scripts": {
    "build": "tsc",
    "test": "vitest run",
    "typecheck": "tsc --noEmit"
  },
  "devDependencies": {
    "@types/node": "24.x",
    "typescript": "^5.8",
    "vitest": "^3"
  },
  "engines": {
    "node": ">=22"
  }
}
```

- [ ] **Step 2: Create the tsconfig and gitignore**

`clients/temper-ts/tsconfig.json` — emits `dist/`, which is what the `exports` map points at.

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022"],
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "types": ["node"],
    "strict": true,
    "declaration": true,
    "outDir": "dist",
    "rootDir": "src",
    "esModuleInterop": true,
    "skipLibCheck": true
  },
  "include": ["src/**/*.ts"],
  "exclude": ["tests", "dist"]
}
```

`clients/temper-ts/.gitignore`:

```
node_modules
dist
*.tsbuildinfo
```

- [ ] **Step 3: Create the probe export**

`clients/temper-ts/src/index.ts`. A value (not a type) so the bundler cannot tree-shake the dependency away — the whole point is to force real resolution through the build.

```ts
/**
 * The TypeScript client for the Temper knowledge base API. Sibling of `temper-rb`
 * (and the coming `temper-py`); the three are pinned to the same wire contracts.
 */

/** Identifies the client in server logs, and — being a value — keeps the bundler from tree-shaking this package out of a consumer's build. */
export const TEMPER_TS_VERSION = "0.0.0";
```

- [ ] **Step 4: Install and build the package**

Run from `clients/temper-ts`:

```bash
cd clients/temper-ts && npm install && npm run build && ls dist
```

Expected: `dist/index.js` and `dist/index.d.ts` exist. Commit the generated `package-lock.json` — CI's `npm ci` and the steward's `build:dep` both need it.

- [ ] **Step 5: Wire the steward's `file:` dependency**

Modify `packages/agent-workflows/steward/package.json`. `build:dep` builds the symlinked package explicitly rather than relying on npm's `prepare` semantics for `file:` specs, which differ across npm versions. `prebuild`/`pretest` are npm lifecycle hooks — npm runs them automatically before `build`/`test`, so Vercel's `npm run build` picks it up with no `vercel.json` change.

```json
{
  "name": "@temper/steward-agent",
  "version": "0.0.0",
  "type": "module",
  "imports": {
    "#*": "./agent/*",
    "#evals/*": "./evals/*"
  },
  "scripts": {
    "build:dep": "npm --prefix ../../../clients/temper-ts ci && npm --prefix ../../../clients/temper-ts run build",
    "prebuild": "npm run build:dep",
    "build": "eve build",
    "dev": "eve dev",
    "start": "eve start",
    "typecheck": "tsc"
  },
  "dependencies": {
    "@vercel/connect": "0.2.2",
    "ai": "^7.0.0",
    "eve": "^0.18.1",
    "temper-ts": "file:../../../clients/temper-ts",
    "zod": "4.4.3"
  },
  "devDependencies": {
    "@types/node": "24.x",
    "typescript": "7.0.1-rc"
  },
  "overrides": {
    "ai": "^7.0.0"
  },
  "engines": {
    "node": "24.x"
  }
}
```

- [ ] **Step 6: Consume the export from a real code path**

Modify `packages/agent-workflows/steward/agent/schedules/steward.ts`. Add the import at the top of the import block:

```ts
import { TEMPER_TS_VERSION } from "temper-ts";
```

and change the tick's opening log line (currently line 51) to carry it, so a deployed build *proves* the dependency resolved — visible in the steward's Vercel logs rather than inferred from a green build:

```ts
        console.log(`[steward-dispatch] tick ${correlationId} starting (temper-ts ${TEMPER_TS_VERSION})`);
```

- [ ] **Step 7: Verify the steward installs, typechecks, and builds locally**

Run from `packages/agent-workflows/steward`:

```bash
cd packages/agent-workflows/steward && npm install && npm run typecheck && npm run build
```

Expected: install symlinks `node_modules/temper-ts` → `../../../clients/temper-ts`; `typecheck` passes; `build` runs `prebuild` (building temper-ts) then `eve build` with no unresolved-import error.

- [ ] **Step 8: Enable the Vercel setting**

**This step is operator-run — do not delegate it to a subagent.** In the Vercel dashboard for project `steward-agent` (`prj_fCEcdlF3QiOO2FU76AjylBoqKcQJ`, Root Directory `packages/agent-workflows/steward`): Settings → Build & Deployment → Root Directory → enable **"Include source files outside of the Root Directory in the Build Step"**.

- [ ] **Step 9: Commit and push — the preview deployment IS the test**

```bash
git add clients/temper-ts packages/agent-workflows/steward/package.json packages/agent-workflows/steward/package-lock.json packages/agent-workflows/steward/agent/schedules/steward.ts
git commit -m "probe(steward): file: dep on clients/temper-ts — prove the deploy shape"
git push -u origin jct/steward-m2m-shared-credentials-model-config
```

Then watch the `steward-agent` preview deployment:

```bash
vercel ls steward-agent
```

Expected: the preview reaches `READY`, and its build log shows `prebuild` compiling temper-ts. **Note:** a Vercel preview deploy for this repo is known to fail-then-succeed on the first attempt — red is not necessarily broken; read the log before concluding.

**GATE — do not proceed past this task until the preview is green.** If the build cannot resolve `temper-ts`, stop and switch to the fallback: publish `temper-ts@0.0.x` to npm and change the dependency to a version range. Every later task is unchanged by that switch; only the dependency line and the `build:dep`/`prebuild` scripts go away.

---

## Task 2: Extend the wire contract — and fix the CI gap that hides it

The contract file currently describes only the **request**. The mock issuer must also be faithful to the **response** and to **credential transport**, and both real issuers already agree on those.

While doing this, fix a live gap: `test-ruby` is path-scoped to `clients/temper-rb/`, `openapi.json`, and its own workflow — **`tests/contracts/` is not in its trigger set.** But `clients/temper-rb/spec/temper/credentials_spec.rb:30` reads `tests/contracts/m2m-token-request.json` and asserts against it. So today, changing the M2M wire contract does **not** run the gem spec that asserts it. This very PR would otherwise change the contract with that spec never running.

**Files:**
- Modify: `tests/contracts/m2m-token-request.json`
- Modify: `.github/scripts/detect-ci-scope.sh:130-134`
- Modify: `.github/scripts/test-detect-ci-scope.sh`
- Modify: `packages/temper-cloud/tests/integration/oauth/client-credentials.test.ts:105-118`

**Interfaces:**
- Consumes: nothing.
- Produces: contract keys `response` (`{ fields: string[], no_refresh_token: true }`) and `credential_transport` (`{ client_secret_post: string[], client_secret_basic: string[] }`), read by Task 3's mock issuer and Task 4's client tests.

- [ ] **Step 1: Add the response and transport sections to the contract**

Modify `tests/contracts/m2m-token-request.json`. Keep the existing keys exactly as they are; add the `$comment` consumer line and the two new sections. The full file:

```json
{
  "$comment": [
    "The wire contract for an OAuth 2.0 client_credentials token request, shared by every temper",
    "M2M client and by temper's own authorization server.",
    "",
    "This file exists because a contract asserted only against itself is not asserted at all. The",
    "Ruby gem minted with a JSON body and proved it with a stub that parsed JSON; temper's AS read",
    "the body with `req.formData()` and proved THAT with a form-encoded request. Both suites were",
    "green and no client could mint against temper's issuer. Auth0 tolerates JSON, so the defect",
    "was invisible for as long as Auth0 was the only issuer any client faced.",
    "",
    "Consumed by:",
    "  - clients/temper-rb/spec/temper/credentials_spec.rb  (the client emits this shape)",
    "  - clients/temper-ts/tests/contract.test.ts           (the client emits this shape)",
    "  - clients/temper-ts/src/testing/mock-issuer.ts       (the mock issuer IS this shape)",
    "  - packages/temper-cloud/tests/integration/oauth/client-credentials.test.ts  (the server accepts it)",
    "",
    "Adding a client (temper-py) means pinning it against this file too."
  ],

  "content_type": "application/x-www-form-urlencoded",

  "required_params": ["grant_type", "client_id", "client_secret"],

  "optional_params": {
    "audience": [
      "Auth0 requires it. Temper's own AS ignores a request-supplied audience entirely and mints",
      "with its server-side AS_AUDIENCE, so a temper-issued client must be able to omit it."
    ]
  },

  "grant_type": "client_credentials",

  "rejected_content_types": {
    "application/json": [
      "RFC 6749 §4 mandates form encoding at the token endpoint. Auth0 accepts JSON as an extension;",
      "temper's AS does not, and must answer `invalid_request` rather than throwing a 500."
    ]
  },

  "credential_transport": {
    "client_secret_post": [
      "client_id and client_secret travel in the form body. Both issuers accept this, and it is what",
      "temper's own clients emit."
    ],
    "client_secret_basic": [
      "client_id:client_secret base64'd in an HTTP Basic Authorization header. RFC 6749 §2.3.1 says a",
      "server that supports both MUST prefer Basic when it is present; temper's AS does."
    ]
  },

  "response": {
    "fields": ["access_token", "token_type", "expires_in"],
    "token_type": "Bearer",
    "no_refresh_token": [
      "RFC 6749 §4.4.3: a client_credentials response MUST NOT include a refresh token. The credential",
      "IS the refresh mechanism — a machine re-mints. A client therefore caches against an ABSOLUTE",
      "expiry (expires_in is a duration, and a duration cannot survive being cached) and re-mints on a",
      "401, because a token checked at the top of a long unit of work can die in the middle of it."
    ]
  }
}
```

- [ ] **Step 2: Add `tests/contracts/` to test-ruby's trigger set**

Modify `.github/scripts/detect-ci-scope.sh`. Replace the `HAS_RUBY` block (currently lines ~130-134):

```bash
# Ruby SDK: the gem's own tree, the contracts it is asserted against, and its CI
# workflow. openapi.json is in this set precisely because a contract change must
# be SEEN to move the gem -- that is what the codegen drift gate proves. The same
# logic applies to tests/contracts/: credentials_spec.rb reads
# m2m-token-request.json and asserts the gem emits it, so a contract change that
# does not run this job is a contract change nothing checks.
#
# The no-diff safety fallback must run everything, this job included.
HAS_RUBY=false
if changes_match '^clients/temper-rb/|^tests/contracts/|^openapi\.json$|^\.github/workflows/test-ruby\.yml$|^__force_full_ci__$'; then
    HAS_RUBY=true
fi
```

- [ ] **Step 3: Write the failing scope test**

Add to `.github/scripts/test-detect-ci-scope.sh`, alongside the existing `run_test` cases:

```bash
run_test "contract change triggers the ruby gem spec that asserts it" \
    "tests/contracts/m2m-token-request.json" \
    "DOCS_ONLY=false" "RUN_TEST_RUBY=true"
```

- [ ] **Step 4: Run the scope tests**

```bash
bash .github/scripts/test-detect-ci-scope.sh
```

Expected: all PASS, including the new case. (Run this **before** Step 2's edit to see it FAIL with `RUN_TEST_RUBY: expected='true' actual='false'` — that failure is the proof the gap was real.)

- [ ] **Step 5: Assert the new contract sections in the AS test**

Modify `packages/temper-cloud/tests/integration/oauth/client-credentials.test.ts`. Widen the `contract` type in the `describe("the shared M2M wire contract")` block (currently lines 105-118):

```ts
    ) as {
      content_type: string;
      required_params: string[];
      grant_type: string;
      response: { fields: string[]; token_type: string };
    };
```

and add this test inside that same `describe` block, after the existing "accepts a request built from the contract" test:

```ts
    // The response half of the contract. The client caches against an ABSOLUTE expiry derived from
    // expires_in and re-mints on 401 — both of which are unimplementable if these fields drift.
    it("returns exactly the response shape the contract promises, with no refresh token", async () => {
      await seedTemperClient(sql, "tmpr_response", "s3cr3t");

      const res = await handleToken(
        tokenRequest({
          grant_type: "client_credentials",
          client_id: "tmpr_response",
          client_secret: "s3cr3t",
        }),
        db,
      );

      expect(res.status).toBe(200);
      const body = await res.json();
      for (const field of contract.response.fields) {
        expect(body[field]).toBeDefined();
      }
      expect(body.token_type).toBe(contract.response.token_type);
      expect(body.refresh_token).toBeUndefined();
    });
```

- [ ] **Step 6: Run the AS integration suite**

Postgres must be up (`cargo make docker-up` from the repo root if it is not).

```bash
cd packages/temper-cloud && bun run test:integration -- oauth/client-credentials
```

Expected: PASS, including the new response-shape test.

- [ ] **Step 7: Run the Ruby gem spec**

```bash
cd clients/temper-rb && bundle install && bundle exec rspec spec/temper/credentials_spec.rb
```

Expected: PASS. The contract change is purely additive — the gem reads `content_type`, `required_params`, and `grant_type`, none of which moved.

- [ ] **Step 8: Commit**

```bash
git add tests/contracts/m2m-token-request.json .github/scripts/detect-ci-scope.sh .github/scripts/test-detect-ci-scope.sh packages/temper-cloud/tests/integration/oauth/client-credentials.test.ts
git commit -m "contract(m2m): pin the response shape + transport, and run the gem spec that asserts it

tests/contracts/ was not in test-ruby's trigger set, so a change to the M2M wire
contract never ran credentials_spec.rb — the spec that asserts the gem emits it."
```

---

## Task 3: The faithful mock issuer

A real `node:http` server with two personalities. It is **built from the contract file**, and the real AS is asserted against that same file (Task 2) — so a mock that drifts from the AS breaks the AS's own test first. That transitivity is what lets us prove the temper-AS path without standing an AS up.

**Files:**
- Create: `clients/temper-ts/src/testing/mock-issuer.ts`
- Create: `clients/temper-ts/src/testing/index.ts`
- Create: `clients/temper-ts/tests/mock-issuer.test.ts`
- Create: `clients/temper-ts/vitest.config.ts`

**Interfaces:**
- Consumes: `tests/contracts/m2m-token-request.json` (Task 2).
- Produces:
  - `type IssuerFlavor = "auth0" | "temper-as"`
  - `interface MintRequest { contentType: string; params: Record<string, string>; basic?: { clientId: string; clientSecret: string } }`
  - `interface MockIssuer { url: string; requests: MintRequest[]; close(): Promise<void> }`
  - `startMockIssuer(opts: MockIssuerOptions): Promise<MockIssuer>` where `MockIssuerOptions = { flavor: IssuerFlavor; clientId: string; clientSecret: string; audience?: string; expiresInSeconds?: number; previousSecret?: string; previousSecretExpiresAt?: number }`
  - Both re-exported from `temper-ts/testing`. Task 4 and Task 5 drive this.

- [ ] **Step 1: Add the vitest config**

`clients/temper-ts/vitest.config.ts`:

```ts
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["tests/**/*.test.ts"],
    environment: "node",
  },
});
```

- [ ] **Step 2: Write the failing test**

`clients/temper-ts/tests/mock-issuer.test.ts`. This test is what makes the mock *faithful*: every assertion is read from the contract, not hardcoded.

```ts
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { type MockIssuer, startMockIssuer } from "../src/testing/index.js";

const contract = JSON.parse(
  readFileSync(
    fileURLToPath(new URL("../../../tests/contracts/m2m-token-request.json", import.meta.url)),
    "utf8",
  ),
) as {
  content_type: string;
  grant_type: string;
  response: { fields: string[]; token_type: string };
};

let issuer: MockIssuer | undefined;

afterEach(async () => {
  await issuer?.close();
  issuer = undefined;
});

/** The form-encoded mint every client emits. Deliberately NOT using ClientCredentials — this test proves the MOCK, not the client. */
async function mint(url: string, params: Record<string, string>, init: RequestInit = {}) {
  return fetch(url, {
    method: "POST",
    headers: { "content-type": contract.content_type, ...(init.headers ?? {}) },
    body: new URLSearchParams(params),
    ...init,
  });
}

describe("the temper-AS-shaped issuer", () => {
  it("mints the contract's response shape and never a refresh token", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });

    const res = await mint(issuer.url, {
      grant_type: contract.grant_type,
      client_id: "tmpr_a",
      client_secret: "s3cr3t",
    });

    expect(res.status).toBe(200);
    const body = await res.json();
    for (const field of contract.response.fields) {
      expect(body[field]).toBeDefined();
    }
    expect(body.token_type).toBe(contract.response.token_type);
    expect(body.refresh_token).toBeUndefined();
    // The AS's AS_ACCESS_TTL_SECONDS default. Short enough that a tick can outlive its token.
    expect(body.expires_in).toBe(900);
  });

  it("ignores a request-supplied audience rather than rejecting it", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });

    const res = await mint(issuer.url, {
      grant_type: contract.grant_type,
      client_id: "tmpr_a",
      client_secret: "s3cr3t",
      audience: "https://ignored.example",
    });

    expect(res.status).toBe(200);
  });

  it("refuses a JSON body with invalid_request rather than throwing", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });

    const res = await fetch(issuer.url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ grant_type: "client_credentials", client_id: "tmpr_a", client_secret: "s3cr3t" }),
    });

    expect(res.status).toBe(400);
    expect((await res.json()).error).toBe("invalid_request");
  });

  it("accepts credentials via HTTP Basic", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });

    const res = await mint(
      issuer.url,
      { grant_type: contract.grant_type },
      { headers: { authorization: `Basic ${Buffer.from("tmpr_a:s3cr3t").toString("base64")}` } },
    );

    expect(res.status).toBe(200);
    expect(issuer.requests[0]?.basic).toEqual({ clientId: "tmpr_a", clientSecret: "s3cr3t" });
  });

  it("accepts the previous secret inside its grace window and rejects it after", async () => {
    issuer = await startMockIssuer({
      flavor: "temper-as",
      clientId: "tmpr_a",
      clientSecret: "new-secret",
      previousSecret: "old-secret",
      previousSecretExpiresAt: Date.now() + 60_000,
    });

    const inside = await mint(issuer.url, {
      grant_type: contract.grant_type,
      client_id: "tmpr_a",
      client_secret: "old-secret",
    });
    expect(inside.status).toBe(200);

    await issuer.close();
    issuer = await startMockIssuer({
      flavor: "temper-as",
      clientId: "tmpr_a",
      clientSecret: "new-secret",
      previousSecret: "old-secret",
      previousSecretExpiresAt: Date.now() - 1,
    });

    const lapsed = await mint(issuer.url, {
      grant_type: contract.grant_type,
      client_id: "tmpr_a",
      client_secret: "old-secret",
    });
    expect(lapsed.status).toBe(401);
    expect((await lapsed.json()).error).toBe("invalid_client");
  });

  it("rejects a wrong secret with invalid_client", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });

    const res = await mint(issuer.url, {
      grant_type: contract.grant_type,
      client_id: "tmpr_a",
      client_secret: "wrong",
    });

    expect(res.status).toBe(401);
    expect((await res.json()).error).toBe("invalid_client");
  });
});

describe("the Auth0-shaped issuer", () => {
  it("requires an audience", async () => {
    issuer = await startMockIssuer({
      flavor: "auth0",
      clientId: "auth0_a",
      clientSecret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });

    const without = await mint(issuer.url, {
      grant_type: contract.grant_type,
      client_id: "auth0_a",
      client_secret: "s3cr3t",
    });
    expect(without.status).toBe(400);

    const with_ = await mint(issuer.url, {
      grant_type: contract.grant_type,
      client_id: "auth0_a",
      client_secret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });
    expect(with_.status).toBe(200);
  });

  // Auth0 tolerates JSON as an extension. This is EXACTLY why the gem's JSON mint stayed green for
  // months: the only issuer it ever faced forgave it. The mock forgives it too, or it would not be
  // faithful — and a client that only ever meets a strict mock would never catch this class of bug.
  it("tolerates a JSON body", async () => {
    issuer = await startMockIssuer({
      flavor: "auth0",
      clientId: "auth0_a",
      clientSecret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });

    const res = await fetch(issuer.url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        grant_type: "client_credentials",
        client_id: "auth0_a",
        client_secret: "s3cr3t",
        audience: "https://temperkb.io/api",
      }),
    });

    expect(res.status).toBe(200);
  });

  it("mints a long-lived token", async () => {
    issuer = await startMockIssuer({
      flavor: "auth0",
      clientId: "auth0_a",
      clientSecret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });

    const res = await mint(issuer.url, {
      grant_type: contract.grant_type,
      client_id: "auth0_a",
      client_secret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });

    expect((await res.json()).expires_in).toBe(86_400);
  });
});
```

- [ ] **Step 3: Run the test to verify it fails**

```bash
cd clients/temper-ts && npx vitest run tests/mock-issuer.test.ts
```

Expected: FAIL — `Failed to resolve import "../src/testing/index.js"`.

- [ ] **Step 4: Implement the mock issuer**

`clients/temper-ts/src/testing/mock-issuer.ts`:

```ts
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";

/**
 * A faithful in-process stand-in for the two issuers a temper instance can be fronted by.
 *
 * Faithfulness is TRANSITIVE, and that is the whole design: this mock is built from
 * `tests/contracts/m2m-token-request.json`, and the REAL authorization server is asserted against
 * that same file by packages/temper-cloud's oauth integration suite. A mock that drifts from the AS
 * breaks the AS's own test first. That is what lets a client prove the temper-AS path — the one a
 * self-hosted/SAML instance depends on — without standing an AS up.
 *
 * The two flavors differ in ways that MATTER to a client, and the differences are the point:
 *
 *   auth0      — `audience` is required; a JSON body is tolerated (Auth0's extension); long TTL.
 *   temper-as  — `audience` is ignored entirely; a JSON body is `invalid_request`; 900s TTL; a
 *                rotated previous secret stays valid inside its grace window.
 *
 * The Auth0 flavor's JSON tolerance is deliberately reproduced. It is precisely why a JSON-minting
 * client stayed green for months: the only issuer it ever met forgave it.
 */

export type IssuerFlavor = "auth0" | "temper-as";

/** A recorded mint attempt, so a test can assert what the client actually put on the wire. */
export interface MintRequest {
  contentType: string;
  params: Record<string, string>;
  /** Present when the client used `client_secret_basic` instead of putting credentials in the body. */
  basic?: { clientId: string; clientSecret: string };
}

export interface MockIssuerOptions {
  flavor: IssuerFlavor;
  clientId: string;
  clientSecret: string;
  /** auth0 only: the audience the issuer demands. Ignored by the temper-as flavor, as the real AS ignores it. */
  audience?: string;
  /** Defaults: 86400 (auth0), 900 (temper-as — the AS's AS_ACCESS_TTL_SECONDS default). */
  expiresInSeconds?: number;
  /** temper-as only: a rotated-out secret, valid until `previousSecretExpiresAt`. */
  previousSecret?: string;
  /** Absolute ms since epoch. */
  previousSecretExpiresAt?: number;
}

export interface MockIssuer {
  /** The token endpoint — hand this to a client as its `tokenUrl`. */
  url: string;
  /** Every mint attempt, in order. */
  requests: MintRequest[];
  close(): Promise<void>;
}

function json(res: ServerResponse, status: number, body: unknown): void {
  const payload = JSON.stringify(body);
  res.writeHead(status, { "content-type": "application/json" });
  res.end(payload);
}

async function readBody(req: IncomingMessage): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of req) {
    chunks.push(chunk as Buffer);
  }
  return Buffer.concat(chunks).toString("utf8");
}

/** RFC 6749 §2.3.1 — `Basic base64(client_id:client_secret)`. The AS prefers this over the body when present. */
function parseBasic(header: string | undefined): { clientId: string; clientSecret: string } | undefined {
  if (header === undefined || !header.startsWith("Basic ")) {
    return undefined;
  }
  const decoded = Buffer.from(header.slice("Basic ".length), "base64").toString("utf8");
  const separator = decoded.indexOf(":");
  if (separator === -1) {
    return undefined;
  }
  return { clientId: decoded.slice(0, separator), clientSecret: decoded.slice(separator + 1) };
}

export async function startMockIssuer(opts: MockIssuerOptions): Promise<MockIssuer> {
  const requests: MintRequest[] = [];
  const isAs = opts.flavor === "temper-as";
  const expiresIn = opts.expiresInSeconds ?? (isAs ? 900 : 86_400);
  let minted = 0;

  const server: Server = createServer((req, res) => {
    void (async () => {
      const contentType = (req.headers["content-type"] ?? "").split(";")[0]?.trim() ?? "";
      const raw = await readBody(req);

      // RFC 6749 §4 mandates form encoding. Auth0 tolerates JSON; temper's AS answers
      // `invalid_request` — and must NOT throw, or a JSON-minting client cannot read its own error.
      let params: Record<string, string> = {};
      if (contentType === "application/json") {
        if (isAs) {
          json(res, 400, { error: "invalid_request", error_description: "body must be form-encoded" });
          return;
        }
        params = JSON.parse(raw) as Record<string, string>;
      } else {
        params = Object.fromEntries(new URLSearchParams(raw));
      }

      const basic = parseBasic(req.headers.authorization);
      requests.push({ contentType, params, ...(basic === undefined ? {} : { basic }) });

      const clientId = basic?.clientId ?? params.client_id;
      const clientSecret = basic?.clientSecret ?? params.client_secret;

      if (params.grant_type !== "client_credentials") {
        json(res, 400, { error: "unsupported_grant_type" });
        return;
      }

      // Auth0's audience is not part of the client_credentials protocol — it is Auth0's. The AS
      // ignores a request-supplied one entirely, which is why a temper-issued client omits it.
      if (!isAs && params.audience !== opts.audience) {
        json(res, 400, { error: "invalid_request", error_description: "audience is required" });
        return;
      }

      const secretIsCurrent = clientSecret === opts.clientSecret;
      const secretIsInGrace =
        isAs &&
        opts.previousSecret !== undefined &&
        clientSecret === opts.previousSecret &&
        (opts.previousSecretExpiresAt ?? 0) > Date.now();

      if (clientId !== opts.clientId || (!secretIsCurrent && !secretIsInGrace)) {
        json(res, 401, { error: "invalid_client" });
        return;
      }

      minted += 1;
      // No refresh token, ever (RFC 6749 §4.4.3): the credential IS the refresh mechanism.
      json(res, 200, {
        access_token: `${opts.flavor}-token-${minted}`,
        token_type: "Bearer",
        expires_in: expiresIn,
      });
    })();
  });

  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;

  return {
    url: `http://127.0.0.1:${port}/oauth/token`,
    requests,
    close: () =>
      new Promise<void>((resolve, reject) =>
        server.close((err) => (err ? reject(err) : resolve())),
      ),
  };
}
```

`clients/temper-ts/src/testing/index.ts`:

```ts
export {
  type IssuerFlavor,
  type MintRequest,
  type MockIssuer,
  type MockIssuerOptions,
  startMockIssuer,
} from "./mock-issuer.js";
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd clients/temper-ts && npx vitest run tests/mock-issuer.test.ts && npm run typecheck
```

Expected: all PASS; typecheck clean.

- [ ] **Step 6: Commit**

```bash
git add clients/temper-ts
git commit -m "feat(temper-ts): a faithful mock issuer, built from the shared wire contract

Two personalities that differ where it matters to a client: Auth0 (audience
required, JSON tolerated, long TTL) and temper's AS (audience ignored, JSON
refused, 900s TTL, rotation grace). Faithfulness is transitive — the real AS is
asserted against the same contract file, so a mock that drifts breaks the AS's
own test first."
```

---

## Task 4: `ClientCredentials` — the mint, shared

Transliterate `Temper::Credentials` (`clients/temper-rb/lib/temper/credentials.rb`) into TypeScript. That file was itself **ported from the steward** and fixed two bugs the steward still has; this task brings the fixed version home. Two first-party clients that mint differently is the bug class this whole arc has been fighting.

**Files:**
- Create: `clients/temper-ts/src/credentials.ts`
- Create: `clients/temper-ts/tests/credentials.test.ts`
- Create: `clients/temper-ts/tests/contract.test.ts`
- Modify: `clients/temper-ts/src/index.ts`

**Interfaces:**
- Consumes: `startMockIssuer` (Task 3).
- Produces, all exported from `temper-ts`:
  - `interface TokenResult { token: string; expiresAt: number }` — `expiresAt` is **absolute ms since epoch**, the shape eve's connection `auth.getToken` wants.
  - `interface Credentials { token(): Promise<string>; tokenResult(): Promise<TokenResult>; refresh(): Promise<TokenResult> }`
  - `class BearerToken implements Credentials` — `new BearerToken(token: string)`.
  - `class ClientCredentials implements Credentials` — `new ClientCredentials(opts: ClientCredentialsOptions)`.
  - `interface ClientCredentialsOptions { tokenUrl: string; clientId: string; clientSecret: string; audience?: string; now?: () => number }`
  - `class TokenMintError extends Error` — carries `status: number`.
  - Task 5's steward consumes `ClientCredentials`, `Credentials`, `TokenResult`, and `TokenMintError`.

- [ ] **Step 1: Write the failing tests**

`clients/temper-ts/tests/credentials.test.ts`:

```ts
import { afterEach, describe, expect, it } from "vitest";
import { ClientCredentials, TokenMintError } from "../src/index.js";
import { type MockIssuer, startMockIssuer } from "../src/testing/index.js";

let issuer: MockIssuer | undefined;

afterEach(async () => {
  await issuer?.close();
  issuer = undefined;
});

describe("ClientCredentials against a temper-issued (tmpr_) credential", () => {
  it("mints with NO audience — the AS ignores one, so sending it would be a lie", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
    });

    expect(await creds.token()).toBe("temper-as-token-1");
    expect(issuer.requests[0]?.contentType).toBe("application/x-www-form-urlencoded");
    expect(issuer.requests[0]?.params.audience).toBeUndefined();
  });

  it("caches against an ABSOLUTE expiry and re-mints only past the skew", async () => {
    issuer = await startMockIssuer({
      flavor: "temper-as",
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
      expiresInSeconds: 900,
    });
    let now = 1_000_000;
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
      now: () => now,
    });

    expect(await creds.token()).toBe("temper-as-token-1");

    // Inside the token's life, outside the 60s skew — the cache holds.
    now += 800_000;
    expect(await creds.token()).toBe("temper-as-token-1");
    expect(issuer.requests).toHaveLength(1);

    // Inside the 60s skew of a 900s token — re-mint AHEAD of expiry rather than racing it.
    now += 60_000;
    expect(await creds.token()).toBe("temper-as-token-2");
    expect(issuer.requests).toHaveLength(2);
  });

  it("reports the absolute expiry eve needs to refresh ahead of a 401", async () => {
    issuer = await startMockIssuer({
      flavor: "temper-as",
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
      expiresInSeconds: 900,
    });
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
      now: () => 1_000_000,
    });

    expect(await creds.tokenResult()).toEqual({
      token: "temper-as-token-1",
      expiresAt: 1_000_000 + 900_000,
    });
  });

  it("re-mints on refresh() even when the cached token is still fresh", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
    });

    expect(await creds.token()).toBe("temper-as-token-1");
    // This is the fix the gem documented against the steward: refresh-ahead-of-expiry alone is
    // insufficient, because a tick can outlive a token it checked at the top.
    expect((await creds.refresh()).token).toBe("temper-as-token-2");
    expect(await creds.token()).toBe("temper-as-token-2");
  });

  it("mints with the rotated-out previous secret while its grace window is open", async () => {
    issuer = await startMockIssuer({
      flavor: "temper-as",
      clientId: "tmpr_a",
      clientSecret: "new-secret",
      previousSecret: "old-secret",
      previousSecretExpiresAt: Date.now() + 60_000,
    });
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "tmpr_a",
      clientSecret: "old-secret",
    });

    expect(await creds.token()).toBe("temper-as-token-1");
  });

  it("throws TokenMintError carrying the status on a bad secret", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "tmpr_a",
      clientSecret: "wrong",
    });

    await expect(creds.token()).rejects.toThrow(TokenMintError);
    await expect(creds.token()).rejects.toMatchObject({ status: 401 });
  });
});

describe("ClientCredentials against an Auth0-provisioned credential", () => {
  it("sends the audience when configured — Auth0 requires it", async () => {
    issuer = await startMockIssuer({
      flavor: "auth0",
      clientId: "auth0_a",
      clientSecret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "auth0_a",
      clientSecret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });

    expect(await creds.token()).toBe("auth0-token-1");
    expect(issuer.requests[0]?.params.audience).toBe("https://temperkb.io/api");
  });

  // The bite: the SAME client object, given no audience, must fail against Auth0. If this passes,
  // the audience is not actually reaching the wire and the test above proves nothing.
  it("fails against Auth0 when no audience is configured", async () => {
    issuer = await startMockIssuer({
      flavor: "auth0",
      clientId: "auth0_a",
      clientSecret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });
    const creds = new ClientCredentials({
      tokenUrl: issuer.url,
      clientId: "auth0_a",
      clientSecret: "s3cr3t",
    });

    await expect(creds.token()).rejects.toMatchObject({ status: 400 });
  });
});
```

`clients/temper-ts/tests/contract.test.ts` — temper-ts becomes the contract's third consumer, exactly as the file's own header demands:

```ts
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, expect, it } from "vitest";
import { ClientCredentials } from "../src/index.js";
import { type MockIssuer, startMockIssuer } from "../src/testing/index.js";

// The client half of the cross-language wire contract in tests/contracts/m2m-token-request.json.
// The gem's spec asserts IT emits this shape; the AS's integration test asserts the server ACCEPTS
// it. Neither catches a mismatch alone — which is exactly how the gem shipped a JSON mint against a
// formData() parser with both suites green.
const contract = JSON.parse(
  readFileSync(
    fileURLToPath(new URL("../../../tests/contracts/m2m-token-request.json", import.meta.url)),
    "utf8",
  ),
) as {
  content_type: string;
  required_params: string[];
  grant_type: string;
};

let issuer: MockIssuer | undefined;

afterEach(async () => {
  await issuer?.close();
  issuer = undefined;
});

it("emits exactly the content type and params the shared wire contract requires", async () => {
  issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_c", clientSecret: "s3cr3t" });
  const creds = new ClientCredentials({
    tokenUrl: issuer.url,
    clientId: "tmpr_c",
    clientSecret: "s3cr3t",
  });

  await creds.token();

  const sent = issuer.requests[0];
  expect(sent?.contentType).toBe(contract.content_type);
  expect(Object.keys(sent?.params ?? {})).toEqual(expect.arrayContaining(contract.required_params));
  expect(sent?.params.grant_type).toBe(contract.grant_type);
});
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd clients/temper-ts && npx vitest run tests/credentials.test.ts tests/contract.test.ts
```

Expected: FAIL — `ClientCredentials` and `TokenMintError` are not exported from `../src/index.js`.

- [ ] **Step 3: Implement the credentials module**

`clients/temper-ts/src/credentials.ts`:

```ts
/**
 * Two strategies behind one interface. Precedence is the CALLER's explicit choice, never discovered
 * from the environment — that is how the steward's schedules went Connect-first while its MCP
 * connection went M2M-first, so on the Auth0-fronted instance the schedules' REST calls silently
 * failed while MCP worked.
 *
 * This is `Temper::Credentials` (clients/temper-rb/lib/temper/credentials.rb) transliterated. That
 * Ruby module was itself ported FROM the steward's hand-rolled mint, and fixed two bugs in the
 * process; this brings the fixed version home so the two first-party clients cannot drift again.
 * Of the gem's two divergences, only ONE is reproduced here:
 *
 *   - `refresh()` — KEPT. Refresh-ahead-of-expiry alone is insufficient: the steward resolves a
 *     token once per tick, so a tick outliving its cached token takes a 401 that nothing recovers.
 *     Temper's own AS mints 900-second tokens by default, which makes that ordinary rather than
 *     exotic. Re-mint ON 401.
 *   - The mutex — DROPPED. The gem needs one because Puma is threaded. A serverless function is
 *     not, and a bare field is the honest shape for it.
 */

/** `expiresAt` is ABSOLUTE (ms since epoch). A duration cannot survive being cached — and eve's connection auth wants exactly this shape. */
export interface TokenResult {
  token: string;
  expiresAt: number;
}

export interface Credentials {
  token(): Promise<string>;
  tokenResult(): Promise<TokenResult>;
  refresh(): Promise<TokenResult>;
}

export class TokenMintError extends Error {
  readonly status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = "TokenMintError";
    this.status = status;
  }
}

/** A token the caller already holds — a request serving a signed-in human. No I/O, no refresh. */
export class BearerToken implements Credentials {
  readonly #token: string;

  constructor(token: string) {
    if (token === "") {
      throw new TypeError("token must be a non-empty string");
    }
    this.#token = token;
  }

  async token(): Promise<string> {
    return this.#token;
  }

  async tokenResult(): Promise<TokenResult> {
    // No expiry is knowable from a token handed to us. `0` would claim "already expired"; a caller
    // that needs refresh-ahead must use ClientCredentials.
    return { token: this.#token, expiresAt: Number.POSITIVE_INFINITY };
  }

  async refresh(): Promise<TokenResult> {
    throw new TokenMintError("BearerToken cannot refresh; mint a new token upstream", 401);
  }
}

export interface ClientCredentialsOptions {
  tokenUrl: string;
  clientId: string;
  clientSecret: string;
  /**
   * Auth0 REQUIRES it; temper's own AS ignores a request-supplied audience entirely and mints with
   * its server-side AS_AUDIENCE. Omit it for a temper-issued (`tmpr_`) credential — sending an
   * empty one would be a lie.
   */
  audience?: string;
  /** Injectable clock (ms since epoch) — tests drive expiry without sleeping. */
  now?: () => number;
}

/** A `client_credentials` machine principal. Works against BOTH issuers a temper instance can be fronted by. */
export class ClientCredentials implements Credentials {
  /** Re-mint this far AHEAD of expiry rather than racing it. */
  static readonly SKEW_MS = 60_000;

  readonly #tokenUrl: string;
  readonly #clientId: string;
  readonly #clientSecret: string;
  readonly #audience: string | undefined;
  readonly #now: () => number;
  #cached: TokenResult | undefined;

  constructor(opts: ClientCredentialsOptions) {
    this.#tokenUrl = requireNonEmpty(opts.tokenUrl, "tokenUrl");
    this.#clientId = requireNonEmpty(opts.clientId, "clientId");
    this.#clientSecret = requireNonEmpty(opts.clientSecret, "clientSecret");
    this.#audience = opts.audience === undefined ? undefined : requireNonEmpty(opts.audience, "audience");
    this.#now = opts.now ?? (() => Date.now());
  }

  async token(): Promise<string> {
    return (await this.tokenResult()).token;
  }

  async tokenResult(): Promise<TokenResult> {
    if (this.#cached !== undefined && this.#cached.expiresAt - ClientCredentials.SKEW_MS > this.#now()) {
      return this.#cached;
    }
    return this.refresh();
  }

  /** Mint unconditionally, discarding any cached token. The on-401 path — see the class comment. */
  async refresh(): Promise<TokenResult> {
    const res = await fetch(this.#tokenUrl, {
      method: "POST",
      // RFC 6749 §4 mandates form encoding. Auth0 tolerates JSON, which is why a JSON mint stayed
      // green for as long as Auth0 was the only issuer any client faced; temper's AS reads the body
      // with `req.formData()` and a JSON mint never reaches its grant branch.
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: this.#requestBody(),
    });

    if (!res.ok) {
      throw new TokenMintError(
        `token mint failed (${res.status}): ${await res.text()}`,
        res.status,
      );
    }

    const body = (await res.json()) as { access_token: string; expires_in: number };
    // Absolute, not relative: a duration cannot survive being cached.
    this.#cached = {
      token: body.access_token,
      expiresAt: this.#now() + body.expires_in * 1000,
    };
    return this.#cached;
  }

  #requestBody(): URLSearchParams {
    const params = new URLSearchParams({
      grant_type: "client_credentials",
      client_id: this.#clientId,
      client_secret: this.#clientSecret,
    });
    if (this.#audience !== undefined) {
      params.set("audience", this.#audience);
    }
    return params;
  }
}

function requireNonEmpty(value: string, name: string): string {
  if (typeof value !== "string" || value === "") {
    throw new TypeError(`${name} must be a non-empty string`);
  }
  return value;
}
```

- [ ] **Step 4: Export it from the package entry point**

Replace `clients/temper-ts/src/index.ts`:

```ts
/**
 * The TypeScript client for the Temper knowledge base API. Sibling of `temper-rb`
 * (and the coming `temper-py`); the three are pinned to the same wire contracts.
 */

export {
  BearerToken,
  ClientCredentials,
  type ClientCredentialsOptions,
  type Credentials,
  TokenMintError,
  type TokenResult,
} from "./credentials.js";

/** Identifies the client in server logs, and — being a value — keeps the bundler from tree-shaking this package out of a consumer's build. */
export const TEMPER_TS_VERSION = "0.0.0";
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd clients/temper-ts && npm test && npm run typecheck && npm run build
```

Expected: all tests PASS; typecheck clean; `dist/` builds.

- [ ] **Step 6: Commit**

```bash
git add clients/temper-ts
git commit -m "feat(temper-ts): ClientCredentials — one mint against both issuers

Temper::Credentials transliterated. The gem was ported FROM the steward and fixed
two bugs on the way; this brings the fixed version home. Audience is optional (it
is Auth0's, not the protocol's), expiry is cached absolute with a 60s skew, and
refresh() exists because a tick can outlive a token it checked at the top —
which the AS's 900s default TTL makes ordinary."
```

---

## Task 5: The steward composes it — optional audience, re-mint on 401

The steward keeps what is genuinely steward-specific (the `TEMPER_M2M_*` env resolution, the Vercel Connect and static-token strategies — eve/Vercel concepts with no business in a general SDK) and delegates the mint.

The bug being fixed: `temperToken()` hands out a *string*, and each schedule resolves one token then holds it across N parallel fetches. A token that dies mid-tick takes the tick down with it, and there is no recovery.

**Files:**
- Modify: `packages/agent-workflows/steward/agent/lib/temper-auth.ts` (full rewrite)
- Modify: `packages/agent-workflows/steward/agent/schedules/steward.ts:53-71`
- Modify: `packages/agent-workflows/steward/agent/schedules/materialize.ts:43-76`
- Modify: `packages/agent-workflows/steward/package.json`
- Modify: `packages/agent-workflows/steward/tsconfig.json`
- Create: `packages/agent-workflows/steward/vitest.config.ts`
- Create: `packages/agent-workflows/steward/tests/temper-auth.test.ts`
- Create: `clients/temper-ts/src/testing/mock-api.ts`
- Modify: `clients/temper-ts/src/testing/index.ts`

**Interfaces:**
- Consumes: `ClientCredentials`, `Credentials`, `TokenResult`, `TokenMintError` (Task 4); `startMockIssuer` (Task 3).
- Produces:
  - From `temper-ts/testing`: `startMockApi(opts: { rejectFirst?: number }): Promise<MockApi>` where `MockApi = { url: string; bearers: string[]; close(): Promise<void> }` — rejects the first `rejectFirst` requests with 401, recording every bearer it saw.
  - From the steward's `agent/lib/temper-auth.ts`: `mintM2mToken(): Promise<TokenResult>` (**name unchanged** — `agent/connections/temper.ts:34` passes it to eve as `getToken` and must not change), `temperToken(): Promise<string>`, `temperFetch(url: string, init: RequestInit, opts?: RetryOptions): Promise<Response>`, `requireEnv(name: string): string`.

- [ ] **Step 1: Add the mock API to temper-ts's testing surface**

`clients/temper-ts/src/testing/mock-api.ts`. The mock *issuer* cannot prove the on-401 path — a 401 comes from the resource server, not the token endpoint.

```ts
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";

/**
 * A stand-in resource server that rejects its first N requests with 401.
 *
 * This is what proves the on-401 re-mint: the failure a client must recover from is a 401 from the
 * RESOURCE server (a token that expired mid-flight), not from the token endpoint. `bearers` records
 * every Authorization header it saw, so a test can assert the retry carried a DIFFERENT, freshly
 * minted token rather than blindly replaying the dead one.
 */
export interface MockApi {
  url: string;
  /** Every bearer token presented, in order. */
  bearers: string[];
  close(): Promise<void>;
}

export interface MockApiOptions {
  /** Reject this many leading requests with 401 before serving 200. Default 0. */
  rejectFirst?: number;
}

export async function startMockApi(opts: MockApiOptions = {}): Promise<MockApi> {
  const bearers: string[] = [];
  const rejectFirst = opts.rejectFirst ?? 0;
  let seen = 0;

  const server: Server = createServer((req, res) => {
    const header = req.headers.authorization ?? "";
    bearers.push(header.replace(/^Bearer /, ""));
    seen += 1;

    if (seen <= rejectFirst) {
      res.writeHead(401, { "content-type": "application/json" });
      res.end(JSON.stringify({ error: "unauthorized" }));
      return;
    }

    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({ ok: true }));
  });

  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;

  return {
    url: `http://127.0.0.1:${port}/api/steward/dispatch`,
    bearers,
    close: () =>
      new Promise<void>((resolve, reject) => server.close((err) => (err ? reject(err) : resolve()))),
  };
}
```

Replace `clients/temper-ts/src/testing/index.ts`:

```ts
export {
  type IssuerFlavor,
  type MintRequest,
  type MockIssuer,
  type MockIssuerOptions,
  startMockIssuer,
} from "./mock-issuer.js";
export { type MockApi, type MockApiOptions, startMockApi } from "./mock-api.js";
```

- [ ] **Step 2: Add the steward's test harness**

Modify `packages/agent-workflows/steward/package.json` — add `vitest` and the `test` scripts. `pretest` builds the `file:` dependency so vitest can resolve `temper-ts`'s `exports` map (which points at `dist/`). Full `scripts` and `devDependencies` blocks:

```json
  "scripts": {
    "build:dep": "npm --prefix ../../../clients/temper-ts ci && npm --prefix ../../../clients/temper-ts run build",
    "prebuild": "npm run build:dep",
    "build": "eve build",
    "dev": "eve dev",
    "start": "eve start",
    "pretest": "npm run build:dep",
    "test": "vitest run",
    "typecheck": "tsc"
  },
  "devDependencies": {
    "@types/node": "24.x",
    "typescript": "7.0.1-rc",
    "vitest": "^3"
  },
```

Create `packages/agent-workflows/steward/vitest.config.ts`:

```ts
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["tests/**/*.test.ts"],
    environment: "node",
  },
});
```

Modify `packages/agent-workflows/steward/tsconfig.json` — add `tests` to `include` so `npm run typecheck` covers them:

```json
  "include": ["agent/**/*.ts", "evals/**/*.ts", "tests/**/*.ts", ".eve/**/*.d.ts"]
```

- [ ] **Step 3: Write the failing tests**

`packages/agent-workflows/steward/tests/temper-auth.test.ts`. The env is process-global, so each test sets it explicitly and the module cache is reset — the memoized credential must not leak between tests.

```ts
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { type MockApi, type MockIssuer, startMockApi, startMockIssuer } from "temper-ts/testing";

let issuer: MockIssuer | undefined;
let api: MockApi | undefined;

beforeEach(() => {
  vi.resetModules();
  delete process.env.TEMPER_M2M_CLIENT_ID;
  delete process.env.TEMPER_M2M_CLIENT_SECRET;
  delete process.env.TEMPER_M2M_TOKEN_URL;
  delete process.env.TEMPER_M2M_AUDIENCE;
  delete process.env.TEMPER_CONNECT_CONNECTOR;
  delete process.env.TEMPER_TOKEN;
});

afterEach(async () => {
  await issuer?.close();
  await api?.close();
  issuer = undefined;
  api = undefined;
});

describe("temper-auth env composition", () => {
  it("omits the audience when TEMPER_M2M_AUDIENCE is unset — a tmpr_ credential must be able to", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    process.env.TEMPER_M2M_CLIENT_ID = "tmpr_a";
    process.env.TEMPER_M2M_CLIENT_SECRET = "s3cr3t";
    process.env.TEMPER_M2M_TOKEN_URL = issuer.url;

    const { temperToken } = await import("../agent/lib/temper-auth.js");

    expect(await temperToken()).toBe("temper-as-token-1");
    expect(issuer.requests[0]?.params.audience).toBeUndefined();
  });

  it("sends the audience when TEMPER_M2M_AUDIENCE is set — Auth0 requires it", async () => {
    issuer = await startMockIssuer({
      flavor: "auth0",
      clientId: "auth0_a",
      clientSecret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });
    process.env.TEMPER_M2M_CLIENT_ID = "auth0_a";
    process.env.TEMPER_M2M_CLIENT_SECRET = "s3cr3t";
    process.env.TEMPER_M2M_TOKEN_URL = issuer.url;
    process.env.TEMPER_M2M_AUDIENCE = "https://temperkb.io/api";

    const { temperToken } = await import("../agent/lib/temper-auth.js");

    expect(await temperToken()).toBe("auth0-token-1");
    expect(issuer.requests[0]?.params.audience).toBe("https://temperkb.io/api");
  });

  it("mintM2mToken reports an absolute expiry, which is what eve refreshes against", async () => {
    issuer = await startMockIssuer({
      flavor: "temper-as",
      clientId: "tmpr_a",
      clientSecret: "s3cr3t",
      expiresInSeconds: 900,
    });
    process.env.TEMPER_M2M_CLIENT_ID = "tmpr_a";
    process.env.TEMPER_M2M_CLIENT_SECRET = "s3cr3t";
    process.env.TEMPER_M2M_TOKEN_URL = issuer.url;

    const { mintM2mToken } = await import("../agent/lib/temper-auth.js");
    const before = Date.now();
    const result = await mintM2mToken();

    expect(result.token).toBe("temper-as-token-1");
    expect(result.expiresAt).toBeGreaterThanOrEqual(before + 900_000);
  });

  it("falls back to the static TEMPER_TOKEN when no machine identity is configured", async () => {
    process.env.TEMPER_TOKEN = "dev-token";

    const { temperToken } = await import("../agent/lib/temper-auth.js");

    expect(await temperToken()).toBe("dev-token");
  });
});

describe("temperFetch", () => {
  it("re-mints ONCE on a 401 and retries with the fresh token", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    api = await startMockApi({ rejectFirst: 1 });
    process.env.TEMPER_M2M_CLIENT_ID = "tmpr_a";
    process.env.TEMPER_M2M_CLIENT_SECRET = "s3cr3t";
    process.env.TEMPER_M2M_TOKEN_URL = issuer.url;

    const { temperFetch } = await import("../agent/lib/temper-auth.js");
    const res = await temperFetch(api.url, { method: "POST", body: "{}" });

    expect(res.status).toBe(200);
    // The retry must carry a DIFFERENT token. Replaying the dead one would 401 again — and a test
    // that only asserted "two requests" would pass even then.
    expect(api.bearers).toEqual(["temper-as-token-1", "temper-as-token-2"]);
    expect(issuer.requests).toHaveLength(2);
  });

  it("gives up after ONE re-mint — a persistent 401 is a real authz failure, not an expiry", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    api = await startMockApi({ rejectFirst: 99 });
    process.env.TEMPER_M2M_CLIENT_ID = "tmpr_a";
    process.env.TEMPER_M2M_CLIENT_SECRET = "s3cr3t";
    process.env.TEMPER_M2M_TOKEN_URL = issuer.url;

    const { temperFetch } = await import("../agent/lib/temper-auth.js");
    const res = await temperFetch(api.url, { method: "POST", body: "{}" });

    expect(res.status).toBe(401);
    expect(api.bearers).toHaveLength(2);
  });

  it("does not re-mint on a 200 — the happy path mints exactly once", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    api = await startMockApi();
    process.env.TEMPER_M2M_CLIENT_ID = "tmpr_a";
    process.env.TEMPER_M2M_CLIENT_SECRET = "s3cr3t";
    process.env.TEMPER_M2M_TOKEN_URL = issuer.url;

    const { temperFetch } = await import("../agent/lib/temper-auth.js");
    const res = await temperFetch(api.url, { method: "GET" });

    expect(res.status).toBe(200);
    expect(issuer.requests).toHaveLength(1);
  });
});
```

- [ ] **Step 4: Run the tests to verify they fail**

```bash
cd packages/agent-workflows/steward && npm install && npm test
```

Expected: FAIL — `temperFetch` is not exported from `../agent/lib/temper-auth.js`.

- [ ] **Step 5: Rewrite the steward's auth lib**

Replace `packages/agent-workflows/steward/agent/lib/temper-auth.ts` entirely:

```ts
import { getToken } from "@vercel/connect";
import { BearerToken, ClientCredentials, type Credentials, type TokenResult } from "temper-ts";

import { fetchWithRetry, type RetryOptions } from "./fetch-retry.js";

/**
 * Machine-identity auth for reaching temper, shared by the MCP connection AND the code schedules so
 * the two can never drift on how they authenticate (they did: the schedules used a Connect-first
 * `temperToken()` while the connection used M2M-first `mintM2mToken`, and on the Auth0-fronted prod
 * instance the Connect connector has no M2M app behind it — so the schedules' REST fetches silently
 * failed while the MCP connection worked).
 *
 * The MINT itself lives in `temper-ts` (`ClientCredentials`), shared with the Ruby gem's
 * `Temper::Credentials` by way of one wire contract (tests/contracts/m2m-token-request.json). What
 * stays here is what is genuinely steward-specific: the env names, and the Vercel Connect / static
 * token strategies — eve and Vercel concepts with no business in a general-purpose client.
 *
 * Ordering is **machine-identity-first**, identical to what the connection declares:
 *   1. `TEMPER_M2M_CLIENT_ID` present → mint the agent's own token via the OAuth `client_credentials`
 *      grant. This is the production path, and it works against BOTH issuers a temper instance can be
 *      fronted by: an external IdP (`temper admin machine provision`, audience required) and temper's
 *      own AS (`temper admin machine issue`, a `tmpr_` client id, audience omitted).
 *   2. else `TEMPER_CONNECT_CONNECTOR` → a Vercel Connect app token (instances where that works).
 *   3. else `TEMPER_TOKEN` (the already-OAuth-obtained token that drives `eve dev`).
 */

let cached: Credentials | undefined;

/**
 * `TEMPER_M2M_AUDIENCE` is read but NOT required. Auth0 demands an audience; temper's own AS ignores
 * a request-supplied one entirely and mints with its server-side `AS_AUDIENCE`. So a temper-issued
 * (`tmpr_`) credential must be able to omit it — requiring it here is precisely what made this agent
 * unable to consume one.
 */
function credentials(): Credentials {
  if (cached !== undefined) {
    return cached;
  }

  const clientId = process.env.TEMPER_M2M_CLIENT_ID;
  if (clientId) {
    cached = new ClientCredentials({
      tokenUrl: requireEnv("TEMPER_M2M_TOKEN_URL"),
      clientId,
      clientSecret: requireEnv("TEMPER_M2M_CLIENT_SECRET"),
      audience: process.env.TEMPER_M2M_AUDIENCE || undefined,
    });
    return cached;
  }

  const connector = process.env.TEMPER_CONNECT_CONNECTOR;
  if (connector) {
    cached = {
      token: () => getToken(connector, { subject: { type: "app" } }),
      tokenResult: async () => ({
        token: await getToken(connector, { subject: { type: "app" } }),
        expiresAt: Number.POSITIVE_INFINITY,
      }),
      refresh: async () => ({
        token: await getToken(connector, { subject: { type: "app" } }),
        expiresAt: Number.POSITIVE_INFINITY,
      }),
    };
    return cached;
  }

  cached = new BearerToken(requireEnv("TEMPER_TOKEN"));
  return cached;
}

/**
 * The token + its ABSOLUTE expiry, handed straight to eve's `auth.getToken` by the MCP connection so
 * eve can refresh ahead of a 401. Name and shape are load-bearing — `connections/temper.ts` passes
 * this function itself as `getToken`.
 */
export async function mintM2mToken(): Promise<TokenResult> {
  return credentials().tokenResult();
}

/** A bearer token string for imperative temper REST/MCP `fetch`es from the code schedules. */
export async function temperToken(): Promise<string> {
  return credentials().token();
}

/**
 * `fetch` against temper, authenticated, with the 5xx cold-start retry AND a single re-mint on 401.
 *
 * The 401 branch is the fix for a bug the Ruby port documented against this very file: a schedule
 * resolves ONE token and then holds it across N parallel fetches, so a token that dies mid-tick
 * takes the tick down with it and nothing recovers. Refresh-ahead-of-expiry cannot help — the token
 * was live when it was checked. Temper's AS mints 900-second tokens by default, which makes a tick
 * outliving its token ordinary rather than exotic.
 *
 * Exactly ONE re-mint: a 401 that survives a fresh token is a real authorization failure (a revoked
 * credential, missing reach), and retrying it forever would only bury the error.
 */
export async function temperFetch(
  url: string,
  init: RequestInit,
  opts: RetryOptions = {},
): Promise<Response> {
  const creds = credentials();
  const headers = new Headers(init.headers);

  headers.set("authorization", `Bearer ${await creds.token()}`);
  const res = await fetchWithRetry(url, { ...init, headers }, opts);
  if (res.status !== 401) {
    return res;
  }

  const refreshed = await creds.refresh();
  console.log(`[temper-auth] ${opts.label ?? url} returned 401; re-minted and retrying once`);
  headers.set("authorization", `Bearer ${refreshed.token}`);
  return fetchWithRetry(url, { ...init, headers }, opts);
}

export function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is required — the steward's target/credential is never hardcoded`);
  }
  return value;
}
```

- [ ] **Step 6: Route the dispatch schedule through `temperFetch`**

Modify `packages/agent-workflows/steward/agent/schedules/steward.ts`. Change the import line:

```ts
import { requireEnv, temperFetch } from "../lib/temper-auth.js";
```

(`fetchWithRetry` is no longer imported here — `temperFetch` wraps it.) Then replace the token resolution and the dispatch fetch (currently lines 53-71) with:

```ts
          const apiUrl = requireEnv("TEMPER_API_URL").replace(/\/+$/, "");

          // Retry on 5xx: this hourly call always hits a cold serverless function, which can 500 on
          // a Neon pool-acquire timeout at startup; a retry warms it and succeeds. And re-mint on
          // 401: the token is resolved per-call now, so a tick outliving its token recovers instead
          // of dying — see temperFetch.
          const res = await temperFetch(
            `${apiUrl}/api/steward/dispatch`,
            {
              method: "POST",
              headers: {
                "content-type": "application/json",
                "x-steward-correlation-id": correlationId,
              },
              // Empty body → server defaults (ingest threshold + dispatch cap).
              body: "{}",
            },
            { label: "dispatch" },
          );
```

The `const token = await temperToken();` line is deleted, and the `authorization` header comes out of the `headers` object — `temperFetch` sets it.

- [ ] **Step 7: Route the materialize schedule through `temperFetch`**

Modify `packages/agent-workflows/steward/agent/schedules/materialize.ts`. Change the imports:

```ts
import { requireEnv, temperFetch } from "../lib/temper-auth.js";
```

(`fetchWithRetry` is no longer imported.) Replace the body of `materializeTick` (currently lines 43-77):

```ts
async function materializeTick(): Promise<void> {
  const apiUrl = requireEnv("TEMPER_API_URL").replace(/\/+$/, "");

  const list = await temperFetch(`${apiUrl}/api/steward/candidates`, {}, { label: "candidates" });
  if (!list.ok) {
    throw new Error(`candidates fetch failed: ${list.status} ${await list.text()}`);
  }
  const ids = (await list.json()) as string[];
  console.log(`[steward-materialize] materializing ${ids.length} candidate cogmap(s)`);

  await Promise.all(
    ids.map(async (id) => {
      const res = await temperFetch(
        `${apiUrl}/api/cognitive-maps/${id}/materialize`,
        {
          method: "POST",
          headers: { "content-type": "application/json" },
          // Empty body → the server applies its DEFAULT_MATERIALIZE_THRESHOLD (self-gating no-op below).
          body: "{}",
        },
        { label: `materialize ${id}` },
      );
      if (!res.ok) {
        throw new Error(`materialize ${id} POST failed: ${res.status} ${await res.text()}`);
      }
    }),
  );
}
```

Also update the auth paragraph in that file's header comment — it says "Auth is machine-identity-first via the shared `temperToken`"; it is now `temperFetch`, which additionally re-mints on 401.

- [ ] **Step 8: Run the tests and the typecheck**

```bash
cd packages/agent-workflows/steward && npm test && npm run typecheck && npm run build
```

Expected: all tests PASS; typecheck clean; `eve build` succeeds.

- [ ] **Step 9: Commit**

```bash
git add clients/temper-ts packages/agent-workflows/steward
git commit -m "fix(steward): compose temper-ts's mint — optional audience, re-mint on 401

The steward could not consume a temper-issued (tmpr_) credential at all:
requireEnv('TEMPER_M2M_AUDIENCE') threw, but such a client must OMIT audience.
And it resolved one token per tick and held it across N parallel fetches, so a
token dying mid-tick took the tick with it — the exact bug temper-rb's port
documented against this file, and which the AS's 900s TTL makes ordinary."
```

---

## Task 6: Config-driven model selection

eve executes `agent.ts` at **build** time and freezes the resolved model into the compiled manifest — so env is the only lever that exists, and a model change takes a redeploy. The primary is validated against the AI Gateway catalog at compile (a typo fails the build); the fallback list is not (a typo there fails at runtime, only when it is needed).

**Files:**
- Create: `packages/agent-workflows/steward/agent/lib/model-config.ts`
- Create: `packages/agent-workflows/steward/tests/model-config.test.ts`
- Modify: `packages/agent-workflows/steward/agent/agent.ts` (full rewrite)

**Interfaces:**
- Consumes: nothing.
- Produces: `interface ModelConfig { primary: string; fallbacks: string[] }`, `resolveModelConfig(env?: NodeJS.ProcessEnv): ModelConfig`, `DEFAULT_MODEL: string`, `DEFAULT_FALLBACKS: readonly string[]`.

- [ ] **Step 1: Write the failing test**

`packages/agent-workflows/steward/tests/model-config.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { DEFAULT_FALLBACKS, DEFAULT_MODEL, resolveModelConfig } from "../agent/lib/model-config.js";

describe("resolveModelConfig", () => {
  it("reproduces today's behavior when nothing is configured", () => {
    expect(resolveModelConfig({})).toEqual({
      primary: DEFAULT_MODEL,
      fallbacks: [...DEFAULT_FALLBACKS],
    });
  });

  it("takes the primary from STEWARD_MODEL", () => {
    expect(resolveModelConfig({ STEWARD_MODEL: "anthropic/claude-sonnet-5" }).primary).toBe(
      "anthropic/claude-sonnet-5",
    );
  });

  it("takes an ordered fallback list from STEWARD_MODEL_FALLBACKS", () => {
    expect(
      resolveModelConfig({
        STEWARD_MODEL_FALLBACKS: "anthropic/claude-haiku-4.5,openai/gpt-5.5",
      }).fallbacks,
    ).toEqual(["anthropic/claude-haiku-4.5", "openai/gpt-5.5"]);
  });

  it("trims whitespace and drops empty entries", () => {
    expect(
      resolveModelConfig({
        STEWARD_MODEL_FALLBACKS: " anthropic/claude-haiku-4.5 , , openai/gpt-5.5,",
      }).fallbacks,
    ).toEqual(["anthropic/claude-haiku-4.5", "openai/gpt-5.5"]);
  });

  // The gateway walks the list AFTER the primary fails. Leaving the primary in it would re-try the
  // model that just failed before reaching a model that might work.
  it("drops the primary out of its own fallback list", () => {
    expect(
      resolveModelConfig({
        STEWARD_MODEL: "minimax/minimax-m3",
        STEWARD_MODEL_FALLBACKS: "minimax/minimax-m3,anthropic/claude-haiku-4.5",
      }).fallbacks,
    ).toEqual(["anthropic/claude-haiku-4.5"]);
  });

  it("dedupes repeated fallbacks, keeping first-seen order", () => {
    expect(
      resolveModelConfig({
        STEWARD_MODEL_FALLBACKS: "openai/gpt-5.5,anthropic/claude-haiku-4.5,openai/gpt-5.5",
      }).fallbacks,
    ).toEqual(["openai/gpt-5.5", "anthropic/claude-haiku-4.5"]);
  });

  it("supports an explicitly empty fallback list", () => {
    expect(resolveModelConfig({ STEWARD_MODEL_FALLBACKS: "" }).fallbacks).toEqual([]);
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd packages/agent-workflows/steward && npx vitest run tests/model-config.test.ts
```

Expected: FAIL — cannot resolve `../agent/lib/model-config.js`.

- [ ] **Step 3: Implement the model config**

`packages/agent-workflows/steward/agent/lib/model-config.ts`:

```ts
/**
 * The steward's model, resolved from configuration rather than baked into source.
 *
 * eve executes `agent.ts` at BUILD time (`compileAgentConfig`) and freezes the resolved model into
 * the compiled manifest — there is no session, no request context, and no DB anywhere near that
 * resolution. Env is therefore the only lever that exists, and a model change takes a REDEPLOY, not
 * a restart. That also means the primary is validated against the AI Gateway catalog at compile
 * time: a typo fails the build rather than a 3am cron tick.
 *
 * The fallbacks are NOT so validated — they ride through the compile untouched inside
 * `providerOptions`, so a typo there surfaces at runtime, only when it is actually needed.
 */

/**
 * Distillation and supersession are judgment-heavy, and sonnet/opus 4.x are the quality bar — but
 * their per-tick cost is not sustainable at the dev/community tier, where the loop runs hourly and
 * confidence comes from running it enough to trust it keeps working. minimax-m3 is a sound model at
 * ~10x lower cost with a matching 1M context window. Enterprise deployments override this with
 * `STEWARD_MODEL`.
 */
export const DEFAULT_MODEL = "minimax/minimax-m3";

/** In-family, ~3x cheaper, and a known-good tool-caller — the safe landing if minimax is unavailable. */
export const DEFAULT_FALLBACKS = ["anthropic/claude-haiku-4.5"] as const;

export interface ModelConfig {
  primary: string;
  /**
   * Tried IN ORDER after the primary fails, by the AI Gateway itself
   * (`providerOptions.gateway.models`). This covers AVAILABILITY — a 5xx, a rate limit, a model that
   * is gone. It cannot cover QUALITY: no gateway can tell that a model fumbled a tool sequence. The
   * mechanism for that is changing `STEWARD_MODEL` and redeploying, which is what making this
   * configurable buys.
   */
  fallbacks: string[];
}

export function resolveModelConfig(env: NodeJS.ProcessEnv = process.env): ModelConfig {
  const primary = env.STEWARD_MODEL?.trim() || DEFAULT_MODEL;

  const raw = env.STEWARD_MODEL_FALLBACKS;
  const configured =
    raw === undefined
      ? [...DEFAULT_FALLBACKS]
      : raw
          .split(",")
          .map((entry) => entry.trim())
          .filter((entry) => entry !== "");

  // Dedupe, and drop the primary: the gateway walks this list only AFTER the primary fails, so
  // repeating it there just re-tries a model that has already failed.
  const fallbacks = [...new Set(configured)].filter((model) => model !== primary);

  return { primary, fallbacks };
}
```

- [ ] **Step 4: Run the test to verify it passes**

```bash
cd packages/agent-workflows/steward && npx vitest run tests/model-config.test.ts
```

Expected: PASS.

- [ ] **Step 5: Wire it into the agent definition**

Replace `packages/agent-workflows/steward/agent/agent.ts` entirely. The `modelOptions` helper returns `{}` when there is nothing to fall back to, so an empty list omits `gateway.models` rather than sending an empty array:

```ts
import { defineAgent } from "eve";

import { resolveModelConfig } from "./lib/model-config.js";

const model = resolveModelConfig();

/**
 * The AI Gateway tries the primary, then each fallback IN ORDER, returning the first that succeeds.
 * Omit the key entirely when there is nothing to fall back to — an empty list is not a policy.
 */
function modelOptions(fallbacks: string[]) {
  return fallbacks.length === 0
    ? {}
    : { providerOptions: { gateway: { models: fallbacks } } };
}

export default defineAgent({
  // Config-driven, resolved at BUILD time — see lib/model-config.ts for why env is the only lever
  // eve offers, and why a model change takes a redeploy. Defaults reproduce the previous hardcoded
  // behavior exactly, so a deploy with no new env set is a no-op.
  model: model.primary,
  modelOptions: modelOptions(model.fallbacks),
  description:
    "Team self-cognition steward: distills a team's own temper resources into cogmap-homed nodes and tends the team's cognitive map via the authored-4 (create/assert/facet/fold), audited by the invocation envelope.",
});
```

- [ ] **Step 6: Verify the build still resolves the model**

```bash
cd packages/agent-workflows/steward && npm run typecheck && npm run build
```

Expected: typecheck clean, and `eve build` succeeds — proving the Gateway catalog still resolves the primary through the env indirection.

Then prove the override reaches the build (a deliberately bogus id must FAIL the build — that is the build-time validation the design claims):

```bash
STEWARD_MODEL=anthropic/not-a-real-model npm run build
```

Expected: build FAILS with a message naming the unknown model. If it *succeeds*, the model is not being validated and the claim in the plan is wrong — investigate before proceeding.

- [ ] **Step 7: Commit**

```bash
git add packages/agent-workflows/steward
git commit -m "feat(steward): config-driven model with a Gateway fallback list

STEWARD_MODEL / STEWARD_MODEL_FALLBACKS, defaulting to today's model exactly.
eve resolves the model at BUILD time, so env is the only lever there is and a
change takes a redeploy. Fallback is availability-only: no gateway can detect a
fumbled tool sequence."
```

---

## Task 7: Make the new suites run in CI

Both new projects are outside the cargo workspace (`members = ["crates/*", "tests/e2e"]`) and outside the bun workspace (an explicit two-entry list), and the repo pre-commit never touches them. So a suite added there runs **nowhere** by default — the exact rot that let a 484-second test hide behind a green tick for months. This is path-scoped for the same reason `test-ruby` is, and the safety argument is identical: nothing outside the trigger set can reach these projects.

**Files:**
- Create: `.github/workflows/test-agents-ts.yml`
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/scripts/detect-ci-scope.sh`
- Modify: `.github/scripts/test-detect-ci-scope.sh`

**Interfaces:**
- Consumes: `clients/temper-ts`'s `test`/`typecheck` scripts (Tasks 3-4); the steward's `test`/`typecheck` scripts (Task 5).
- Produces: the `run-test-agents-ts` scope output and the `test-agents-ts` CI job.

- [ ] **Step 1: Write the failing scope test**

Add to `.github/scripts/test-detect-ci-scope.sh`:

```bash
run_test "temper-ts change runs the TS SDK + agent job" \
    "clients/temper-ts/src/credentials.ts" \
    "DOCS_ONLY=false" "RUN_TEST_AGENTS_TS=true"

run_test "steward change runs the TS SDK + agent job" \
    "packages/agent-workflows/steward/agent/agent.ts" \
    "DOCS_ONLY=false" "RUN_TEST_AGENTS_TS=true"

run_test "contract change runs the TS SDK + agent job (temper-ts asserts it)" \
    "tests/contracts/m2m-token-request.json" \
    "DOCS_ONLY=false" "RUN_TEST_AGENTS_TS=true"

run_test "an unrelated rust change does not run the TS SDK + agent job" \
    "crates/temper-api/src/main.rs" \
    "DOCS_ONLY=false" "RUN_TEST_AGENTS_TS=false"

run_test "docs-only skips the TS SDK + agent job" \
    "README.md" \
    "DOCS_ONLY=true" "RUN_TEST_AGENTS_TS=false"
```

- [ ] **Step 2: Run it to verify it fails**

```bash
bash .github/scripts/test-detect-ci-scope.sh
```

Expected: the five new cases FAIL with `RUN_TEST_AGENTS_TS: expected='true' actual=''` — the flag is not emitted yet.

- [ ] **Step 3: Add the scope flag**

Modify `.github/scripts/detect-ci-scope.sh`. After the `HAS_RUBY` block, add:

```bash
# TypeScript SDK + agent workflows: clients/temper-ts (the TS client) and
# packages/agent-workflows/** (the eve agents that consume it), plus the wire
# contracts both are asserted against.
#
# Path-scoped for exactly the reason test-ruby is: these projects are inert to
# both cargo (`members = ["crates/*", "tests/e2e"]`) and bun (an explicit
# two-entry `workspaces` list), so no Rust or TS change can reach them except
# through a contract, which is in the trigger set.
HAS_AGENTS_TS=false
if changes_match '^clients/temper-ts/|^packages/agent-workflows/|^tests/contracts/|^\.github/workflows/test-agents-ts\.yml$|^__force_full_ci__$'; then
    HAS_AGENTS_TS=true
fi
```

Then, in the `if [ "$DOCS_ONLY" = "true" ]` branch add `RUN_TEST_AGENTS_TS=false`, and in the `else` branch add:

```bash
    if [ "$HAS_AGENTS_TS" = "true" ] || [ "$HAS_SELF" = "true" ]; then
        RUN_TEST_AGENTS_TS=true
    else
        RUN_TEST_AGENTS_TS=false
    fi
```

Update the `SCOPE_SUMMARY` strings to mention it, add the `printf 'RUN_TEST_AGENTS_TS=%s\n' "$RUN_TEST_AGENTS_TS"` line to the stdout block, and add `echo "run-test-agents-ts=${RUN_TEST_AGENTS_TS}"` to the `$GITHUB_OUTPUT` block. Also extend the `debug` line to carry `HAS_AGENTS_TS`.

- [ ] **Step 4: Run the scope tests to verify they pass**

```bash
bash .github/scripts/test-detect-ci-scope.sh
```

Expected: all PASS, existing cases included.

- [ ] **Step 5: Add the workflow**

Create `.github/workflows/test-agents-ts.yml`. Node 24 matches both projects' `engines`. The steward's `pretest` builds the `file:` dependency, so temper-ts does not need a separate build step in that job.

```yaml
name: TypeScript SDK & Agent Tests

on:
  workflow_call:

jobs:
  temper-ts:
    name: temper-ts (typecheck, vitest)
    runs-on: ubuntu-latest
    timeout-minutes: 10

    defaults:
      run:
        working-directory: clients/temper-ts

    steps:
      - name: Checkout code
        uses: actions/checkout@v6

      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: "24"

      # npm, not bun: temper-ts is deliberately NOT a bun workspace member (the root list is exactly
      # temper-cloud + temper-ui), and a root install would inherit the root's overrides.
      - name: Install dependencies
        run: npm ci

      - name: Type-check
        run: npm run typecheck

      - name: Run tests
        run: npm test

  steward:
    name: steward agent (typecheck, vitest)
    runs-on: ubuntu-latest
    timeout-minutes: 10

    defaults:
      run:
        working-directory: packages/agent-workflows/steward

    steps:
      - name: Checkout code
        uses: actions/checkout@v6

      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: "24"

      # The steward takes temper-ts as a `file:` dependency, so npm symlinks it and its `pretest`
      # hook builds it. Run from INSIDE this directory — a root install fails on the root's bun
      # overrides.
      - name: Install dependencies
        run: npm install

      - name: Type-check
        run: npm run typecheck

      - name: Run tests
        run: npm test
```

- [ ] **Step 6: Wire the job into the pipeline**

Modify `.github/workflows/ci.yml`:

1. Add to `detect-scope`'s `outputs`:

```yaml
      run-test-agents-ts: ${{ steps.detect.outputs.run-test-agents-ts }}
```

2. Add the job after `test-ruby`:

```yaml
  # Path-scoped: runs only for clients/temper-ts/**, packages/agent-workflows/**,
  # tests/contracts/**, and its own workflow. Both projects are inert to cargo and
  # to bun, so nothing else can reach them. See detect-ci-scope.sh.
  test-agents-ts:
    needs: detect-scope
    if: needs.detect-scope.outputs.run-test-agents-ts == 'true'
    uses: ./.github/workflows/test-agents-ts.yml
```

3. Add `test-agents-ts` to `ci-success`'s `needs` list, and add its gate check next to the others:

```bash
          check_job "test-agents-ts" \
            "${{ needs.test-agents-ts.result }}" \
            "${{ needs.detect-scope.outputs.run-test-agents-ts }}"
```

- [ ] **Step 7: Commit and push**

```bash
git add .github
git commit -m "ci: run the TS SDK + agent suites — a test that runs nowhere is not a test

clients/temper-ts and packages/agent-workflows/** are outside both the cargo and
bun workspaces and untouched by pre-commit, so their suites would run nowhere by
default. Path-scoped on the same safety argument as test-ruby."
git push
```

Expected: on the PR, `test-agents-ts` runs (both jobs green), `test-ruby` runs (the contract changed), and `ci-success` is green.

---

## Task 8: Documentation + final verification

**Files:**
- Modify: `docs/guides/machine-credentials.md:60-64`
- Modify: `packages/agent-workflows/steward/CLAUDE.md`

**Interfaces:**
- Consumes: everything above.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Point the guide at the third client**

Modify `docs/guides/machine-credentials.md`. The paragraph beginning *"Temper's own clients read these from `TEMPER_M2M_TOKEN_URL`…"* says "the same four names in the Ruby gem and the steward runtime". Replace that paragraph:

```markdown
Temper's own clients read these from `TEMPER_M2M_TOKEN_URL`, `TEMPER_M2M_CLIENT_ID`,
`TEMPER_M2M_CLIENT_SECRET`, and (external IdP only) `TEMPER_M2M_AUDIENCE` — the same four names in
the Ruby gem (`Temper::Credentials`), the TypeScript client (`temper-ts`'s `ClientCredentials`), and
the steward runtime, which composes the TypeScript one. Follow the convention; it is one less thing
to translate.
```

Also update the `$comment` "Consumed by" list already added in Task 2 if any path changed, and add `clients/temper-ts/tests/contract.test.ts` to the **See also** section's contract bullet:

```markdown
- **The cross-language wire contract:** `tests/contracts/m2m-token-request.json` — pin any new client
  against it. Pinned today by `temper-rb` (`spec/temper/credentials_spec.rb`), `temper-ts`
  (`tests/contract.test.ts`), and the AS itself
  (`packages/temper-cloud/tests/integration/oauth/client-credentials.test.ts`).
```

- [ ] **Step 2: Record the steward's new shape**

Modify `packages/agent-workflows/steward/CLAUDE.md`. Replace its contents:

```markdown
> This is the Temper team-self-cognition **steward** — an Eve agent. Design: docs/superpowers/specs/2026-07-01-t5-eve-steward-agent-directory-design.md. It is a workspace-isolated Eve project; run tooling from THIS directory, not the repo root.

**Auth:** the M2M mint lives in `temper-ts` (`ClientCredentials`), taken as an npm `file:`
dependency — a deliberate bridge until temper-ts publishes, at which point the dependency becomes a
normal version range and the Vercel "include files outside the Root Directory" setting can go back
off. `agent/lib/temper-auth.ts` holds only what is steward-specific: the env names and the Vercel
Connect / static-token strategies. Reach for `temperFetch`, never a bare `fetch` — it carries the
5xx cold-start retry AND the single re-mint on 401.

**Model:** configured via `STEWARD_MODEL` / `STEWARD_MODEL_FALLBACKS` (`agent/lib/model-config.ts`).
eve resolves the model at BUILD time, so a change takes a **redeploy**, not a restart. The fallback
list is the AI Gateway's own (`providerOptions.gateway.models`) and covers availability, never
quality.

**Tests:** `npm test` (vitest, `tests/`). They run in CI via `.github/workflows/test-agents-ts.yml`.

@AGENTS.md
```

- [ ] **Step 3: Full local verification**

```bash
cd clients/temper-ts && npm ci && npm run typecheck && npm test && npm run build
cd ../../packages/agent-workflows/steward && npm install && npm run typecheck && npm test && npm run build
cd ../../.. && bash .github/scripts/test-detect-ci-scope.sh
cd packages/temper-cloud && bun run test:integration -- oauth/client-credentials
cd ../../clients/temper-rb && bundle exec rspec spec/temper/credentials_spec.rb
```

Expected: every command green.

- [ ] **Step 4: Confirm the deployed agent still authenticates**

The steward's prod env has `TEMPER_M2M_AUDIENCE` set (Auth0-fronted), so the audience is still sent and behavior is unchanged. After the PR merges and `steward-agent` redeploys, check the next hourly tick's logs:

```bash
vercel logs steward-agent
```

Expected: `[steward-dispatch] tick <uuid> starting (temper-ts 0.0.0)` followed by a claimed-jobs line — the dependency resolved in the deployed bundle, and the dispatch call authenticated.

- [ ] **Step 5: Commit**

```bash
git add docs/guides/machine-credentials.md packages/agent-workflows/steward/CLAUDE.md
git commit -m "docs: temper-ts is the third client pinned to the M2M wire contract"
git push
```

---

## Self-Review

**Spec coverage:**

| Spec section | Task |
|---|---|
| 1. `clients/temper-ts` seeded with credentials module | 1 (package), 4 (module) |
| 2. The faithful mock issuer | 3 (issuer), 5 (mock API for the 401 path) |
| 3. Steward composes the module | 5 |
| 4. Consumption: `file:` dep + Vercel toggle | 1 (probe gate) |
| 5. Contract gains a response section | 2 |
| 6. Model selection | 6 |
| 7. CI — a test that runs nowhere is not a test | 7 |
| Verification: probe deploy shape FIRST | 1, Step 9 (explicit GATE) |
| Verification: regression on gem + AS suites | 2 (Steps 6-7), 8 (Step 3) |
| Verification: prod is a no-op | 8 (Step 4) |

One item the spec did not anticipate and this plan adds: **`tests/contracts/` was missing from `test-ruby`'s trigger set** (Task 2, Steps 2-4), so a contract change never ran the gem spec asserting it. Discovered while reading `detect-ci-scope.sh`; fixed here because this PR changes that contract.

**Placeholder scan:** none. Every code step carries complete code; every command carries expected output.

**Type consistency:** `TokenResult { token, expiresAt }` is defined in Task 4 and consumed by Task 5's `mintM2mToken` return type. `Credentials { token, tokenResult, refresh }` is implemented by `BearerToken`, `ClientCredentials`, and the steward's inline Connect strategy — all three supply all three methods. `startMockIssuer`/`startMockApi` signatures in Tasks 3 and 5 match their call sites in Tasks 3, 4, and 5. `RetryOptions` is imported from the existing `fetch-retry.ts`, where it is already exported. `mintM2mToken` keeps its name because `agent/connections/temper.ts:34` passes the function itself to eve — that file is deliberately not modified.
