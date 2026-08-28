/**
 * Whether an OPTIONAL agent should run on this tick — the axes that are not about credentials.
 *
 * `schedules/auditor.ts` is the whole reason this exists. That file ships in the repo and eve
 * registers one Vercel Cron Job per file under `agent/schedules/`, so every fork and self-hosted
 * deploy inherits the cron whether or not it wants an auditor. Separate questions decide whether a
 * given tick does any work, and they are deliberately different questions with deliberately
 * different defaults:
 *
 *   credential   — absent means SKIP.    `credentialConfigured` in `temper-auth.ts`.
 *   enablement   — absent means RUN.     [`agentEnabled`] below.
 *
 * The credential axis stays in `temper-auth.ts` because it is genuinely auth: it is derived from the
 * same `CredentialEnv` shape `build` reads, in the same order, so the two cannot drift on what
 * "configured" means. Enablement is not auth — it is the guard — so it lives here.
 *
 * They share one contract, and it is the incumbent's: **a skip, never a fallback.** Neither widens
 * what may authenticate as the auditor, and neither starts the tick as somebody else. The fix for
 * "cannot run" is to not start.
 */

/** The auditor's enable toggle. Named beside `AUDITOR_CREDENTIALS`, and read the same way: by name. */
export const AUDITOR_ENABLED = "TEMPER_AUDITOR_ENABLED";

/**
 * The values that turn an optional agent OFF. Nothing else does.
 *
 * Compared after `trim().toLowerCase()`, so `" FALSE "` disables and the operator does not have to
 * guess a canonical spelling.
 */
const DISABLING_VALUES = new Set(["0", "false", "off", "no"]);

/**
 * The values that explicitly turn it ON. Functionally redundant — absence already means enabled —
 * but an operator who writes `TEMPER_AUDITOR_ENABLED=true` deserves silence rather than the
 * unrecognized-value warning below.
 */
const ENABLING_VALUES = new Set(["1", "true", "on", "yes"]);

/**
 * Should the optional agent gated by `name` run?
 *
 * **Absence means ENABLED, and that is the inverse of every other environment predicate in this
 * package.** `credentialConfigured` and `otlpExportConfigured` are both presence checks — absent
 * means off. This one is an opt-OUT, deliberately: a deployment may hold auditor credentials and
 * still want agent maintenance off, which the credential axis cannot express (unsetting the
 * credential also removes the ability to run the auditor on demand). But every deployment auditing
 * today must keep auditing across the change that introduces this. **A merge may not turn a
 * production cron off any more than it may turn one on** — that lesson is why the schedule's own
 * location, not a flag, is the on/off switch for the cron itself.
 *
 * Three cases collapse to "run", and each one is a decision rather than a fallthrough:
 *
 *   - **Unset.** The overwhelming majority of deployments, including every one that predates this.
 *   - **Declared but empty.** Vercel surfaces a declared-with-no-value variable as `""`, not
 *     `undefined` — the same trap `credentialConfigured` documents. An empty declaration must not be
 *     the one keystroke that silently stops a deployment auditing.
 *   - **A value nobody recognizes.** `TEMPER_AUDITOR_ENABLED=fasle` keeps auditing and complains. A
 *     typo is not an operator decision, and a deployment that silently stopped for a reason nobody
 *     can see is the worse failure — the same argument that keeps a partially-configured credential
 *     loud instead of silently skipping it.
 */
export function agentEnabled(name: string): boolean {
  const raw = process.env[name];
  if (raw === undefined) {
    return true;
  }

  const value = raw.trim().toLowerCase();
  if (value === "" || ENABLING_VALUES.has(value)) {
    return true;
  }
  if (DISABLING_VALUES.has(value)) {
    return false;
  }

  console.warn(
    `[optional-agent] ${name} is set to a value this does not recognize; ` +
      `treating the agent as ENABLED. Set it to one of ${[...DISABLING_VALUES].join(", ")} to ` +
      "turn it off — an unreadable value is not taken as an instruction to stop.",
  );
  return true;
}
