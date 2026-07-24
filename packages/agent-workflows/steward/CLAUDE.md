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
project ships the **citation auditor** (Set 5) as a declared subagent at `agent/subagents/auditor/`,
driven by `agent/schedules/auditor.ts` (hourly at `:30`, trailing the steward's `:00`). Code
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
