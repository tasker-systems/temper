/**
 * The steward's model, resolved from configuration rather than baked into source.
 *
 * eve executes `agent.ts` at BUILD time (`compileAgentConfig`) and freezes the resolved model into
 * the compiled manifest — there is no session, no request context, and no DB anywhere near that
 * resolution. Env is therefore the only lever that exists, and a model change takes a REDEPLOY, not
 * a restart. That also means the primary is validated against the AI Gateway catalog at compile
 * time: a typo fails the build rather than a 3am cron tick.
 *
 * The fallbacks are NOT so validated — they ride through the compile untouched inside
 * `providerOptions`, so a typo there surfaces at runtime, only when it is actually needed.
 */

/**
 * Distillation and supersession are judgment-heavy, and sonnet/opus 4.x are the quality bar — but
 * their per-tick cost is not sustainable at the dev/community tier, where the loop runs hourly and
 * confidence comes from running it enough to trust it keeps working. minimax-m3 is a sound model at
 * ~10x lower cost with a matching 1M context window. Enterprise deployments override this with
 * `STEWARD_MODEL`.
 */
export const DEFAULT_MODEL = "minimax/minimax-m3";

/** In-family, ~3x cheaper, and a known-good tool-caller — the safe landing if minimax is unavailable. */
export const DEFAULT_FALLBACKS = ["anthropic/claude-haiku-4.5"] as const;

export interface ModelConfig {
  primary: string;
  /**
   * Tried IN ORDER after the primary fails, by the AI Gateway itself
   * (`providerOptions.gateway.models`). This covers AVAILABILITY — a 5xx, a rate limit, a model that
   * is gone. It cannot cover QUALITY: no gateway can tell that a model fumbled a tool sequence. The
   * mechanism for that is changing `STEWARD_MODEL` and redeploying, which is what making this
   * configurable buys.
   */
  fallbacks: string[];
}

/**
 * The auditor's default primary — **deliberately a different provider family from
 * [`DEFAULT_MODEL`]**, and that is the entire point of it being a separate constant.
 *
 * Spec §5.3: code colocation does not create epistemic dependence, but shared trained priors do,
 * and "the one lever that genuinely attacks shared trained priors — running the auditor on a
 * different model — is a config choice available under either layout, and is recommended." A
 * default that reproduced the steward's model would leave that lever un-pulled for every deployment
 * that never sets the env, which is most of them.
 */
export const DEFAULT_AUDITOR_MODEL = "anthropic/claude-haiku-4.5";

/**
 * Availability fallback for the auditor.
 *
 * **This deliberately lands on the steward's default primary, and that is a documented trade, not
 * an oversight.** Fallbacks cover availability, never quality (see [`ModelConfig.fallbacks`]) — and
 * an auditor that cannot run at all provides less independence than one that briefly runs on the
 * same family. A deployment that would rather the tick fail than silently collapse the two personas
 * onto one model sets `AUDITOR_MODEL_FALLBACKS=""`, which this resolver reads as an explicitly empty
 * list.
 */
export const DEFAULT_AUDITOR_FALLBACKS = ["minimax/minimax-m3"] as const;

/**
 * The shared resolution, parameterized by env prefix — one implementation, because the two personas'
 * config would otherwise drift apart silently (only one of them would gain a fix).
 */
function resolveFor(
  env: NodeJS.ProcessEnv,
  prefix: string,
  defaultModel: string,
  defaultFallbacks: readonly string[],
): ModelConfig {
  const primary = env[`${prefix}_MODEL`]?.trim() || defaultModel;

  const raw = env[`${prefix}_MODEL_FALLBACKS`];
  const configured =
    raw === undefined
      ? [...defaultFallbacks]
      : raw
          .split(",")
          .map((entry) => entry.trim())
          .filter((entry) => entry !== "");

  // Dedupe, and drop the primary: the gateway walks this list only AFTER the primary fails, so
  // repeating it there just re-tries a model that has already failed.
  const fallbacks = [...new Set(configured)].filter((model) => model !== primary);

  return { primary, fallbacks };
}

export function resolveModelConfig(env: NodeJS.ProcessEnv = process.env): ModelConfig {
  return resolveFor(env, "STEWARD", DEFAULT_MODEL, DEFAULT_FALLBACKS);
}

/**
 * The citation auditor's model (Set 5), configured through `AUDITOR_MODEL` /
 * `AUDITOR_MODEL_FALLBACKS`. Same build-time resolution and same redeploy-to-change semantics as the
 * steward's — eve freezes a declared agent's model into the compiled manifest, and a declared
 * subagent is a declared agent.
 */
export function resolveAuditorModelConfig(env: NodeJS.ProcessEnv = process.env): ModelConfig {
  return resolveFor(env, "AUDITOR", DEFAULT_AUDITOR_MODEL, DEFAULT_AUDITOR_FALLBACKS);
}

/**
 * `defineAgent`'s `modelOptions` for a fallback list.
 *
 * The AI Gateway tries the primary, then each fallback IN ORDER, returning the first that succeeds.
 * Omit the key entirely when there is nothing to fall back to — an empty list is not a policy.
 *
 * Lives here rather than in `agent.ts` because there are now two agents that need it (the root and
 * the `auditor` subagent), and a copy in each is a copy that will diverge.
 */
export function gatewayModelOptions(fallbacks: string[]) {
  return fallbacks.length === 0 ? {} : { providerOptions: { gateway: { models: fallbacks } } };
}
