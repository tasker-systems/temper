import { defineSchedule } from "eve/schedules";
import { TEMPER_TS_VERSION } from "temper-ts";

import auditorWorker from "../agent/channels/auditor-worker.js";
import { auditorFetch, requireEnv } from "../agent/lib/temper-auth.js";

/**
 * ═══ DISABLED 2026-07-25 — this file is deliberately OUTSIDE the eve agent root. ═══
 *
 * eve registers exactly one Vercel Cron Job per file under `agent/schedules/`
 * (`node_modules/eve/docs/schedules.mdx`: *"Each one is a single file under `agent/schedules/`
 * carrying a cron expression"*). Moving it out is therefore the only way to stop the cron
 * EXISTING. Guarding the `run` body would have left an hourly job that fires and no-ops, which is
 * still an enabled schedule — and "enabled" is the thing being withdrawn here, not "erroring".
 *
 * It sits here rather than in `agent/disabled/` because eve warns
 * `[discover/unsupported-directory]` on any unrecognized directory inside the agent root, and a
 * warning printed on every production build is noise that teaches people to skip warnings. It is
 * still typechecked — `tsconfig.json` includes this directory — so it cannot rot silently while
 * it waits.
 *
 * **Why it was turned off.** It shipped enabled in PR #531 and began firing in production on
 * 2026-07-24T23:16Z. Every tick since died at `requireEnv("TEMPER_AUDITOR_TOKEN")` before its
 * outbound fetch — ~18 consecutive failures, unannounced, while the subsystem is still being
 * designed. The credential requirement is real (see below), documented in
 * `docs/auth/machine-token-contract.md` §C, and was never surfaced as a deploy action item:
 * `DEPLOYING.md` mentions the auditor only for its migration cutover. Enabling a scheduled agent
 * against production is an operator decision, and it was never taken.
 *
 * **This is not a fix for the credential.** Nothing here is broken in a way a token would repair.
 * FOUR independent gates stand between this file and a single audit row, and the token is only
 * the first. (This list said "three" until 2026-07-27 and omitted gate 3 — which was already
 * true when the list was written, D11 having landed five days earlier. All four are now CLOSED
 * on temperkb.io; the list is kept because restoring this file is not the only path back here.)
 *
 *   1. ✅ A SECOND machine principal, provisioned per `machine-token-contract.md` §C — its own IdP
 *      application, `--team <ref>:member`, and cogmap reach absent or `:ro`. Never the steward's
 *      client_id: one credential is one `kb_events.emitter_entity_id`, so a shared client makes
 *      every steward-authored citation self-authored to the auditor (`AuditAuthority::Author`)
 *      and 404s every audit. A *writable* `--cogmap` grant does the same thing through
 *      `can_modify_resource`.
 *      Done 2026-07-27: `EcbiQJWxSDbhSMfTPMCOEBQboDa5CMua`, profile
 *      `019fa583-1142-7121-bd67-597945e5f45f`.
 *   2. ✅ Registration in `kb_machine_clients` — `resolve_machine_from_claims` is lookup-or-401,
 *      with no JIT create branch.
 *   3. ✅ ADMISSION. Since D11 (`20260720000110_repoint_predicates.sql`) `has_system_access`
 *      reads `kb_principal_standing` alone, and every mint door births the principal `denied`
 *      — reach does NOT clear it. A registered, correctly-reached machine still 403s
 *      `SYSTEM_ACCESS_REQUIRED` on every call until `temper admin access approve <profile>`
 *      runs. This gate has no deploy-time symptom whatsoever: the token mints, the claims are
 *      perfect, and every request fails. It is the one that actually bit on 2026-07-27.
 *   4. ✅ The Set 5 migration cutover, which `DEPLOYING.md:78-86` says is non-additive and must not
 *      ride an auto-deploy of `main`. Until an operator runs it, `/api/auditor/dispatch` 500s.
 *
 * **Do not restore this file by itself.** Its trigger model is being redesigned — task
 * `019f975e-7be9-7ff3-a5bd-ef7ea72ff4a5`, register
 * `docs/superpowers/specs/2026-07-25-auditor-trigger-model-outcome-register.md` — and that work
 * changes the cadence, the selection predicate, and the dispatch payload this handler sends. The
 * register also records that the dispatch prompt below tells the agent not to re-check whether a
 * finding needs auditing (finding grain) while its unit of work is a citation (citation grain),
 * which duplicates verdicts on already-audited citations. Restoring is `git mv` back into
 * `agent/schedules/`, and it should happen as part of that work, after the three gates above.
 *
 * The auditor subagent, its channel, its tools and its instructions are all untouched and still
 * build; only the trigger is withdrawn.
 *
 * ─────────────────────────────────────────────────────────────────────────────────────────────
 *
 * Citation-auditor fan-out dispatcher (Set 5; spec
 * `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §6).
 *
 * CONFORM to `schedules/steward.ts` on all three of its load-bearing shapes, deliberately:
 *
 * 1. **A code `run` handler, not a model-driven `markdown` prompt.** Selecting which findings have
 *    uncovered citations is deterministic substrate work — `POST /api/auditor/dispatch` runs
 *    reap → sweep → group → enqueue → claim server-side — and only weighing a citation is model
 *    work. So this handler does the deterministic dispatch and then starts ONE isolated agent
 *    session per claimed job.
 * 2. **The fan-out is over the WORKFLOW, never over an agent's target.** One session, one job, one
 *    cogmap. What differs from the steward is *inside* a session: an auditor session iterates the
 *    finding list its job carries. That list exists because the queue's single-flight index is
 *    `(cogmap_id, persona, dispatch_type)` while the auditor's unit of work is a finding — so the
 *    server groups by cogmap and puts the findings in the payload rather than enqueuing per finding,
 *    which `ON CONFLICT DO NOTHING` would silently collapse to one (spec §6.1).
 * 3. **A correlation id minted per tick and threaded across the app boundary** — logged here before
 *    the outbound fetch, sent as `x-auditor-correlation-id`, echoed back by the server, and stamped
 *    onto every claimed job so each session's `invocation_open` inherits it server-side. The prompt
 *    below therefore does NOT mention the correlation: one tick is one dispatch act plus N run-grain
 *    sessions, and an agent that passed the tick id to a write tool would collapse act grain into
 *    run grain.
 *
 * What is NOT shared with the steward, and must not become shared:
 *
 * - **The credential.** `auditorFetch`, not `temperFetch`. One credential is one
 *   `emitter_entity_id`; a shared client would leave the ledger unable to tell an audit from the
 *   citation it audits (spec §5.2). The auditor is its own registered machine client, provisioned
 *   with team membership and **read-only** cogmap reach — a writable `--cogmap` grant makes it
 *   `AuditAuthority::Author` for every finding in that map and 404s every audit it attempts. See
 *   `docs/auth/machine-token-contract.md` §C.
 * - **The model.** The session runs on the `auditor` declared subagent, whose `agent.ts` resolves
 *   `AUDITOR_MODEL` (`lib/model-config.ts`) — spec §5.3's one real lever against shared trained
 *   priors. eve gives a schedule no per-session model override and no way to start a session
 *   directly on a subagent, so the tick starts a root session that immediately delegates. That hop
 *   is the price of the isolation, and it buys the whole of it: a declared subagent inherits NONE of
 *   the root's authored slots, so the auditor's connection and credential are unreachable from a
 *   steward session, and vice versa.
 *
 *   **The hop itself is capability-bounded, not prompt-bounded — and that is asserted.** One turn of
 *   every audit run executes in a root STEWARD session holding the steward credential, told by
 *   prompt to call nothing but the subagent; the subagent's report, derived from attacker-authorable
 *   resource text, then returns into it. What stops a misbehaving hop from emitting an audit under
 *   the wrong principal is that `record_citation_audit` is not in the steward's allow-list
 *   (`lib/tool-allowlists.ts`). That held by accident of two separately-authored lists until
 *   `tests/auditor.test.ts` started asserting it; treat the assertion as load-bearing, not tidiness.
 *
 * Cadence: hourly at :30, half an hour behind the steward's `0 * * * *`. Citations authored by a
 * steward tick are then auditable within the same hour without the two ticks writing concurrently
 * over one map. Single-flight and lease-reaping live in the server, so a fixed cadence is safe.
 */
export default defineSchedule({
  cron: "30 * * * *", // hourly at :30, UTC — trailing the steward's tick; the server gates the rest
  async run({ receive, waitUntil, appAuth }) {
    waitUntil(
      (async () => {
        const correlationId = crypto.randomUUID();
        console.log(`[auditor-dispatch] tick ${correlationId} starting (temper-ts ${TEMPER_TS_VERSION})`);
        try {
          const apiUrl = requireEnv("TEMPER_API_URL").replace(/\/+$/, "");

          const res = await auditorFetch(
            `${apiUrl}/api/auditor/dispatch`,
            {
              method: "POST",
              headers: {
                "content-type": "application/json",
                "x-auditor-correlation-id": correlationId,
              },
              // Empty body → the server's default finding cap.
              body: "{}",
            },
            { label: "auditor-dispatch" },
          );
          if (!res.ok) {
            throw new Error(`auditor dispatch failed: ${res.status} ${await res.text()}`);
          }

          const dispatchVercelId = res.headers.get("x-vercel-id") ?? "unknown";

          const { claimed, correlation_id: stampedId } = (await res.json()) as {
            claimed: { id: string; cogmap_id: string; findings: string[] }[];
            correlation_id?: string;
          };

          // The server echoes the correlation it parsed and stamped. A mismatch (or an absent echo)
          // means the tick's DB-side trace is broken even though the log trace is intact — the jobs
          // self-rooted and their sessions' invocations will inherit nothing. Never fatal:
          // correlation is provenance, and the audit work should still run.
          if (stampedId !== correlationId) {
            console.warn(
              `[auditor-dispatch] tick ${correlationId}: server stamped ${stampedId ?? "<none>"}; ` +
                `this tick's jobs and invocations will not carry it`,
            );
          }

          console.log(
            `[auditor-dispatch] tick ${correlationId}: claimed ${claimed.length} job(s)` +
              (claimed.length
                ? `: ${claimed.map((j) => `${j.id}→${j.cogmap_id}(${j.findings.length} finding(s))`).join(", ")}`
                : " (no uncovered citations)") +
              ` (dispatch vercel-id ${dispatchVercelId})`,
          );

          // A job with an empty finding list is not work. It can only arise from a payload written by
          // an older shape, and delegating it would spend a model session to discover that.
          const workable = claimed.filter((job) => job.findings.length > 0);
          if (workable.length !== claimed.length) {
            console.warn(
              `[auditor-dispatch] tick ${correlationId}: ${claimed.length - workable.length} claimed ` +
                `job(s) carried no findings and were skipped`,
            );
          }

          await Promise.all(
            workable.map((job) =>
              receive(auditorWorker, {
                target: {},
                auth: appAuth,
                message: auditSessionPrompt(job),
              }),
            ),
          );
        } catch (err) {
          console.error(`[auditor-dispatch] tick ${correlationId} failed:`, err);
          throw err;
        }
      })(),
    );
  },
});

/**
 * The dispatch instruction for ONE claimed job.
 *
 * It is deliberately thin. The root session it lands in is the STEWARD agent — same instructions,
 * same model, same connection — and none of that may touch an audit. So this message's only job is
 * to hand the work straight to the `auditor` declared subagent, which has its own instructions
 * (`subagents/auditor/instructions.md`), its own model, and its own temper connection under the
 * auditor's credential. Everything about *how to audit* lives there, not here: a prompt duplicated
 * across a schedule and an instructions file is a prompt that will drift.
 */
function auditSessionPrompt(job: { id: string; cogmap_id: string; findings: string[] }): string {
  return (
    `This is a CITATION AUDIT dispatch, not a stewardship tick. Do not call any temper tool ` +
    `yourself and do not distill anything. Make exactly one tool call — the \`auditor\` subagent — ` +
    `passing it the message below verbatim, then stop and report what it returned.\n\n` +
    `---\n` +
    `Audit the citations of the findings listed below, which are homed in cognitive map ` +
    `${job.cogmap_id} (dispatch job ${job.id}). They were selected by the deterministic coverage ` +
    `sweep because they carry cited sources no auditor has yet weighed, so you do not need to ` +
    `re-check whether they need auditing. Work them in the order given — the list is ordered by ` +
    `how much of each finding's evidence is still unweighed.\n\n` +
    `Findings to audit (${job.findings.length}):\n` +
    job.findings.map((f) => `- ${f}`).join("\n")
  );
}
