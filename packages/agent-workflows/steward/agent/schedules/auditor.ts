import { defineSchedule } from "eve/schedules";
import { TEMPER_TS_VERSION } from "temper-ts";

import auditorWorker from "../channels/auditor-worker.js";
import { auditorFetch, requireEnv } from "../lib/temper-auth.js";

/**
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
