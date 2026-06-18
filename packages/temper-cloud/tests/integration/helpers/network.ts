import type { TestContext } from "vitest";

/**
 * Default budget for a network-dependent operation before we treat it as infra
 * flake. Deliberately below the integration suite's 120s `testTimeout` so that
 * *our* timer wins the race and produces a skip, rather than vitest's timeout
 * producing a hard failure. ~30s of headroom covers a healthy-but-slow model
 * download while still tripping well before vitest would.
 */
export const NETWORK_BUDGET_MS = 90_000;

/**
 * Does this error (or anything in its `.cause` chain) look like a network
 * connectivity failure rather than a defect? `fetch failed` wraps the
 * underlying undici/DNS error as `.cause`, so we walk the chain.
 */
export function isNetworkConnectivityError(err: unknown): boolean {
  const NETWORK_CODES = new Set([
    "UND_ERR_CONNECT_TIMEOUT",
    "UND_ERR_HEADERS_TIMEOUT",
    "UND_ERR_SOCKET",
    "ECONNRESET",
    "ECONNREFUSED",
    "ENOTFOUND",
    "EAI_AGAIN",
    "ETIMEDOUT",
  ]);
  let current: unknown = err;
  for (let depth = 0; depth < 5 && current instanceof Error; depth++) {
    const code = (current as { code?: unknown }).code;
    if (typeof code === "string" && NETWORK_CODES.has(code)) return true;
    if (/fetch failed|connect timeout|getaddrinfo|network/i.test(current.message)) return true;
    current = (current as { cause?: unknown }).cause;
  }
  return false;
}

/**
 * Does this error (or anything in its `.cause` chain) look like the HuggingFace
 * Hub throttling or briefly refusing a model-file download — HTTP 429
 * (rate-limited) or 503 (temporarily unavailable)? transformers.js surfaces
 * these as `Error (429) occurred while trying to load file: "https://huggingface.co/...">`,
 * which carries no `.code` and whose message doesn't match the connectivity
 * patterns above, so it would otherwise slip through as a hard failure. Like a
 * connectivity failure this is infra flake (the Hub is up but shedding load),
 * not a defect, so the guard skips rather than fails the build.
 */
export function isHubThrottleError(err: unknown): boolean {
  let current: unknown = err;
  for (let depth = 0; depth < 5 && current instanceof Error; depth++) {
    if (
      /\b(429|503)\b/.test(current.message) &&
      /huggingface\.co|trying to load file/i.test(current.message)
    ) {
      return true;
    }
    current = (current as { cause?: unknown }).cause;
  }
  return false;
}

/** Internal sentinel: the operation outran its network budget. */
class NetworkBudgetExceeded extends Error {
  constructor(label: string, budgetMs: number) {
    super(`${label}: exceeded ${budgetMs}ms budget`);
    this.name = "NetworkBudgetExceeded";
  }
}

/**
 * Run a network-dependent operation, degrading each flake mode to a *skip*
 * rather than a failure:
 *
 * - the operation **hangs** past `budgetMs` (our timer wins the race),
 * - the operation **throws** a network connectivity error, or
 * - the HuggingFace Hub **throttles** the download (HTTP 429/503).
 *
 * Either way the test is skipped via `ctx.skip(reason)` (which aborts by
 * throwing vitest's skip signal). A genuine resolution returns the value so the
 * caller's assertions run; any non-network error is rethrown so real defects
 * still fail the build.
 */
export async function runOrSkipOnNetworkFlake<T>(
  ctx: TestContext,
  label: string,
  fn: () => Promise<T>,
  opts: { budgetMs?: number } = {},
): Promise<T> {
  const budgetMs = opts.budgetMs ?? NETWORK_BUDGET_MS;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const work = fn();
  // If the timer wins the race, `work` may still reject later; swallow it so it
  // does not surface as an unhandledRejection after the test has moved on.
  work.catch(() => {
    /* late rejection ignored — the race already settled */
  });

  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new NetworkBudgetExceeded(label, budgetMs)), budgetMs);
  });

  try {
    return await Promise.race([work, timeout]);
  } catch (err) {
    if (err instanceof NetworkBudgetExceeded) {
      ctx.skip(`${label}: exceeded ${budgetMs}ms budget — treating as infra flake, not a failure`);
    }
    if (isNetworkConnectivityError(err)) {
      ctx.skip(`${label}: HuggingFace Hub unreachable — model could not be pulled`);
    }
    if (isHubThrottleError(err)) {
      ctx.skip(
        `${label}: HuggingFace Hub throttled the download (429/503) — treating as infra flake`,
      );
    }
    throw err;
  } finally {
    if (timer) clearTimeout(timer);
  }
}
