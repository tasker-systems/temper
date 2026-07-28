> This is the Temper team-self-cognition **steward** — an Eve agent. Design: docs/superpowers/specs/2026-07-01-t5-eve-steward-agent-directory-design.md. It is a workspace-isolated Eve project; run tooling from THIS directory, not the repo root.

**Auth:** the M2M mint lives in `temper-ts` (`ClientCredentials`), taken as an npm `file:`
dependency — a deliberate bridge until temper-ts publishes, at which point the dependency becomes a
normal version range and the Vercel "include files outside the Root Directory" setting can go back
off. `agent/lib/temper-auth.ts` holds only what is deployment-specific: the env names (per principal
— see below) and the Vercel Connect / static-token strategies. Reach for `temperFetch` (steward) or
`auditorFetch` (auditor), never a bare `fetch` — they carry the 5xx cold-start retry AND the single
re-mint on 401.

**Model:** configured via `STEWARD_MODEL` / `STEWARD_MODEL_FALLBACKS` (`agent/lib/model-config.ts`).
eve resolves the model at BUILD time, so a change takes a **redeploy**, not a restart. The fallback
list is the AI Gateway's own (`providerOptions.gateway.models`) and covers availability, never
quality.

**Two agents live here, and their separation is the product.** Besides the root steward, this
project ships the **citation auditor** (Set 5) as a declared subagent at `agent/subagents/auditor/`.

> **The auditor has NO SCHEDULE — it is disabled and cannot fire.** Its dispatcher lives at
> `disabled/auditor.schedule.ts` (outside the agent root, still typechecked) so eve registers no
> cron for it — eve creates one Vercel Cron Job per file in `agent/schedules/`. It shipped enabled in
> PR #531, fired hourly from 2026-07-24T23:16Z, and failed every tick on an unprovisioned
> `TEMPER_AUDITOR_TOKEN` until it was withdrawn on 2026-07-25. Read that file's header before
> restoring it: it enumerates **four** gates between this file and a working audit (its own list
> said "three" until 2026-07-27 — it omitted the post-D11 system-access admission, which is the
> one that actually bit). **All four are now closed** as of 2026-07-27: the auditor principal is
> provisioned, registered, admitted, and has recorded a real audit against production. The
> **trigger-model redesign is also done** — task `019f975e-7be9-7ff3-a5bd-ef7ea72ff4a5` closed
> 2026-07-26 (tier 1 shipped; tier 2 retargeted onto the steward and closed 2026-07-27). The last
> build item, **D6**, is now **closed** too: the dispatch payload was finding-grained while the unit
> of work is citation-grained, so a tick re-audited citations already weighed (register
> `docs/superpowers/specs/2026-07-25-auditor-trigger-model-outcome-register.md` §3; measured on
> production 2026-07-27). `20260727000050_auditable_citations_at_citation_grain.sql` and task
> `019fa5eb-2819-7e71-bd27-0d2875f60960` moved the payload to `citations`, expanded per principal
> through `resource_auditable_citations`, so a session is handed only work it has not done.
> **Nothing but the operator decision now stands between this and an hourly cron** — restoring is a
> `git mv` back into `agent/schedules/`, and it is deliberately not a consequence of the grain work
> landing. The subagent, its channel and tools are live and unchanged; its instructions were
> updated with the grain (the work list is citations, and step 4's provenance read is now context
> for judgement rather than the set to iterate) — only the trigger is gone.

Code
colocation does not create epistemic dependence — the two have separate instructions, separate
success criteria, and never read one another — but three things must NEVER be shared, and eve's
declared-subagent isolation is what enforces them:

| | steward | citation auditor |
|---|---|---|
| Credential | `TEMPER_M2M_*` / `TEMPER_CONNECT_CONNECTOR` / `TEMPER_TOKEN` | `TEMPER_AUDITOR_M2M_*` / `TEMPER_AUDITOR_TOKEN` (**no Connect** — a connector is deployment-scoped, so it would authenticate as the steward) |
| Model | `STEWARD_MODEL` / `STEWARD_MODEL_FALLBACKS` | `AUDITOR_MODEL` / `AUDITOR_MODEL_FALLBACKS`, defaulting to a **different provider family** |
| Reach | `--cogmap <ref>` (write — it authors into its map) | team membership; cogmap reach only as `--cogmap <ref>:ro` |

One credential is one `kb_events.emitter_entity_id`. A shared client would leave the ledger unable
to tell an audit from the citation it audits. And a **writable** cogmap grant classifies the auditor
as `AuditAuthority::Author`, which 404s every audit it attempts with nothing failing at deploy time
— see `docs/auth/machine-token-contract.md` §C. Both schedules need `TEMPER_API_URL`; both
connections need `TEMPER_MCP_URL`.

**Tests:** `npm test` (vitest, `tests/`). They run in CI via `.github/workflows/test-agents-ts.yml`.

@AGENTS.md
