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

export function resolveModelConfig(env: NodeJS.ProcessEnv = process.env): ModelConfig {
  const primary = env.STEWARD_MODEL?.trim() || DEFAULT_MODEL;

  const raw = env.STEWARD_MODEL_FALLBACKS;
  const configured =
    raw === undefined
      ? [...DEFAULT_FALLBACKS]
      : raw
          .split(",")
          .map((entry) => entry.trim())
          .filter((entry) => entry !== "");

  // Dedupe, and drop the primary: the gateway walks this list only AFTER the primary fails, so
  // repeating it there just re-tries a model that has already failed.
  const fallbacks = [...new Set(configured)].filter((model) => model !== primary);

  return { primary, fallbacks };
}
