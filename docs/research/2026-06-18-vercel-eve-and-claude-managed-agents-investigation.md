# Vercel Eve & Claude Managed Agents — Investigation and Comparative Analysis

**Date:** 2026-06-18
**Status:** Research / reference
**Context:** Workstream 7 (Agent surface) under goal `substrate-kernel-to-cognitive-map`. Companion to the design doc [Agentic workflows on temper via Vercel Eve](../../). This document is the *runtime-comparison* substrate that the `temper-agents` neutral-contract design rests on, and doubles as prep for an upcoming Vercel/Eve briefing.

---

## Why this investigation

WS7 split temper's agent surface into two deployment strata on the assumption that the two target runtimes were *different in kind*: Vercel (where "sessions ARE the harness — invest in MCP tools + skills") versus an internal stratum with "managed agents + a thin producer-bound runtime." The `temper-agents` neutral-contract package needs a **`DeploymentProfile`** policy object that names exactly what differs between runtime bindings (decision #7: the profile "absorbs the divergence"). To size that object honestly, we inventoried how each runtime is actually *invoked* and *configured per deployment* — not their feature lists, but their **invocation and deployment-config surface**. The finding reshaped the design.

## Headline finding

**Vercel Eve and Claude Managed Agents (CMA) are nearly isomorphic at the deployment-config level.** Both are managed, durable, filesystem/sandbox-backed agent runtimes built around the same core split — a *persisted, versioned agent definition* plus a *per-run session binding* — with convergent mechanisms for durability, scheduling, human-in-the-loop approval, MCP tool binding, managed credential storage, markdown skills, and egress policy. The genuine divergence is small and clusters around **runtime identity, residency, and spend governance**. This makes the neutral contract *more* viable than WS7 assumed, and makes the `DeploymentProfile` *thin*.

A second-order finding: this convergence independently validates four claims the WS7 design made on intuition — durable resumability, the schedule backstop, the cross-map HITL gate, and markdown-skill portability all have a first-class mechanism on *both* runtimes.

---

## Part 1 — Vercel Eve: invocation & deployment-config surface

Eve (Vercel, open-source, announced June 2026) is a **filesystem-first framework for durable AI agents**: "an agent is a directory." Config splits between **code** (files under `agent/`) and **deployment** (Vercel environment variables / project settings / inherited product limits). Eve deliberately keeps a *thin* deployment-config surface — durability, retries, timeouts, region, and budget are not first-class Eve config; they are inherited from Vercel Workflow / Functions / Sandbox / Spend Management.

### Triggering / invocation
- **Channels** are the entry points (`agent/channels/*`, `defineChannel`): built-in eve (HTTP), Slack, Discord, Teams, Telegram, Twilio (SMS+voice), GitHub, Linear; custom via `defineChannel`.
- Documented HTTP wire shape: `POST /eve/v1/session` with `{"message": "..."}`, response carries `x-eve-session-id`; reattach via `GET /eve/v1/session/<id>/stream`. A **session** is the durable conversation; each inbound message/event is a **turn**. Conversations resume via a `continuationToken`.
- Schedules also trigger (see below).

### Agent definition (`defineAgent`)
`model` (AI Gateway ID, required if `agent.ts` exists; default `anthropic/claude-sonnet-4.6`), `compaction.thresholdPercent` (default 0.9), `modelOptions`, `experimental.codeMode`, `outputSchema`, `build.externalDependencies`, `description` (for subagents). On Vercel, models resolve through **AI Gateway** with **Vercel OIDC** auth — no provider keys held by the agent.

### Durability / workflows
Sessions run on **Vercel Workflow**: progress persisted as an event log, deterministically replayed; a session survives cold starts, redeploys, and long pauses *without consuming compute* while parked. "Every conversation is a durable workflow with each step checkpointed." Retries/timeouts/checkpoint cadence are **not** Eve knobs — inherited from Workflow/Functions limits. Fluid Compute (on by default for new projects) covers long streaming turns.

### Execution environment (sandbox)
One sandbox per agent (`agent/sandbox/*`), `backend: vercel | docker | microsandbox | just-bash`; `vercel({resources:{vcpus}})`, `microsandbox({memoryMiB})`, `docker({image,env,pullPolicy,networkPolicy})`. `networkPolicy`: `allow-all` (default) / `deny-all` / `{allow, subnets:{deny}}` with per-domain credential brokering. Vercel backend default 30-min idle timeout. **Not documented at the Eve level:** disk quotas, region/residency, budget caps — inherited from Vercel Sandbox/Functions.

### Schedules / cadence
`agent/schedules/`, `defineSchedule({cron})`, 5-field UTC cron, minute granularity; action is `markdown` (task prompt) XOR `run` handler. On Vercel each becomes a **Vercel Cron Job** (UTC); self-deployed registers Nitro scheduled tasks. `eve dev` never fires schedules on cadence.

### Connections / auth
`defineMcpClientConnection` (`url`, `tools` allow/block) and `defineOpenAPIConnection` (`spec`, `baseUrl`, `operations`). Auth: static token via `getToken` (Bearer, `principalType: "app" | "user"`), no-auth, or interactive OAuth via **Vercel Connect** (`connect()` manages consent + encrypted token storage). Credentials vary per environment through `process.env` inside `getToken`/`headers`; tokens never reach the model.

### Human-in-the-loop
Per-tool (and per-connection) `needsApproval`: `always()` / `once()` / `never()` / custom predicate. Framework handles the pause/approval-UI/resume durably on top of the Workflow checkpoint — no compute consumed while parked. An agent can also "ask a question" as a HITL pause. **Not documented:** approval timeout/expiry, escalation, approver authz.

### Per-deployment knobs (candidates for a policy object)
Cluster around **credentials/auth** (env vars, OIDC, Vercel Connect, channel route secrets), **model routing** (gateway IDs / provider auth), and **resource & spend governance** (Workflow/Functions/Sandbox limits, Spend Management — none of which Eve exposes as its own config). Everything structural — channel set, tool/skill definitions, compaction threshold, sandbox network policy, HITL modes — is **fixed in code** under `agent/`.

**Sources:** [Eve concepts](https://vercel.com/docs/eve/concepts), [pricing/limits](https://vercel.com/docs/eve/pricing), [agent-config](https://eve.dev/docs/agent-config), [channels](https://eve.dev/docs/channels/overview), [schedules](https://eve.dev/docs/schedules), [sandbox](https://eve.dev/docs/sandbox), [connections](https://eve.dev/docs/connections), [tools](https://eve.dev/docs/tools), [changelog](https://vercel.com/changelog/introducing-eve-an-open-source-agent-framework), [GitHub vercel/eve](https://github.com/vercel/eve).

---

## Part 2 — Claude Managed Agents: invocation & deployment-config surface

Anthropic has **three** agent surfaces; they differ exactly on the axes that matter for a deployment profile (who owns state, durability, triggering, HITL):

| Surface | Who runs the loop | Who owns state | Managed-framework analog? |
|---|---|---|---|
| **Messages API** (`/v1/messages`) | The caller | Caller (stateless; resend history) | No — per-request only |
| **Claude Agent SDK** (in-process `query()`) | A host process you run | Your filesystem (JSONL) | No — host-owned |
| **Managed Agents** (`/v1/agents` + `/v1/sessions`, beta `managed-agents-2026-04-01`) | Anthropic's orchestration layer | Anthropic-hosted event log + sandbox | **Yes — the Eve analog** |

Only **Managed Agents (CMA)** manages state/durability/triggering/HITL for you; the other two push all of it onto the caller/host. CMA is therefore the surface that compares to Eve.

### Invocation / triggering
Create-then-invoke: `POST /v1/agents` (persisted, **versioned** config — once) → `POST /v1/sessions` (references `agent` + `environment_id`; Anthropic provisions a sandbox container and runs the loop) → `POST /v1/sessions/{id}/events` (send `user.message`); receive via SSE stream, paginated list, or **webhooks**. Autonomous triggering via **scheduled deployments** (`POST /v1/deployments`, cron `schedule` + `initial_events`) — fires sessions on cadence with no caller in the loop. The session carries only a *pointer* (`agent` id + `environment_id` + initial events); heavy config lives on the agent object.

### Agent / request config (lives on the *agent object*)
`name`, `model` (string or `{id, speed}`), `system` (≤100K), `tools`, `mcp_servers`, `skills`, `multiagent`, `description`, `metadata`. Each `POST /v1/agents/{id}` update creates an immutable **version**; sessions can pin `{type:"agent", id, version}`. The session carries `agent`, `environment_id`, `title`, `resources`, `vault_ids`, `metadata`. **This persisted-config / per-run-binding split is precisely the pattern `temper-agents` decision #6 reached for.**

### Tools / MCP
Three tool kinds: prebuilt `agent_toolset_20260401` (bash/read/write/edit/glob/grep/web_fetch/web_search), `mcp_toolset` (agent's `mcp_servers[]` = `{type:"url", name, url}`, no inline auth), and custom client-side tools (`agent.custom_tool_use` → orchestrator executes → `user.custom_tool_result`). MCP credentials live in **Vaults** (`vlt_*`, attached via `vault_ids`): `mcp_oauth` (auto-refreshed), `static_bearer`, or `environment_variable` (substituted at egress, never visible in sandbox); matched to servers by URL.

### Durability / state
**Anthropic-managed, durable.** Sessions are long-running, resume cleanly after pauses, store conversation history + sandbox state + outputs server-side. Lifecycle `rescheduling → running ↔ idle → terminated`; built-in compaction + prompt caching; per-session persistent filesystem; optional cross-session **memory stores** (`memstore_*`, versioned, FUSE-mounted).

### Hosting / residency / budget
Environment (`/v1/environments`) chooses where tools run: `config.type: "cloud"` (Anthropic-managed sandbox) or `"self_hosted"` (your infra via an outbound-polling worker; the agent loop still runs on Anthropic). Networking per environment: `unrestricted` or `limited` (`allowed_hosts` / package managers / MCP). Available on first-party API + Claude Platform on AWS; **not** on Bedrock/Vertex/Foundry. **No managed dollar budget** anywhere — spend is expressed as token controls (`max_tokens` hard cap, `output_config.effort`, beta `task_budget` countdown) + per-org RPM/TPM rate limits. Caveat: CMA is stateful → **not ZDR/HIPAA-eligible**; Fable 5 needs 30-day retention.

### Human-in-the-loop
First-class per-tool `permission_policy`: `always_allow` (default) or `always_ask`. On `always_ask` the session goes idle with `stop_reason: requires_action`; you reply with `user.tool_confirmation` (`tool_use_id`, `result: allow|deny`, optional `deny_message`).

**Sources:** [Managed Agents overview](https://platform.claude.com/docs/en/managed-agents/overview), [scheduled deployments](https://platform.claude.com/docs/en/managed-agents/scheduled-deployments), [permission policies](https://platform.claude.com/docs/en/managed-agents/permission-policies), [Messages API](https://platform.claude.com/docs/en/api/messages), [Agent SDK overview](https://code.claude.com/docs/en/agent-sdk/overview).

---

## Part 3 — Cross-relation

### Convergence (the portable layer — lives in the agent definition, not the profile)

| Capability | Vercel Eve | Claude Managed Agents |
|---|---|---|
| Agent = persisted config | `agent/` dir + `defineAgent` | `POST /v1/agents` (versioned) |
| Per-run binding | session via channel | `POST /v1/sessions` (agent + env) |
| Durable park/resume | Vercel Workflow (checkpointed) | Anthropic-hosted stateful sessions |
| Cron triggering | `defineSchedule` → Vercel Cron | `POST /v1/deployments` (cron + initial_events) |
| HITL approval gate | `needsApproval: always/once/never` | `permission_policy: always_ask` → `user.tool_confirmation` |
| MCP tool binding | `defineMcpClientConnection` (url) | `mcp_servers` + `mcp_toolset` (url) |
| Managed credential store | Vercel Connect / connections | Vaults (`mcp_oauth` auto-refresh, keyed by URL) |
| Markdown skills | `skills/` | `skills` (anthropic + custom) |
| Egress policy | sandbox `networkPolicy` | environment `networking` (limited/allowed_hosts) |
| Model selection | AI Gateway model string | `model` id on agent |

Because these are convergent, they are **not** profile fields — they are the portable agent definition (skills, MCP tool list, persona, HITL policy) that both runtimes consume in the same shape. WS7 decision #7 guessed the profile would "absorb durability/checkpointing, triggering/channels, delegation mechanics"; the data shows those are convergent and therefore *assumed*, not absorbed.

### Divergence (what the `DeploymentProfile` must name)

1. **Runtime identity** — `Eve` vs `ClaudeManaged`. The discriminator.
2. **Residency** — where tool execution runs. *Orthogonal* to runtime: Eve can be Vercel-managed *or* docker-self-hosted; CMA can be `cloud` *or* `self_hosted`. This is WS7's "stratum" made concrete and explains why it is its own field rather than implied by the runtime.
3. **Spend governance** — **neither runtime exposes a managed dollar budget.** Eve defers to Vercel Spend Management; CMA expresses spend as *tokens* (`max_tokens`, `task_budget`, `effort`) + rate limits. A portable budget field must therefore be **token-denominated**.

### Resulting `DeploymentProfile` (thin)

```rust
pub enum RuntimeBinding { Eve, ClaudeManaged }
pub enum Residency { Managed, SelfHosted }

pub struct DeploymentProfile {
    pub runtime: RuntimeBinding,
    pub residency: Residency,
    pub token_budget: Option<u64>,  // token-denominated; None = runtime default
}
```

The interactive Claude-Code CLI path (decision #5) is intentionally out of scope — it is a human-driven session, not a deployed agent with a profile. **Region re-materialization cadence is also out of scope** — it is a deterministic, threshold-triggered substrate-maintenance job (re-run clustering when changes since the last materialization exceed a threshold; cheap since the WS5 Lance-Williams work cut region computation ~13.9×), system-wide and not agent-facing. Putting it in an agent's profile would contradict the determinism reframe (agents tend structure and never cluster; region formation is the substrate's pure function).

### Asymmetries worth holding in mind
These don't affect the *profile* (they sit below the CMA-vs-Eve comparison) but matter when reasoning about fallbacks to non-managed Anthropic surfaces:
- **State/durability:** managed only on CMA; the Messages API caller owns 100% (resend history), the Agent SDK host owns local JSONL.
- **Scheduling & HITL primitives:** exist only on CMA among the Anthropic surfaces; the Messages API has neither.
- **Auto-refreshing credential store (Vaults):** CMA-only on the Anthropic side.
- **Spend:** no managed dollar budget on *either* runtime — a framework promising "$X/run" must implement it itself.

---

## Part 4 — Implications for temper

1. **The neutral contract is more viable than WS7 assumed.** The binding seam is thin and the divergence is three fields. `temper-agents` can re-export the (already-shipped) invocation-envelope/authorship contract and add a small `DeploymentProfile`, with most agent config living as portable skills + the `temper-mcp` tool surface.
2. **Validated claims (now grounded, not intuited):** durable resumability as a *substrate* property (both runtimes park/resume) — the charter-bootstrapper's open question; the steward's schedule backstop (both have cron); the cross-map promotion HITL gate (both have approval primitives); markdown-skill portability (same format on both).
3. **The invocation envelope (PR #148) is the right grain.** Both runtimes emit lifecycle markers (Eve Workflow steps; CMA session events / webhooks) that temper can store as *opaque foreign events* (decision #4) without depending on them — the envelope correlates the mutation events the agent emits via MCP regardless of which runtime produced the orchestration markers.
4. **Delegation binding = the scoped principal = the auth.** Eve `principalType` / Vercel Connect and CMA Vaults both express "operate as this principal." The delegation binding maps to which credential the runtime presents to `temper-mcp` — portable.

---

## Part 5 — Open questions to raise with Vercel

Documentation gaps surfaced during the investigation, framed as questions for the Eve team:

1. **Inbound message envelope.** The HTTP channel documents `{message}` + `sessionId` + `continuationToken`, but not the full inbound envelope (metadata, attachments, idempotency keys). What is the complete shape, and is there a stable idempotency key we can correlate to a temper invocation id?
2. **Durability knobs.** Retries, turn timeouts, max session duration, and checkpoint cadence are inherited from Vercel Workflow/Functions rather than exposed as Eve config. Are there Eve-level overrides planned, or is the contract "tune Workflow/Functions directly"?
3. **Sandbox residency.** Region/data-residency for the Vercel sandbox backend isn't documented at the Eve layer. What residency guarantees exist, and can they be pinned per agent? (Directly relevant to the `Residency::Managed` field and any enterprise/self-host stratum.)
4. **HITL governance.** Approval timeout/expiry, escalation, and *who-can-approve* (authz on the approver) aren't documented. How does a durable-parked approval behave if never answered, and can approver identity be constrained?
5. **MCP connection semantics.** Timeout/retry/rate-limit behavior for `defineMcpClientConnection`, and whether an idempotent MCP call re-issues cleanly across a durable park/resume (we rely on WS3 identity-as-input for byte-exact re-issue — does Eve's checkpoint preserve generated ids across resume?).
6. **Spend controls.** Is there any per-agent or per-deployment spend cap on the Eve/Vercel side beyond project-level Spend Management, or is budget governance entirely token-side on the model provider?
7. **Self-hosting posture.** `backend: docker | just-bash` and self-deploy (Nitro) suggest an off-Vercel story. What is supported/intended for running Eve agents on customer infrastructure (relevant to the enterprise self-host stratum)?

---

## Sources

All claims above are cited inline. Primary references: Vercel Eve docs ([concepts](https://vercel.com/docs/eve/concepts), [pricing](https://vercel.com/docs/eve/pricing), [agent-config](https://eve.dev/docs/agent-config), [channels](https://eve.dev/docs/channels/overview), [schedules](https://eve.dev/docs/schedules), [sandbox](https://eve.dev/docs/sandbox), [connections](https://eve.dev/docs/connections), [tools](https://eve.dev/docs/tools)) and Anthropic Managed Agents docs ([overview](https://platform.claude.com/docs/en/managed-agents/overview), [scheduled deployments](https://platform.claude.com/docs/en/managed-agents/scheduled-deployments), [permission policies](https://platform.claude.com/docs/en/managed-agents/permission-policies), [Agent SDK](https://code.claude.com/docs/en/agent-sdk/overview), [Messages API](https://platform.claude.com/docs/en/api/messages)). Eve is beta; CMA is beta (`managed-agents-2026-04-01`) — treat specifics as current-as-of 2026-06-18 and re-verify before relying on any single field.
