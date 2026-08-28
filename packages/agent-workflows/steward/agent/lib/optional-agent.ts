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
 *   capacity     — refused means SKIP.   [`tokenIssuanceUnavailable`] below.
 *
 * The credential axis stays in `temper-auth.ts` because it is genuinely auth: it is derived from the
 * same `CredentialEnv` shape `build` reads, in the same order, so the two cannot drift on what
 * "configured" means. The other two are not auth — they are the guard — so they live here.
 *
 * All three share one contract, and it is the incumbent's: **a skip, never a fallback.** None of
 * them widens what may authenticate as the auditor, and none of them starts the tick as somebody
 * else. The fix for "cannot run" is to not start.
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

/**
 * The status an issuer answers with when it will not mint for a credential it otherwise accepts.
 *
 * **429, and deliberately not the AI Gateway's 402.** These are two different vendors failing two
 * different ways: the gateway's 402 is a credit balance and is raised at the model call; this is
 * Auth0's monthly quota on machine-to-machine token issuance and is raised at the token endpoint.
 * Matching 402 here — the obvious guess, since both are "out of money" — would match a status Auth0
 * never sends.
 */
const ISSUANCE_REFUSED_STATUS = 429;

/**
 * Did a token mint fail because the issuer would not mint RIGHT NOW, rather than because the
 * credential is wrong?
 *
 * **Correct credentials are not sufficient credentials.** Auth0 enforces its own monthly quota on
 * M2M token issuance, so the auditor's credentials can be entirely correct and still be rejected.
 * That is a funding ceiling, not a misconfiguration, and it belongs in the same quiet skip as an
 * absent credential: an optional agent that cannot run right now.
 *
 * **The status line is the whole predicate, and that is a decision rather than a shortcut.**
 *
 * - **It is enough.** A wrong credential is `401 invalid_client`, from Auth0 and from temper's own
 *   AS alike. So the distinction the guard exists to draw — "cannot afford to run" quiet, "believes
 *   it is auditing and is not" loud — falls out of the status without reading a body at all.
 * - **Quota and throttling are not separated, and need not be.** Both are 429 and both mean the same
 *   thing to this caller. Auth0 documents quota headers as the discriminator, but they are
 *   unreachable twice over: `TokenMintError` carries only a status, and an instance's own
 *   `/oauth/token` proxy rebuilds the response keeping content-type and cache-control alone.
 * - **The body is log detail, never a predicate.** Matching `error_description` would couple this to
 *   vendor prose, and confirming that prose against the live issuer would cost a token from the very
 *   quota being probed. Do not pre-check by minting one.
 *
 * **This needs no deployment-profile branch, and that is not an oversight.** A temper-issued `tmpr_`
 * credential never transits Auth0, and temper's own AS has no 429 in its vocabulary — it answers 400,
 * 401 and 503. So on a self-hosted instance this predicate is simply never true, and the behaviour
 * degrades to what it was before rather than assuming an Auth0 error vocabulary everywhere.
 *
 * Checked STRUCTURALLY rather than with `instanceof`: the error is raised by `temper-ts` and crosses
 * a package boundary, where a duplicated module identity would make `instanceof` quietly false. The
 * name check is also what keeps this narrow — a 429 from temper's own API is rate limiting rather
 * than a funding ceiling, and arrives as a plain `Error` that must stay loud.
 */
export function tokenIssuanceUnavailable(err: unknown): boolean {
  if (typeof err !== "object" || err === null) {
    return false;
  }
  const { name, status } = err as { name?: unknown; status?: unknown };
  return name === "TokenMintError" && status === ISSUANCE_REFUSED_STATUS;
}
