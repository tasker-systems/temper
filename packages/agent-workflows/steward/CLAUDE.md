> This is the Temper team-self-cognition **steward** — an Eve agent. Design: temper-artifacts:specs/2026-07-01-t5-eve-steward-agent-directory-design.md. It is a workspace-isolated Eve project; run tooling from THIS directory, not the repo root.

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

> **The auditor's cron is LIVE as of 2026-07-29** — `agent/schedules/auditor.ts`, **daily at 03:30
> UTC**, trailing a steward tick. eve creates one Vercel Cron Job per file in `agent/schedules/`, so
> **this file's location is the repo-wide on/off switch**: withdrawing the *capability* means moving
> it back out. The guards in `run` are a different thing — they decide whether a given *deployment's*
> tick does work, and they never remove the ability to run the auditor on demand.
>
> **Cadence is a budget mechanism, not a preference.** The AI Gateway's monthly allowance is a fixed
> ceiling rather than something to top up, so cadence is how the auditor is made to fit inside it.
> The `:30` minute is load-bearing (it trails the steward's `0 * * * *` so the two never write
> concurrently over one map); the hour is an operator's to move. The committed value is every fork's
> default.
>
> **Three axes decide whether a tick does any work, with three different defaults.** Two live in
> `agent/lib/optional-agent.ts`; the credential one stays in `temper-auth.ts` because it is genuinely
> auth. All three are a **skip, never a fallback** — `auditorFetch` still throws rather than
> borrowing the steward's credential, and that is pinned by test.
>
> | axis | predicate | absent means |
> |---|---|---|
> | credential | `credentialConfigured(AUDITOR_CREDENTIALS)` | **skip** — nobody configured an auditor here |
> | enablement | `agentEnabled(AUDITOR_ENABLED)` | **run** — a merge may not turn a production cron off |
> | capacity | `tokenIssuanceUnavailable(err)` | n/a — classified from the failure, never pre-checked |
>
> The credential guard exists because this schedule ships in the repo, so every fork and self-hosted
> deploy gets the cron whether or not it runs an auditor. Only **total** absence skips: a partially
> configured auditor (client id set, secret missing) still fails loudly, because that is a
> misconfiguration by someone who meant to run one, and silence there means believing you are
> auditing when you are not.
>
> **`TEMPER_AUDITOR_ENABLED` is an opt-OUT** — the inverse polarity of everything else here, so that
> every deployment auditing today keeps auditing across the change that added it. Unset,
> declared-but-empty, and unrecognized values all mean **enabled**; the last one warns.
>
> **Correct credentials are not sufficient credentials.** Auth0 enforces a monthly quota on M2M token
> issuance, so a credential can be entirely correct and still refused — `429`, and deliberately not
> the AI Gateway's `402`. That is a funding ceiling, not a misconfiguration, and it takes the same
> quiet skip. A wrong credential is `401` and stays loud. The status line is the whole predicate;
> the body is never matched, and no probe token is minted, because minting one spends the quota being
> probed.
>
> It shipped enabled once before, in PR #531, fired hourly from 2026-07-24T23:16Z, and failed every
> tick on an unprovisioned `TEMPER_AUDITOR_TOKEN` until it was withdrawn on 2026-07-25. Both lessons
> from that are now structural rather than remembered: enabling a production cron is an operator
> decision (the file's location), and an absent optional credential must no-op (the guard).
>
> Read `agent/schedules/auditor.ts`'s header for the **four** gates between this cron and a working
> audit (its own list said "three" until 2026-07-27 — it omitted the post-D11 system-access
> admission, which is the one that actually bit). All four are closed on temperkb.io as of
> 2026-07-27; a fork standing up an auditor walks the same four. The trigger-model redesign is done
> (task `019f975e-7be9-7ff3-a5bd-ef7ea72ff4a5`, closed 2026-07-26; tier 2 retargeted onto the
> steward and closed 2026-07-27), and the last build item **D6** closed with
> `20260727000050_auditable_citations_at_citation_grain.sql` and task
> `019fa5eb-2819-7e71-bd27-0d2875f60960`: the payload is now `citations`, expanded per principal
> through `resource_auditable_citations`, so a session is handed only work it has not done. Its
> instructions were updated with the grain — the work list is citations, and step 4's provenance
> read is context for judgement rather than the set to iterate.

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

**One principal per agent, team-owned from birth.** Each agent authenticates as its own machine
principal, registered with an `--owner-team` whose owners manage it, and with explicit, plural
reach — `--team` for source read, `--cogmap` for map reach — never inferred from the owner team.
Rotating the credential's secret is an IdP-side act; rotating the IdP **application** re-mints the
principal: register the new client id (`admin machine rebind --no-revoke-old` when the profile's
ownership and reach are already right, otherwise a fresh `provision`), approve the new profile's
admission, swap the deployment's `*_M2M_CLIENT_ID` / `*_M2M_CLIENT_SECRET` env, redeploy, and
verify a full tick under the new credential **before** revoking the old client id. Revocation
denies authentication and leaves authorship history intact.

**Tests:** `npm test` (vitest, `tests/`). They run in CI via `.github/workflows/test-agents-ts.yml`.

@AGENTS.md
