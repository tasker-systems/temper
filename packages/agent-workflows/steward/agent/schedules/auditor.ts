import { defineSchedule } from "eve/schedules";
import { TEMPER_TS_VERSION, type components } from "temper-ts";

import auditorWorker from "../channels/auditor-worker.js";
import { AUDITOR_ENABLED, agentEnabled, tokenIssuanceUnavailable } from "../lib/optional-agent.js";
import {
  AUDITOR_CREDENTIALS,
  auditorFetch,
  credentialConfigured,
  requireEnv,
} from "../lib/temper-auth.js";

/**
 * ═══ ENABLED 2026-07-29, after all four gates below closed and the grain fix shipped. ═══
 *
 * eve registers exactly one Vercel Cron Job per file under `agent/schedules/`
 * (`node_modules/eve/docs/schedules.mdx`: *"Each one is a single file under `agent/schedules/`
 * carrying a cron expression"*), so this file's LOCATION is the on/off switch. It lived outside
 * the agent root from 2026-07-25 until now for exactly that reason.
 *
 * **Withdrawing the CAPABILITY still means moving the file back out.** That is a repo-wide decision
 * — it unregisters the cron for every deployment — and it stays a `git mv` plus an operator
 * decision, the same way restoring it was. The guards in `run` answer a narrower question: whether
 * THIS deployment's tick does work. `TEMPER_AUDITOR_ENABLED` is the per-deployment off switch, and
 * it deliberately leaves the auditor runnable on demand, which is exactly what unsetting a
 * credential cannot do. Neither guard is a substitute for the other.
 *
 * **The history this file exists to not repeat.** It shipped enabled in PR #531 and began firing in
 * production on 2026-07-24T23:16Z. Every tick died at `requireEnv("TEMPER_AUDITOR_TOKEN")` before
 * its outbound fetch — ~18 consecutive failures, unannounced, while the subsystem was still being
 * designed. Two separate lessons came out of that, and both are now structural rather than
 * remembered: enabling a production cron is an operator decision (hence the `git mv`), and an
 * *absent* optional credential must no-op rather than fail (hence the guard in `run`). The
 * credential requirement is real (see below), documented in
 * `docs/auth/machine-token-contract.md` §C, and was not surfaced as a deploy action item at the
 * time: `DEPLOYING.md` mentions the auditor only for its migration cutover.
 *
 * **A configured auditor still needs all FOUR gates below.** The guard in `run` skips a deployment
 * that has no auditor at all; it does nothing for one that has a credential and is missing a gate.
 * Those still fail loudly, which is correct — they are misconfigurations, not absences. The token
 * is only the first of the four. (This list said "three" until 2026-07-27 and omitted gate 3 —
 * which was already true when the list was written, D11 having landed five days earlier. All four
 * are CLOSED on temperkb.io; the list is kept because this instance is not the only deployment,
 * and a fork standing up an auditor walks the same four.)
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
 * **The trigger-model redesign that once blocked this is DONE, not in flight** — recorded here
 * because "is the trigger model settled?" is the first question anyone auditing this cron asks: task
 * `019f975e-7be9-7ff3-a5bd-ef7ea72ff4a5` closed 2026-07-26 with tier 1 shipped
 * (`20260726000010_auditor_tier1_staleness.sql`, selection is `uncovered OR stale`), and tier 2 was
 * deferred onto `019f9bb3-e2cf-7710-9b90-db4ebefb8f64`, which closed 2026-07-27 retargeted onto the
 * steward (PR #557). F9 — "verify pg_uuidv7 ordering on Neon" — was **dissolved**, not verified:
 * rev. 3 of the design changed the comparand to a timestamp, so no generator dependency remains.
 *
 * The payload-grain defect that stood here — the last build item — is **CLOSED**. The dispatch
 * payload was finding-grained (`AuditJobPayload::findings`, `ClaimedAuditJob::findings`) while the
 * unit of work is a citation, and the prompt below *actively forbade* the re-check that would have
 * compensated ("you do not need to re-check whether they need auditing"). Measured on production
 * 2026-07-27: finding `019f5cdd-ba0b-7fa2-90b1-e78ca7979254` was audited on one of its 8 citations
 * and was STILL returned by the sweep at `uncovered: 7`, so a restored hourly tick would have
 * re-dispatched it and the agent emitted verdicts on all 8. Register §3.
 *
 * Fixed by `20260727000050_auditable_citations_at_citation_grain.sql` and task
 * `019fa5eb-2819-7e71-bd27-0d2875f60960`: the server expands each swept finding through
 * `resource_auditable_citations` before it reaches the payload, so a session is handed only
 * citations THIS principal has not already weighed. The invariant is now held by what the session
 * receives rather than by an instruction it is asked to follow — which is why the prompt below no
 * longer tells the agent anything about re-checking. There is nothing left to re-check.
 *
 * The cadence and the selection predicate below are settled, and no build item stands behind this
 * cron any more. It was restored by an operator decision taken separately from the grain work
 * landing — deliberately not as a consequence of it.
 *
 * ─────────────────────────────────────────────────────────────────────────────────────────────
 *
 * Citation-auditor fan-out dispatcher (Set 5; spec
 * `temper-artifacts:specs/2026-07-23-set5-adversary-citation-audit-design.md` §6).
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
 *    citation list its job carries. That list exists because the queue's single-flight index is
 *    `(cogmap_id, persona, dispatch_type)` while the auditor's unit of work is a citation — so the
 *    server groups by cogmap and puts the citations in the payload rather than enqueuing per
 *    finding, which `ON CONFLICT DO NOTHING` would silently collapse to one (spec §6.1). Since
 *    `20260727000050` the list is citation-grained and pre-filtered per principal: every entry is
 *    work this credential has not already done.
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
 * **Cadence is the budget mechanism, not a tuning preference.** The AI Gateway gets a fixed monthly
 * allowance and that allowance IS the ceiling — topping it up is not the plan. Cadence is how the
 * auditor is made to fit inside it. Hourly was also ~24x this corpus's own rate of change, so the
 * extra 23 ticks bought re-derivation rather than coverage.
 *
 * Two costs, and cadence bounds them differently. The **sweep** runs as phase 2 of every dispatch
 * whatever the queue holds, so cadence bounds it unconditionally. **Model spend** is proportional to
 * jobs actually claimed — a tick that claims none starts no session and spends nothing — so cadence
 * bounds it only while there is standing work. There was, which is why this mattered.
 *
 * **Cadence and `DEFAULT_AUDITOR_DISPATCH_CAP` are one knob in two halves, and the second is not
 * set here.** That cap (50, `temper-core::types::workflow_job`) is a per-tick *finding* budget and
 * simultaneously the sweep's `p_limit` and the claim's batch limit, so ticks/day × cap is the
 * ceiling on audit throughput — daily at 50 admits 50 findings/day where hourly admitted up to 1200.
 * It does not bind at this corpus's scale: with a substantial backlog standing, `audit_drift_sweep`
 * at `p_limit` 50 has been measured returning **15** eligible findings — demand well under supply,
 * which is the comparison that matters. A fork whose steward drifts more than ~50 findings a day
 * needs the cap raised or the cadence raised, or its backlog grows behind a green cron. Raising the
 * cap costs model sessions but NOT sweep time — `p_limit` applies after every candidate is scored.
 *
 * The same coupling slows recovery, and that was accepted rather than missed: reaping happens on any
 * tick but only an auditor tick re-claims, so a crashed job retries in ≤24h instead of ≤1h and takes
 * three days rather than three hours to exhaust `max_attempts`. That is the churn-coalescing the
 * spec's §4.2 named as a reason backoff is *less* urgent at this cadence, seen from the other side.
 *
 * **The committed value is every fork's default, so moving it is an operator decision.** It was
 * hourly at :30 from 2026-07-29 until 2026-08-28. The MINUTE is load-bearing and unchanged: :30
 * trails the steward's `0 * * * *` so citations authored by a steward tick stay auditable without
 * the two ticks writing concurrently over one map. The HOUR is not load-bearing — an operator with a
 * larger allowance may raise the frequency, and one with none should reach for the enable toggle
 * below rather than a cron nobody can read as deliberate. Single-flight and lease-reaping live in
 * the server, so any fixed cadence is safe.
 */
/**
 * The claimed-job wire shape, taken from the generated OpenAPI client rather than hand-mirrored.
 *
 * A structural duplicate of this shape is what let the payload grain drift here unnoticed once
 * already: the schedule went on reading `findings` because nothing tied its hand-written type to
 * the Rust one. Now a grain change downstream restales this file at typecheck.
 */
type ClaimedAuditJob = components["schemas"]["ClaimedAuditJob"];

export default defineSchedule({
  cron: "30 3 * * *", // daily at 03:30 UTC — trailing a steward tick; see "Cadence" above
  async run({ receive, waitUntil, appAuth }) {
    // An operator may hold auditor credentials and still want agent maintenance off — the credential
    // axis cannot express that, because unsetting the credential also removes the ability to run the
    // auditor on demand. This is checked FIRST because it is the only axis that carries an EXPLICIT
    // decision: an operator who wrote the variable outranks anything inferred from absence, and the
    // credential guard's log line below would otherwise tell them to set a credential they already
    // have. **Absence means ENABLED** — see `optional-agent.ts` for why that polarity is deliberate.
    if (!agentEnabled(AUDITOR_ENABLED)) {
      console.log(
        `[auditor-dispatch] ${AUDITOR_ENABLED} turns agent maintenance off on this deployment — ` +
          "skipping tick. Unset it, or set it to 1/true/on, to resume. This is a deliberate no-op, " +
          "not a failure.",
      );
      return;
    }

    // The auditor is OPTIONAL; the steward is not. This file ships in the repo, and eve registers
    // one Vercel Cron Job per file here — so every fork and self-hosted deploy of this agent gets
    // this cron whether or not it runs an auditor. Without this guard each of those deployments
    // fails on every tick, forever, on a credential they never intended to set. That is not a
    // signal, it is noise that teaches people to ignore a red cron. (Stated per-tick rather than
    // per-hour deliberately: the cadence above is an operator's to move, and this argument does not
    // depend on it.)
    //
    // **This is a skip, never a fallback.** It does not widen what may authenticate as the auditor:
    // `auditorFetch` still throws rather than borrowing the steward's credential, and
    // `tests/auditor.test.ts` pins that. A borrowed credential would make every steward-authored
    // finding self-authored to the auditor and collapse the adversarial premise entirely — so the
    // fix for "unconfigured" is to not start, never to start as somebody else.
    //
    // Only TOTAL absence skips. A partially-configured auditor (client id set, secret missing)
    // passes this check and then throws in `build` — deliberately, because that is a
    // misconfiguration by someone who *meant* to run an auditor, and silence there is the far worse
    // failure: a deployment that believes it is auditing and is not.
    if (!credentialConfigured(AUDITOR_CREDENTIALS)) {
      console.log(
        "[auditor-dispatch] no auditor credential on this deployment — skipping tick. " +
          "Set TEMPER_AUDITOR_M2M_CLIENT_ID (+ _SECRET, _TOKEN_URL) to enable it, or " +
          "TEMPER_AUDITOR_TOKEN for local dev. This is a deliberate no-op, not a failure.",
      );
      return;
    }

    waitUntil(
      (async () => {
        const correlationId = crypto.randomUUID();
        console.log(`[auditor-dispatch] tick ${correlationId} starting (temper-ts ${TEMPER_TS_VERSION})`);
        // Whether the dispatch call got far enough to claim and LEASE work server-side. The quiet
        // skip below asserts that it did not, and this is what makes that assertion structural
        // rather than a claim about which frames can raise a mint failure. If one ever arrives after
        // the claim, it goes loud — the safe direction, because the alternative is a log line saying
        // nothing was claimed while N cogmap jobs sit leased.
        let workClaimed = false;
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
          // The server ran reap → sweep → group → enqueue → claim before answering, so anything
          // claimed is claimed and leased by the time this line runs.
          workClaimed = true;

          const dispatchVercelId = res.headers.get("x-vercel-id") ?? "unknown";

          const { claimed, correlation_id: stampedId } = (await res.json()) as {
            claimed: ClaimedAuditJob[];
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
                ? `: ${claimed
                    .map(
                      (j) =>
                        `${j.id}→${j.cogmap_id}(${j.citations.length} citation(s) across ` +
                        `${new Set(j.citations.map((c) => c.finding_id)).size} finding(s))`,
                    )
                    .join(", ")}`
                : " (no auditable citations)") +
              ` (dispatch vercel-id ${dispatchVercelId})`,
          );

          // A job with an empty citation list is not work. Since `20260727000050` this is the shape a
          // pre-deploy payload deserializes to — `AuditJobPayload::citations` is `#[serde(default)]`,
          // so an older job claims successfully and arrives here empty rather than failing the claim.
          // Delegating it would spend a model session to discover it has nothing to weigh; skipping
          // costs nothing, because the sweep re-offers those findings on the next tick.
          const workable = claimed.filter((job) => job.citations.length > 0);
          if (workable.length !== claimed.length) {
            console.warn(
              `[auditor-dispatch] tick ${correlationId}: ${claimed.length - workable.length} claimed ` +
                `job(s) carried no citations and were skipped`,
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
          // The THIRD axis of the optional-agent guard, and the one the credential check above
          // cannot see. `credentialConfigured` reads environment variables: it establishes that
          // somebody INTENDED to run an auditor, never that the auditor CAN run. A deployment whose
          // issuance quota is spent holds credentials that are entirely correct and still gets them
          // refused — configured-and-cannot-mint passes the guard and then fails here.
          //
          // Classified rather than pre-checked: minting a probe token to ask whether a token can be
          // minted spends the very quota being probed.
          //
          // **This `catch` cannot be taught to handle the AI Gateway's 402, and adding a branch for
          // it would be dead code.** `receive()` above resolves when a session's workflow run is
          // STARTED, not when it completes — eve's `Session` is an inert, non-thenable result value
          // — so the 402, and every other in-session failure, is raised past this frame in a
          // separate durable run. The same is true of the auditor subagent's own mint. Only not
          // making the model call quiets those, which is the cadence and the toggle above.
          //
          // `console.warn`, where the other two skips log — deliberately, and the difference is
          // their epistemic status. An absent credential and an explicit toggle are RESTING states:
          // the deployment is configured that way and nothing is wrong. This is a DEGRADED one —
          // somebody configured an auditor, it worked, and it is not working now. It must not redden
          // the cron, but it must not read as normal either, and at a daily cadence there is one
          // line a day to notice. Quiet is not the same as invisible.
          if (!workClaimed && tokenIssuanceUnavailable(err)) {
            console.warn(
              `[auditor-dispatch] tick ${correlationId}: the token endpoint will not mint for this ` +
                "credential right now (issuance quota). Agent maintenance is optional; skipping " +
                "this tick. This is a deliberate no-op, not a failure — no work was claimed. It " +
                "resumes on its own when the quota does; a run of these means the auditor has not " +
                "audited for as long as they have been appearing.",
            );
            return;
          }
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
function auditSessionPrompt(job: ClaimedAuditJob): string {
  const findingCount = new Set(job.citations.map((c) => c.finding_id)).size;
  return (
    `This is a CITATION AUDIT dispatch, not a stewardship tick. Do not call any temper tool ` +
    `yourself and do not distill anything. Make exactly one tool call — the \`auditor\` subagent — ` +
    `passing it the message below verbatim, then stop and report what it returned.\n\n` +
    `---\n` +
    `Weigh the citations listed below, which belong to findings homed in cognitive map ` +
    `${job.cogmap_id} (dispatch job ${job.id}). This list IS your work and it is complete: the ` +
    `server resolved it through the coverage sweep and then removed every citation you have ` +
    `already weighed, so each entry is one you have not yet given a verdict on, or one that has ` +
    `changed since you did. Work them in the order given; it is ordered by how much of each ` +
    `finding's evidence is still unweighed.\n\n` +
    `Citations to weigh (${job.citations.length} across ${findingCount} finding(s)), as ` +
    `finding · block · source:\n` +
    job.citations.map((c) => `- ${c.finding_id} · ${c.block_id} · ${c.source_id}`).join("\n")
  );
}
