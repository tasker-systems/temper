# T5 — Eve Steward Agent Directory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Author the Vercel/Eve steward agent directory that binds the shipped temper-mcp tool surface to the Eve runtime, so the team-self-cognition steward can be booted, typechecked, and proven against a live temper-mcp deployment with one real read.

**Architecture:** A self-contained Eve project at `packages/agent-workflows/steward/` (scaffolded by `eve init`, keeping Eve's own toolchain — TS 7 RC, `ai` v7, `@vercel/connect`) that is deliberately **not** a bun-workspace member, so it never collides with `temper-cloud`'s TS 5.8 or the repo pre-commit. One `defineMcpClientConnection` to an env-driven temper-mcp URL exposes a 24-tool allow-list scoped to the steward persona; the persona lives in `instructions.md` (always-on) plus a `map-stewardship.md` skill (on-demand). A cron `defineSchedule` is the backstop; real dispatch is T6.

**Tech Stack:** Eve `^0.18.1` (`eve`, `eve/connections`, `eve/channels/*`, `eve/tools`), `@vercel/connect` `0.2.2`, `ai` `^7`, `zod` `4.4.3`, TypeScript `7.0.1-rc`, Node `24.x`, npm (Eve scaffolds `package-lock.json`). Backend: the live temper-mcp server (`https://temperkb.io/mcp` or a self-hosted URL) — a full OAuth 2.0 server (Auth0 discovery, PKCE, dynamic client registration).

## Global Constraints

- **Eve project is isolated from the bun workspace.** Do NOT add `packages/agent-workflows/**` to the root `package.json` `workspaces` array. It keeps its own `package.json` + `package-lock.json` + `node_modules` + `tsconfig.json`. The repo pre-commit (`githooks/pre-commit`) only `cd packages/temper-cloud` for TS/biome — it must stay that way.
- **Env-driven target, never hardcoded.** The temper-mcp URL comes from `process.env.TEMPER_MCP_URL`; any audience/issuer that varies by deployment is env- or discovery-sourced. One agent dir must point at temperkb.io OR the self-hosted instance by env value alone.
- **No static token as the design auth.** Production auth is platform/OAuth-carried (`defineInteractiveAuthorization` PKCE → temper-mcp Auth0, or `@vercel/connect`). A cached dev token may be used ONLY to exercise the one-real-read verification, never as the committed auth strategy.
- **Steward allow-list is exactly the 24 tools in Task 2.** The 9 excluded (`cogmap_shape`, `cogmap_analytics`, `cogmap_region_metrics`, `cogmap_create`, `cogmap_bind`, `cogmap_unbind`, `cogmap_grant`, `cogmap_revoke`, `create_context`) stay out — region reads violate the determinism reframe; genesis/admin/access belong to other agent roles.
- **Steward behavior invariants (from T3), encoded in prose not code:** declares structure, never clusters or assigns salience; supersedes via `fold`, never in-place edit; wraps every run in the invocation envelope; stamps every act with `AgentAuthorship` (reasoning required on create/edge/fold).
- **Eve dev verification uses `--no-ui` in the background.** Bare `eve dev` opens a REPL and must not be used as a background process. Use `npm exec -- eve dev --no-ui`, wait for the server URL, exercise the HTTP API, then stop it.
- **Do NOT commit Eve's `node_modules`, `.eve`, `.vercel`, `.workflow-data`** (the scaffold `.gitignore` covers these — keep it).

**Spec deviation note:** design D1 said "biome extends root, tsconfig mirrors temper-cloud." The real `eve init` scaffold brings its own TS 7 tsconfig and no biome; keeping Eve's scaffolded toolchain (and isolating the package from the workspace) is the correct seam. Task 6 updates the spec's D1 line to match.

---

## File structure

```
packages/agent-workflows/
├── README.md                       # Task 6 — why this package is workspace-isolated; deploy notes
└── steward/                        # self-contained Eve project (eve init output)
    ├── package.json                # Eve's own (Task 1); NOT a workspace member
    ├── package-lock.json
    ├── tsconfig.json               # Eve's own (TS 7)
    ├── .gitignore  .vercelignore   # from scaffold
    ├── AGENTS.md  CLAUDE.md         # scaffold guidance — trim in Task 1
    └── agent/
        ├── agent.ts                # Task 1 — defineAgent({ model, description })
        ├── instructions.md         # Task 3 — always-on steward persona
        ├── channels/eve.ts         # from scaffold (keep) — entry point
        ├── connections/temper.ts   # Task 2 — MCP binding: env URL, PKCE auth, 24-tool allow-list
        ├── skills/map-stewardship.md   # Task 4 — on-demand procedure (D3/D5/D6/D7/D8 + loop)
        └── schedules/steward.ts    # Task 5 — defineSchedule cron backstop
```

---

## Task 1: Scaffold the isolated Eve project + agent definition

**Files:**
- Create (via `eve init`): `packages/agent-workflows/steward/**` (package.json, tsconfig.json, agent/agent.ts, agent/channels/eve.ts, agent/instructions.md, .gitignore, .vercelignore, AGENTS.md, CLAUDE.md)
- Modify: `packages/agent-workflows/steward/agent/agent.ts` (set model + description)
- Do NOT modify: root `package.json` (must stay `workspaces: ["packages/temper-cloud", "packages/temper-ui"]`)

**Interfaces:**
- Produces: a booting Eve project whose default export in `agent/agent.ts` is `defineAgent({ model, description })`; later tasks add sibling files under `agent/`.

- [ ] **Step 1: Scaffold into the package path**

Run from repo root:
```bash
mkdir -p packages/agent-workflows
npx eve@latest init packages/agent-workflows/steward
```
Expected: scaffold prints the file tree (package.json, tsconfig.json, agent/agent.ts, agent/channels/eve.ts, agent/instructions.md, AGENTS.md, CLAUDE.md, .gitignore, .vercelignore) and installs deps (creates `node_modules` + `package-lock.json`).

- [ ] **Step 2: Confirm the package is isolated from the workspace**

Run:
```bash
grep -n "agent-workflows" package.json || echo "OK: not a workspace member"
```
Expected: `OK: not a workspace member`. If it appears, remove it from the root `workspaces` array.

- [ ] **Step 3: Set the agent model + description**

Replace `packages/agent-workflows/steward/agent/agent.ts` with:
```ts
import { defineAgent } from "eve";

export default defineAgent({
  // Judgment-heavy distillation + supersession; sonnet-5 is the scaffold default.
  // Bump to a larger model here if synthesis quality warrants it.
  model: "anthropic/claude-sonnet-5",
  description:
    "Team self-cognition steward: distills a team's own temper resources into cogmap-homed nodes and tends the team's cognitive map via the authored-4 (create/assert/facet/fold), audited by the invocation envelope.",
});
```

- [ ] **Step 4: Trim scaffold guidance files**

The scaffold drops `AGENTS.md` and `CLAUDE.md` with generic Eve guidance. Keep them (they help future editors) but prepend a one-line pointer at the top of `packages/agent-workflows/steward/CLAUDE.md`:
```md
> This is the Temper team-self-cognition **steward** — an Eve agent. Design: docs/superpowers/specs/2026-07-01-t5-eve-steward-agent-directory-design.md. It is a workspace-isolated Eve project; run tooling from THIS directory, not the repo root.
```

- [ ] **Step 5: Verify typecheck passes**

Run:
```bash
cd packages/agent-workflows/steward && npm run typecheck
```
Expected: `tsc` exits 0, no errors.

- [ ] **Step 6: Verify the agent boots**

Run (background, no UI):
```bash
cd packages/agent-workflows/steward && npm exec -- eve dev --no-ui > /tmp/eve-boot.log 2>&1 &
```
Wait ~15s, then:
```bash
grep -iE "http://127.0.0.1|listening|ready|error" /tmp/eve-boot.log | head
```
Expected: a local server URL (e.g. `http://127.0.0.1:3000`) and no fatal error. Then stop it:
```bash
pkill -f "eve dev" || true
```

- [ ] **Step 7: Commit**

```bash
git add packages/agent-workflows/steward
git commit -m "feat(steward): scaffold isolated Eve project for team-self-cognition steward (T5)"
```

---

## Task 2: temper-mcp connection — env URL, PKCE auth, 24-tool allow-list

**Files:**
- Create: `packages/agent-workflows/steward/agent/connections/temper.ts`

**Interfaces:**
- Consumes: `defineMcpClientConnection` from `eve/connections`; `process.env.TEMPER_MCP_URL`.
- Produces: an MCP connection registered as `temper` (tools callable by the model as `temper__<tool>`), exposing exactly the 24-tool allow-list.

- [ ] **Step 1: Write the connection with the allow-list and env URL**

Create `packages/agent-workflows/steward/agent/connections/temper.ts`:
```ts
import { defineMcpClientConnection } from "eve/connections";
import { temperAuth } from "#connections/temper-auth.ts";

/**
 * The steward's sole seam to temper-mcp. One env-driven URL (temperkb.io OR a
 * self-hosted instance), platform/OAuth-carried auth, and a 24-tool allow-list
 * scoped to the steward persona. The 9 excluded tools (region reads +
 * genesis/admin/access) are role-inappropriate for a steward — see the design.
 */
export default defineMcpClientConnection({
  url: requireEnv("TEMPER_MCP_URL"),
  description:
    "Temper knowledge base: the team's own resources (the steward's ingest source) and the team cognitive map it tends. Authored-4 writes, the invocation envelope, and the steward ingest-delta live here.",
  auth: temperAuth,
  tools: {
    allow: [
      // Authored-4
      "create_resource",
      "assert_relationship",
      "facet_set",
      "fold_relationship",
      // Invocation envelope
      "invocation_open",
      "invocation_close",
      "invocation_show",
      "invocation_list",
      // Steward delta / watermark
      "steward_ingest_delta",
      "steward_advance_watermark",
      // Reads
      "search",
      "get_resource",
      "get_context",
      "list_contexts",
      "list_resources",
      "cogmap_read_charter",
      "describe_doc_type",
      "list_doc_types",
      "get_profile",
      // Mutations (delete_resource is soft-delete: flips is_active via resource_deleted event)
      "update_resource",
      "update_resource_meta",
      "delete_resource",
      "retype_relationship",
      "reweight_relationship",
    ],
  },
});

function requireEnv(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`${name} is required (temper-mcp target URL; not hardcoded)`);
  return v;
}
```

- [ ] **Step 2: Write the auth strategy (production PKCE; dev-token escape hatch for verification)**

Create `packages/agent-workflows/steward/agent/connections/temper-auth.ts`:
```ts
import { defineInteractiveAuthorization, ConnectionAuthorizationRequiredError } from "eve/connections";

/**
 * Production auth is PKCE against temper-mcp's own OAuth server (Auth0 discovery
 * at TEMPER_MCP_URL/.well-known/oauth-authorization-server). No static secret in
 * code; authorize once, cache + refresh. The TEMPER_DEV_TOKEN branch exists ONLY
 * to exercise the one-real-read verification (Task 2 Step 4 / Task 6) and must
 * not become the committed default — the design commits to platform-carried auth.
 */
export const temperAuth = process.env.TEMPER_DEV_TOKEN
  ? { getToken: async () => ({ token: process.env.TEMPER_DEV_TOKEN! }) }
  : defineInteractiveAuthorization<{ verifier: string }>({
      getToken: async ({ principal }) => {
        const token = await lookupCachedToken(principal);
        if (!token) throw new ConnectionAuthorizationRequiredError("temper");
        return { token };
      },
      startAuthorization: async ({ callbackUrl }) => {
        const verifier = makePkceVerifier();
        return {
          challenge: { url: await buildAuthorizeUrl(callbackUrl, verifier) },
          resume: { verifier },
        };
      },
      completeAuthorization: async ({ resume, callback }) => {
        const token = await exchangeCode(resume!.verifier, callback.params.code!);
        return { token };
      },
    });
```

> **Implementation note (beta API):** `defineInteractiveAuthorization`'s exact generic/return shape is verified against `eve.dev/docs/connections` as of 2026-07-01 but Eve is beta — confirm the `startAuthorization`/`completeAuthorization` field names against the installed `eve` package's `.d.ts` before finalizing, and implement `makePkceVerifier` / `buildAuthorizeUrl` / `exchangeCode` / `lookupCachedToken` against temper-mcp's discovery document (`GET ${TEMPER_MCP_URL}/.well-known/oauth-authorization-server` → `authorization_endpoint`, `token_endpoint`; dynamic client registration at `/oauth/register`). The temper CLI already implements this exact Auth0 PKCE flow — mirror it. If the full PKCE helper set balloons past this session, keep the `TEMPER_DEV_TOKEN` branch for the live-read gate and land the hardened interactive flow in T6, per the spec's risk section.

- [ ] **Step 3: Verify typecheck passes**

Run:
```bash
cd packages/agent-workflows/steward && npm run typecheck
```
Expected: exits 0. (If `defineInteractiveAuthorization` field names differ from the installed `.d.ts`, fix per the implementation note, then re-run.)

- [ ] **Step 4: Verify ONE real MCP read against a live deployment**

Obtain a real temper token (reuse the temper CLI's cached token; the CLI already runs the Auth0 device/PKCE flow):
```bash
# Locate the cached token the temper CLI stores; export it for the dev-token branch.
# (Path per temper-client token cache; e.g. ~/.config/temper/… — confirm at runtime.)
export TEMPER_MCP_URL="https://temperkb.io/mcp"
export TEMPER_DEV_TOKEN="<cached temper JWT>"
cd packages/agent-workflows/steward && npm exec -- eve dev --no-ui > /tmp/eve-read.log 2>&1 &
```
Wait for the server URL, then drive one read through a session (ask the agent to call a read tool):
```bash
SID=$(curl -s -D - -X POST http://127.0.0.1:3000/eve/v1/session \
  -H 'content-type: application/json' \
  -d '{"message":"Call temper__list_contexts and report the raw result."}' \
  | tr -d '\r' | awk -F': ' '/x-eve-session-id/{print $2}')
curl -s "http://127.0.0.1:3000/eve/v1/session/$SID/stream" | head -c 4000
```
Expected: the stream shows a `temper__list_contexts` (or `temper__cogmap_read_charter`) tool call returning real data from temperkb.io. Capture the output as the verification artifact, then:
```bash
pkill -f "eve dev" || true
```

> If the model-driven session path is impractical in this environment (AI Gateway auth, headless friction), fall back to a direct MCP probe: with the same token, `curl` the MCP endpoint's tool-call for `list_contexts` to prove the connection target + auth resolve. Record which path was used.

- [ ] **Step 5: Commit**

```bash
git add packages/agent-workflows/steward/agent/connections
git commit -m "feat(steward): temper-mcp connection — env URL, PKCE auth, 24-tool allow-list (T5)"
```

---

## Task 3: instructions.md — always-on steward persona

**Files:**
- Modify: `packages/agent-workflows/steward/agent/instructions.md` (replace scaffold placeholder)

**Interfaces:**
- Consumes: the `temper__*` tools from Task 2 (referenced by name in prose).
- Produces: the always-on system prompt; points at the `map-stewardship` skill (Task 4) for the detailed procedure.

- [ ] **Step 1: Write the persona**

Replace the file contents with:
```md
# Identity

You are the **team self-cognition steward**. Your charge is to keep one cognitive
map — the team's self-cognition map — a faithful, current distillation of the
team's own work, drawn only from the team's own temper resources.

You operate under the map's **telos**. Read it first, every run
(`temper__cogmap_read_charter`), and let it decide what is worth distilling.

## What you do

Each run, over the resources that are new or changed since your last run
(`temper__steward_ingest_delta`), you tend the map with the **authored-4**:

- **create** cogmap-homed nodes that distill sources (`temper__create_resource`),
- **assert** edges — provenance (`derived_from`) and inter-node relationships
  (`temper__assert_relationship`),
- **set facets** on nodes (`temper__facet_set`),
- **fold** nodes whose source has been materially superseded
  (`temper__fold_relationship`).

Before creating, always **search** the map for an existing node covering the same
source or idea (`temper__search`) — dedup, don't duplicate.

## What you never do

- **You never cluster or assign salience.** Regions and their weights are the
  substrate's job, formed by materialization. You declare structure — nodes,
  edges, facets — and let regions emerge. Do not reason about regions.
- **You never edit a node in place to reflect a changed source.** Supersession is
  by **fold-then-recreate**, which preserves history. In-place reconciliation is
  out of scope.
- **You never reach beyond the team's own resources.** Your reads are
  access-bounded; treat anything you cannot read as out of scope, not an error.
- **You never create, bind, or grant on maps or contexts.** Those are not your
  tools and not your role.

## Discipline

Wrap every run in the invocation envelope: `temper__invocation_open` at the start,
`temper__invocation_close` with an outcome at the end. Stamp every act with your
authorship — confidence (tentative/probable/confident) and reasoning; reasoning is
**required** on every create, edge, and fold.

When you need the detailed method — how to choose a node's label, how to size its
granularity, which edge kind to use, or how to judge "materially changed" — load
the **map-stewardship** skill.
```

- [ ] **Step 2: Verify boot still loads instructions**

Run:
```bash
cd packages/agent-workflows/steward && npm run typecheck && npm exec -- eve dev --no-ui > /tmp/eve-instr.log 2>&1 &
sleep 12; grep -iE "127.0.0.1|error" /tmp/eve-instr.log | head; pkill -f "eve dev" || true
```
Expected: boots clean (instructions.md is prose; no typecheck impact).

- [ ] **Step 3: Commit**

```bash
git add packages/agent-workflows/steward/agent/instructions.md
git commit -m "feat(steward): always-on steward persona in instructions.md (T5)"
```

---

## Task 4: map-stewardship skill — on-demand procedure

**Files:**
- Create: `packages/agent-workflows/steward/agent/skills/map-stewardship.md`

**Interfaces:**
- Consumes: nothing new at runtime; encodes T3 decisions D3/D5/D6/D7/D8 as prose.
- Produces: an auto-discovered Eve skill (progressive disclosure via `load_skill`); the description line is what the model matches on.

- [ ] **Step 1: Write the skill**

Create the file. (Eve auto-discovers `agent/skills/*.md` and exposes the description; confirm the description-frontmatter format against the installed Eve docs — a leading `# Title` + first line, or YAML `description:`. Use YAML frontmatter and adjust if Eve expects otherwise.)
```md
---
description: The detailed method for tending the team self-cognition map — how to choose a node's label, size its granularity, pick an edge kind, judge "materially changed", and stamp authorship. Load when creating or superseding nodes.
---

# Map stewardship

## The loop

```
delta = temper__steward_ingest_delta(cogmap, threshold)   # skip if under threshold
inv   = temper__invocation_open(cogmap, trigger="scheduled")
telos = temper__cogmap_read_charter(cogmap)               # orient
for source in delta.new_or_changed:
  existing = temper__search(cogmap, source)               # dedup
  if materially_changed(source, existing):                # your judgment (below)
    temper__fold_relationship(existing.derived_from); existing = none
  if not existing:
    node = temper__create_resource(cogmap=cogmap, type=<label>, authorship=…)
    temper__assert_relationship(node -> source, label="derived_from", kind="express")
    for rel in inter_node_relationships(node):
      temper__assert_relationship(node -> other, kind, polarity, label, weight)
    for f in facets(node): temper__facet_set(node, f)
temper__steward_advance_watermark(cogmap, delta.max_event_id)
temper__invocation_close(inv, outcome)
```

## Choosing a node label (D3)

Per-source labels cite ~one source; synthesized labels span many (see granularity).

| Label | Kind | Use it for |
|-------|------|-----------|
| `fact` | per-source | An observation distilled from one resource ("the team uses pgvector"). |
| `memory` | per-source | A lesson/regulation carried forward ("run test-e2e-embed before context pushes"); often scar-linked. |
| `decision` | per-source | A settled choice. |
| `concept` | synthesized | A distilled idea spanning sources. |
| `question` | synthesized | An open question-with-context ("how should access RBAC work?"). |
| `theme` | synthesized | A higher-order cluster — "what they work on". |
| `concern` | synthesized | A live tension or risk the team holds. |
| `principle` | synthesized | A guiding tenet the team operates by. |
| `commitment` | synthesized | Something the team has committed to / owes. |
| `domain` | synthesized | An area of expertise / responsibility the team owns. |

If none fit, pass your best short label through as-is — the vocabulary has an open
tail. Prefer a recognized label when one is honest. `concern` vs `question`:
a concern is a held tension, a question is an open ask. `concept` vs `theme`:
a theme is broader, organizing many concepts.

## Granularity (D5)

- **Per-source** (`fact`/`memory`/`decision`): one node cites ~one source — a single
  `derived_from` edge.
- **Synthesized** (`concept`/`question`/`theme`/`concern`/`principle`/`commitment`/
  `domain`): one node spans many sources — many `derived_from` edges into it.

## Edge conventions (D6)

The structural `edge_kind` carries affinity; the free-text `label` carries meaning.

| Semantic label | edge_kind | polarity | Use |
|----------------|-----------|----------|-----|
| `derived_from` | `express` | forward | node ← source provenance (every node). |
| `relates_to` | `near` | forward | symmetric affinity between nodes. |
| `part_of` | `contains` | inverse | whole–part. |
| `answers` | `leads_to` | forward | a fact/concept answers a question. |
| `supports` / `contradicts` | `leads_to` | forward / inverse | stance between nodes. |

## "Materially changed" (D7)

Read the changed source against the existing node. It is **materially changed** if
the distillation would now say something different — a new claim, a reversed
decision, a dropped commitment — not if the source merely got a typo fix or a
reworded sentence. When materially changed: **fold** the stale node's
`derived_from` edge and create a fresh node. Never edit the node's blocks in place.
When in doubt, prefer leaving the node and lowering your confidence over churning
a fold.

## Authorship (D8)

Every act carries your authorship on the wire: `confidence` (tentative / probable /
confident) and `reasoning`. Reasoning is **required** on create, edge, and fold;
optional on facets. The `invocation_id` from `invocation_open` correlates the run.
Close with an outcome summarizing nodes / edges / facets / folds.
```

- [ ] **Step 2: Verify the skill is discovered**

Run:
```bash
cd packages/agent-workflows/steward && npm run typecheck && npm exec -- eve dev --no-ui > /tmp/eve-skill.log 2>&1 &
sleep 12; grep -iE "127.0.0.1|skill|map-stewardship|error" /tmp/eve-skill.log | head; pkill -f "eve dev" || true
```
Expected: boots clean; if Eve logs discovered skills, `map-stewardship` appears. (If Eve rejects the frontmatter format, adjust to the format the installed docs specify and re-run.)

- [ ] **Step 3: Commit**

```bash
git add packages/agent-workflows/steward/agent/skills/map-stewardship.md
git commit -m "feat(steward): map-stewardship skill — labels, edges, granularity, materially-changed, authorship (T5)"
```

---

## Task 5: schedule backstop

**Files:**
- Create: `packages/agent-workflows/steward/agent/schedules/steward.ts`

**Interfaces:**
- Consumes: `defineSchedule` from `eve/schedules` (confirm import path against installed package).
- Produces: a UTC cron backstop whose action prompts the steward loop. `eve dev` does not fire schedules on cadence.

- [ ] **Step 1: Write the schedule**

Create `packages/agent-workflows/steward/agent/schedules/steward.ts`:
```ts
import { defineSchedule } from "eve/schedules";

/**
 * Cron BACKSTOP only. The real threshold-driven dispatch (steward_ingest_delta
 * gate; the eve-vs-temper scheduler switch) is T6. The loop itself no-ops when
 * the ingest delta is under threshold, so a fixed cadence is safe.
 */
export default defineSchedule({
  cron: "0 * * * *", // hourly, UTC; the loop self-gates on the ingest threshold
  markdown:
    "Run one steward tick over the team self-cognition map: check the ingest " +
    "delta, and if it clears the threshold, tend the map per the map-stewardship " +
    "skill (open the invocation envelope, read the telos, distill new/changed " +
    "sources with the authored-4, then close and advance the watermark). If the " +
    "delta is under threshold, close the envelope with a no-op outcome.",
});
```

> **Implementation note:** confirm `defineSchedule`'s import path (`eve/schedules`) and whether the action key is `markdown` or `run` against the installed package `.d.ts` / `eve.dev/docs/schedules`. Adjust if the beta API differs.

- [ ] **Step 2: Verify typecheck + boot**

Run:
```bash
cd packages/agent-workflows/steward && npm run typecheck && npm exec -- eve dev --no-ui > /tmp/eve-sched.log 2>&1 &
sleep 12; grep -iE "127.0.0.1|schedule|error" /tmp/eve-sched.log | head; pkill -f "eve dev" || true
```
Expected: typecheck exits 0; boots clean.

- [ ] **Step 3: Commit**

```bash
git add packages/agent-workflows/steward/agent/schedules/steward.ts
git commit -m "feat(steward): hourly cron backstop (real dispatch → T6) (T5)"
```

---

## Task 6: Package README, spec reconciliation, final verification

**Files:**
- Create: `packages/agent-workflows/README.md`
- Modify: `docs/superpowers/specs/2026-07-01-t5-eve-steward-agent-directory-design.md` (D1 toolchain line)
- Modify: `CLAUDE.md` (one line registering the new package + its isolation)

**Interfaces:**
- Consumes: everything from Tasks 1–5.
- Produces: the integration story a future editor needs.

- [ ] **Step 1: Write the package README**

Create `packages/agent-workflows/README.md`:
```md
# agent-workflows

Deployed agent runtimes over the temper-mcp surface. **Eve** is the first runtime
binding; **Claude Managed Agents (CMA)** is a planned second (the two runtimes are
near-isomorphic — see docs/research/2026-06-18-vercel-eve-and-claude-managed-agents-investigation.md).

## Agents

- `steward/` — the team self-cognition steward (Eve). Design:
  docs/superpowers/specs/2026-07-01-t5-eve-steward-agent-directory-design.md.

## Why this package is workspace-isolated

Each Eve agent is a **self-contained Eve project** with its own toolchain
(TypeScript 7, `ai` v7, `@vercel/connect`, npm lockfile). It is deliberately NOT a
member of the root bun `workspaces` array, so it never collides with
`temper-cloud`'s TypeScript 5.8 and the repo pre-commit never touches it. Run all
tooling from inside the agent's directory:

```bash
cd steward && npm run typecheck      # tsc
cd steward && npm exec -- eve dev --no-ui   # boot locally (no REPL)
```

## Config

- `TEMPER_MCP_URL` — the temper-mcp target (`https://temperkb.io/mcp` or a
  self-hosted instance URL). Never hardcoded.
- Auth is platform/OAuth-carried (PKCE → temper-mcp Auth0); no static token in code.

Deployment (Vercel cron, envelope audit live) is T6.
```

- [ ] **Step 2: Reconcile the spec's D1 toolchain line**

In the design doc, update D1's file-tree comment that says the package shares
biome/tsconfig with the root. Replace the `package.json`/`biome.json`/`tsconfig.json`
lines under the D1 tree with a note that the Eve project keeps its own scaffolded
toolchain and is workspace-isolated (matching what shipped). Keep the rest of D1.

- [ ] **Step 3: Register the package in CLAUDE.md**

Add one line to the TypeScript Packages section of `CLAUDE.md`:
```md
- **agent-workflows** — Deployed agent runtimes over temper-mcp (Eve now, CMA later). Self-contained Eve project per agent; **workspace-isolated** (not a bun `workspaces` member — its own TS 7 toolchain), so run tooling from inside each agent dir. First agent: `steward/` (team self-cognition steward).
```

- [ ] **Step 4: Final verification — repo pre-commit is unaffected**

Confirm the isolated package did not disturb the repo's TS/biome scope:
```bash
(cd packages/temper-cloud && bun run typecheck && bun run check)
```
Expected: both pass (they only touch temper-cloud). Then confirm the steward still typechecks + boots (Task 5 Step 2 command).

- [ ] **Step 5: Commit**

```bash
git add packages/agent-workflows/README.md docs/superpowers/specs/2026-07-01-t5-eve-steward-agent-directory-design.md CLAUDE.md
git commit -m "docs(steward): agent-workflows README + spec/CLAUDE reconciliation (T5)"
```

---

## Self-review

**Spec coverage:**
- D1 (package placement, isolation, no pre-abstraction) → Tasks 1, 6.
- D2 (direct allow-list, 24 tools, 9 excluded, delete_resource soft-delete) → Task 2 Step 1.
- D3 (interactive PKCE auth, env-driven URL, no static token) → Task 2 Steps 1–2.
- D4 (instructions always-on + map-stewardship on-demand) → Tasks 3, 4.
- D5 (cron backstop; dispatch → T6) → Task 5.
- D6 (boot + typecheck + one real read) → Task 1 Steps 5–6, Task 2 Steps 3–4.

**Placeholder scan:** The PKCE helper functions (`makePkceVerifier`, `buildAuthorizeUrl`, `exchangeCode`, `lookupCachedToken`) are named-but-unimplemented in Task 2 Step 2 — deliberately, with an implementation note pointing at the temper CLI's existing Auth0 flow as the reference and a documented `TEMPER_DEV_TOKEN` fallback for the live-read gate. This is the one genuinely-beta, genuinely-involved surface; the note + fallback are honest for a beta framework rather than a hidden TODO. Everything else ships real content.

**Type consistency:** tool names in the allow-list (Task 2) match the `temper__<tool>` references in instructions.md (Task 3) and the skill loop (Task 4). `temperAuth` is defined in `temper-auth.ts` (Task 2 Step 2) and imported in `temper.ts` (Task 2 Step 1) under the `#connections/*` subpath alias from the scaffold's `imports` map.

**Beta-API confirmations to make at execution (noted inline, not placeholders):** `defineInteractiveAuthorization` field names, skill description frontmatter format, `defineSchedule` import path + action key. Each has a concrete verification step against the installed package.
```