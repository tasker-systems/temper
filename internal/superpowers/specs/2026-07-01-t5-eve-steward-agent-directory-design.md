# T5 — Eve Steward Agent Directory

**Date:** 2026-07-01
**Status:** Design — approved in brainstorming, pending plan
**Goal:** `team-self-cognition-steward-agent-eve-mvp` (019f1ac7)
**Task:** T5 (build; depends on T1 tools + T3 act-model, both shipped)
**Workstream:** WS7 (Agent surface) under `substrate-kernel-to-cognitive-map`

> This spec pins the concrete directory shape, MCP tool surface, auth binding, and
> persona for the Eve steward agent. It inherits every behavioral decision (labels,
> edges, provenance, re-run semantics, authorship) from the T3 keystone
> ([2026-06-30-steward-act-model-cogmap-resource-vocabulary-design.md](2026-06-30-steward-act-model-cogmap-resource-vocabulary-design.md))
> — this spec adds only the *runtime binding*: where the code lives, which tools the
> steward may reach, how it authenticates, and how the persona is expressed.

## Context

The goal dogfoods cognitive maps: a team's own temper resources become the ingest
source for a cogmap 1:1 with the team, tended by a Vercel/Eve steward on a
cron-threshold cadence. T1 shipped the `facet_set` + `cogmap_read_charter` MCP tools
and doctype parity; T3 pinned the act-model; T4a shipped the ingest-delta/watermark
tools. T5 is the agent directory that binds those to the Eve runtime.

Two framing decisions from the brainstorm shape this spec beyond T3:

1. **This is the first agent in a multi-runtime, multi-agent framework.** Eve is the
   first runtime binding; **Claude Managed Agents (CMA) is a planned second** (the
   research doc [2026-06-18-vercel-eve-and-claude-managed-agents-investigation.md](../../research/2026-06-18-vercel-eve-and-claude-managed-agents-investigation.md)
   found the two runtimes near-isomorphic, with a thin `DeploymentProfile` seam). The
   steward is the first agent; more agents (charter-bootstrapper, admin) will follow.
   The package is therefore named `agent-workflows`, not `temper-steward`.
2. **Env-driven target, platform-carried auth.** The agent must launch simultaneously
   against temperkb.io **and** the freshly-spun-up self-hosted instance, so the MCP
   *target URL* must never be hardcoded. Auth is carried by the platform/OAuth layer
   (not a static token env var) — the connection authorizes once and carries through.

## Scope

**In (this session):** the full `agent-workflows/steward/agent/` directory (agent.ts,
instructions, MCP connection, tools allow-list, schedule backstop, map-stewardship
skill); `eve dev` boots and the directory typechecks; **one real temper-mcp read**
against a live deployment (proving target + token resolve).

**Deferred (named):** the full authored-4 **write** loop run end-to-end against the live
team self-cogmap (T6, needs the map to exist + deploy); the scheduler dispatch wiring
proper (`kb_system_settings.steward_scheduler` eve-vs-temper — deferred in the arch
spec); the CMA runtime binding + the runtime-abstraction layer (built when CMA is real,
not before — YAGNI); other agent personas (charter-bootstrapper, admin).

## Decisions

### D1 — Package: `packages/agent-workflows/`, steward as first agent

A new bun-workspace package alongside `temper-cloud` and `temper-ui`. Named for the
framework, not the one agent, to reserve room for CMA and future agents. **The
runtime-abstraction layer is NOT built now** — the portable artifacts (instructions,
map-stewardship skill, tool allow-list) physically live in the Eve agent directory for
the MVP; they get factored to a shared location when a second consumer (the CMA binding)
actually exists. The name reserves the room; we do not pre-abstract.

```
packages/agent-workflows/
├── README.md                 # why the package is workspace-isolated; deploy notes
└── steward/                  # a self-contained Eve project (from `eve init`)
    ├── package.json          # Eve's OWN toolchain: eve, @vercel/connect, ai v7, zod; TS 7 RC; node 24
    ├── package-lock.json     # pins the isolated toolchain (npm, not bun)
    ├── tsconfig.json         # Eve's own (ES2022, NodeNext, strict) — NOT shared with the repo
    └── agent/                # Eve "an agent is a directory"
        ├── agent.ts          # defineAgent({ model, description })
        ├── instructions.md   # always-on steward persona
        ├── channels/eve.ts   # scaffold entry point (vercelOidc/localDev/placeholderAuth)
        ├── connections/
        │   └── temper.ts     # defineMcpClientConnection — env URL, platform auth, approval never(), 24-tool allow-list
        ├── skills/
        │   └── map-stewardship.md   # on-demand: D3/D5/D6/D7/D8 procedure + concrete loop
        └── schedules/
            └── steward.ts    # defineSchedule({ cron, markdown }) — cron backstop
```

**As-built note:** the earlier draft of this tree had the package share the repo's
biome/tsconfig. The real `eve init` scaffold (eve 0.18.1) brings its own opinionated
toolchain (TypeScript 7 RC, `ai` v7, npm lockfile, node 24) and no biome, so the Eve
project keeps its scaffolded toolchain and is **not** a bun-`workspaces` member — the
repo pre-commit (`cd packages/temper-cloud`) never touches it, and its own TS 7 never
collides with temper-cloud's TS 5.8. Install/run tooling from inside `steward/` (an
`npm install` from the repo root inherits the root's bun-oriented `overrides` and
fails).

### D2 — MCP connection: direct allow-list binding, no wrapper code

One `defineMcpClientConnection` (`agent/connections/temper.ts`) to the temper-mcp URL,
with an explicit tool allow-list. The model drives the tools directly per the
instructions and skill — **no local `defineTool` wrappers**. Wrappers are rejected: they
add a second contract to maintain, are less portable to CMA (whose `mcp_servers` uses the
same URL shape), and the multi-step composition they would encode (dedup, supersede)
lives more legibly in the map-stewardship skill prose. Eve exposes remote tools as
`temper__<tool>` (e.g. `temper__create_resource`).

**The allow-list is scoped to the steward's role, not minimized to the MVP loop.** The
steward will grow into the full resource-CRUD + edge + envelope + orientation surface, so
those are all included. Exactly **9 of the 33** production temper-mcp tools are excluded,
on two *principled* grounds (not "the MVP doesn't need it"):

**Excluded — region/salience reads (3):** `cogmap_shape`, `cogmap_analytics`,
`cogmap_region_metrics`. The determinism reframe (T3/D1) forbids the steward reasoning
about regions or salience — region formation is the substrate's pure function on
`materialize` (T4b), never the agent's concern. Exposing these reads invites exactly the
salience-thinking the design prohibits.

**Excluded — genesis/admin/access (6):** `cogmap_create`, `cogmap_bind`, `cogmap_unbind`
(map genesis is operator/L1/bootstrapper work — the steward tends *one existing* map);
`cogmap_grant`, `cogmap_revoke` (the steward must never alter access — leak-safety);
`create_context` (the steward writes nodes into the cogmap, it does not make contexts).
These belong to *other* agent roles (charter-bootstrapper, admin) whose own connections
will carry them. Least-privilege by role.

**Included (24):**

| Group | Tools |
|-------|-------|
| Authored-4 | `create_resource`, `assert_relationship`, `facet_set`, `fold_relationship` |
| Envelope | `invocation_open`, `invocation_close`, `invocation_show`, `invocation_list` |
| Steward delta | `steward_ingest_delta`, `steward_advance_watermark` |
| Reads | `search`, `get_resource`, `get_context`, `list_contexts`, `list_resources`, `cogmap_read_charter`, `describe_doc_type`, `list_doc_types`, `get_profile` |
| Mutations | `update_resource`, `update_resource_meta`, `delete_resource`, `retype_relationship`, `reweight_relationship` |

`delete_resource` is **verified soft-delete** — it routes through a `resource_deleted`
event that flips `is_active` (`crates/temper-substrate/src/payloads.rs:407`,
`writes.rs:299`), never a hard row delete, consistent with the append-only/fold model.

The steward's MVP *behavior* uses only the T3 loop's subset (delta → charter → search →
create/assert/facet/fold → close); the broader connection surface is the framework
affordance, constrained to the loop by the **instructions**, not the allow-list.

### D3 — Auth: platform-carried, env-driven target only

temper-mcp is a complete OAuth 2.0 server — Auth0 discovery at
`/.well-known/oauth-authorization-server` (RFC 8414), protected-resource metadata (RFC
9728), dynamic client registration at `/oauth/register`, PKCE, and the `refresh_token`
grant (`crates/temper-mcp/src/discovery.rs`; the temper CLI already drives this exact
flow). Auth is **platform-carried, not a static secret in code**.

**As-built note (eve 0.18.1):** the earlier draft named `defineInteractiveAuthorization`,
which does not exist in the shipped eve. The real connection-auth surface is two modes:
`connect()` from `@vercel/connect/eve` (Vercel Connect — the documented, recommended path
for OAuth-backed MCP servers: it owns consent, encrypted storage, and refresh), and
`auth: { getToken }` (which explicitly supports "your own OAuth exchange", runs per
connection attempt, and honors `expiresAt` for refresh-ahead-of-401). The shipped
connection uses **`connect({ connector, principalType: "app" })` as the production path**
(gated on `TEMPER_CONNECT_CONNECTOR`, registered via `vercel connect create` in T6), with
a **`getToken` dev fallback** that returns an already-OAuth-obtained temper token
(`TEMPER_TOKEN`) for local boot and the one-real-read gate. Approval is `never()`
(fully autonomous + audited, D8).

**Only the target URL is env-driven:** `url: process.env.TEMPER_MCP_URL`. One agent dir
points at `https://temperkb.io/mcp` or the self-hosted instance by env value alone. Any
audience/issuer that varies by target is likewise env-sourced
(`TEMPER_MCP_AUDIENCE` / discovery-derived), never hardcoded.

Per-act authorship is orthogonal to the connection principal: every mutating call carries
`AgentAuthorship` (`invocation_id`, `confidence` graded band, `reasoning`, `persona`,
`model`) on the `ActInput` wire (T3/D8) — reasoning required on structural acts
(create/edge/fold), optional on facets.

### D4 — Persona split: `instructions.md` (always-on) + `map-stewardship.md` (on-demand)

`instructions.md` is the always-loaded identity — short: the steward operates under the
team telos, distills the team's own resources into cogmap-homed nodes, **declares
structure and never clusters or assigns salience**, supersedes via fold (never in-place
edit), and wraps every run in the invocation envelope. It points at the skill for the
detailed procedure.

`skills/map-stewardship.md` is the on-demand detailed stewardship procedure, authored
fresh from the T3 keystone (there is no pre-existing Claude-Code skill file to port — the
"port" is authoring from the spec). It encodes:

- **D3 label glosses** — crisp one-line definitions of the recognized node labels
  (`fact`/`memory`/`decision`/`concept`/`question`/`theme`/`concern`/`principle`/
  `commitment`/`domain`) so label choice stays consistent, plus the open-tail rule.
- **D5 granularity** — per-source labels (fact/memory/decision) cite ~1 source; synthesized
  labels span many.
- **D6 edge conventions** — the semantic-label → EdgeKind/Polarity table (`derived_from` →
  Express, `relates_to` → Near, `part_of` → Contains, `answers`/`supports`/`contradicts`
  → LeadsTo).
- **D7 re-run + "materially changed" heuristic** — accretive; search-before-create dedup;
  fold-on-supersede as agent judgment, not a hash threshold.
- **D8 authorship discipline** — envelope open/close, confidence bands, reasoning-required
  acts.
- The concrete steward loop (T3 pseudocode).

The skill is portable markdown — the same file a future CMA binding consumes as an
Anthropic skill (research doc, "markdown-skill portability").

### D5 — Schedule: cron backstop only

`schedules/steward.ts` = `defineSchedule({ cron })` → a Vercel Cron backstop. The
scheduler dispatch proper (threshold-driven wake via `steward_ingest_delta`, and the
`steward_scheduler ∈ {eve, temper}` switch) is **deferred** per the arch spec — T6
territory. The schedule's action invokes the steward loop (which itself checks the
ingest-delta threshold and no-ops under threshold).

### D6 — Verification this session

- `eve dev` boots the agent directory (`tsc` passes; the repo `biome check` still
  only touches temper-cloud — the isolated package is invisible to it).
- **One real temper-mcp read** against live `TEMPER_MCP_URL=https://temperkb.io/mcp`
  proving the connection's target + token resolve: `initialize` returns 401 without
  the bearer and 200 with it, and a `tools/call` for the allow-listed `list_contexts`
  returns real data. **As-built:** the model-driven eve-session read is blocked in a
  local dev loop by AI Gateway auth (`eve link` / `AI_GATEWAY_API_KEY` — a deploy
  concern), so the read was proven by a direct authenticated MCP call, per the risk
  note. The full model-driven session read lands with T6.
- The full authored-4 **write** loop against the live team self-cogmap is T6.

## The steward loop (inherited from T3, for reference)

```
on tick:
  delta = steward_ingest_delta(team_cogmap, threshold)     # T4a; no-op under threshold
  inv   = invocation_open(team_cogmap, trigger=scheduled)
  telos = cogmap_read_charter(team_cogmap)                  # orient
  for source in delta.new_or_changed:
    existing = search(team_cogmap, source)                  # dedup (D7.2)
    if materially_changed(source, existing):                # agent judgment (D7.4)
      fold_relationship(existing.derived_from); existing = none
    if not existing:
      node = create_resource(cogmap=team_cogmap, type=<label>, authorship=stamp(inv,…))
      assert_relationship(node -> source, label="derived_from", kind=express)
      for rel in inter_node_relationships(node):
        assert_relationship(node -> other, kind, polarity, label, weight)
      for f in facets(node):
        facet_set(node, f)
  steward_advance_watermark(team_cogmap, delta.max_event_id)  # wired to close
  invocation_close(inv, outcome)
```

## Isolation & interfaces

- **The connection** (`temper.ts`) is the sole seam to temper-mcp — one URL, one
  allow-list, one auth strategy. Everything else references tools by `temper__<name>`.
- **The persona** (`instructions.md` + `map-stewardship.md`) is pure portable markdown —
  no runtime coupling; consumable by CMA unchanged.
- **The schedule** is a thin trigger; the loop logic lives in the skill/persona, not the
  schedule handler.
- **The `DeploymentProfile` seam** (research doc) stays out of code until CMA is real —
  the Eve binding is the only binding, so there is nothing to abstract yet.

## Risks & open items

- **Eve API drift (resolved this build)** — Eve is beta; the connection API was verified
  against the *installed* eve 0.18.1 docs at `node_modules/eve/docs/` (authoritative,
  version-matched). Net corrections vs the earlier web-sourced draft: no
  `defineInteractiveAuthorization` (use `connect()` / `getToken`); `tools: { allow: [...] }`
  is the filter shape; `defineSchedule({ cron, markdown })`; skills auto-discover under
  `agent/skills/` with a `description` frontmatter routing hint.
- **AI Gateway in `eve dev`** — the model-driven session read needs `eve link`
  (`VERCEL_OIDC_TOKEN`) or `AI_GATEWAY_API_KEY`; unavailable in a bare local loop. The
  one-real-read was therefore proven by a direct authenticated MCP call (401→200 +
  `list_contexts` real data). Wiring the gateway credential and running the full
  model-driven read is folded into T6 (deploy).
- **Production Connect connector** — `connect()` needs temper-mcp registered as a Vercel
  Connect connector (`vercel connect create`) and `TEMPER_CONNECT_CONNECTOR` set; that
  registration + the app-scoped token issuance is a T6 step. Until then the `getToken`
  dev fallback carries local runs.
- **Self-hosted target parity** — the self-hosted instance's Auth0 tenant/audience may
  differ; the discovery-driven flow should absorb this via
  `TEMPER_MCP_URL` + discovery, but confirm against the self-hosted `/.well-known`
  endpoints.

## Code anchors (verified 2026-07-01)

- Shipped MCP tools: `crates/temper-mcp/src/tools/` (registration `service.rs`);
  `facet_set` `facets.rs`, `cogmap_read_charter` `cognitive_maps.rs`,
  `steward_ingest_delta`/`steward_advance_watermark` `steward.rs`,
  `invocation_open`/`close`/`show`/`list` `invocations.rs`.
- `delete_resource` soft-delete: `crates/temper-substrate/src/payloads.rs:407`,
  `writes.rs:299`; MCP handler `crates/temper-mcp/src/tools/resources.rs:755`.
- OAuth discovery: `crates/temper-mcp/src/discovery.rs`; config
  `crates/temper-mcp/src/config.rs`.
- MCP deploy adapter + routes: `api/mcp.rs`, `vercel.json` (`/mcp` → `/api/mcp`).
- Bun workspaces: root `package.json` (`workspaces: [packages/temper-cloud, packages/temper-ui]`).

## Connections

- Keystone: [2026-06-30-steward-act-model-cogmap-resource-vocabulary-design.md](2026-06-30-steward-act-model-cogmap-resource-vocabulary-design.md).
- Arch spec (L0/L1/L2 tiers, scheduler): [2026-06-25-cognitive-map-agent-invocation-architecture-design.md](2026-06-25-cognitive-map-agent-invocation-architecture-design.md).
- Runtime comparison (Eve vs CMA, DeploymentProfile): [2026-06-18-vercel-eve-and-claude-managed-agents-investigation.md](../../research/2026-06-18-vercel-eve-and-claude-managed-agents-investigation.md).
- Goal `team-self-cognition-steward-agent-eve-mvp`; tasks T1 (tools), T3 (act-model), T4a (delta), T4b (materialize), T6 (deploy), T7 (block provenance).
