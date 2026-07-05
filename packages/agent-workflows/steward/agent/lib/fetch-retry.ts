/**
 * `fetch` with retry on 5xx / network errors, shared by the code schedules.
 *
 * Why the steward needs this: the temperkb serverless API (axum on Vercel) opens a fresh DB pool per
 * cold invocation, and Neon compute waking from idle can exceed the pool-acquire timeout — so the
 * function panics at startup (`Failed to connect to database: PoolTimedOut`) and returns 500. An
 * INFREQUENT caller like the hourly steward always hits a cold function, so its first request often
 * 500s; a retry warms both the function and Neon and then succeeds. This mirrors the read-path's
 * existing client-retry mitigation for the same cold-start flakiness.
 *
 * 4xx are NOT retried — those are real client errors (auth, bad request), not transient. Returns the
 * final `Response`; the caller inspects `res.ok`. Throws only after exhausting retries on 5xx/network.
 */

export interface RetryOptions {
  /** Total attempts, including the first (default 4). */
  attempts?: number;
  /** Backoff base in ms; delay before attempt N is `baseDelayMs * (N-1)` (default 1000 → 1s, 2s, 3s). */
  baseDelayMs?: number;
  /** Short label for log lines (default: the URL). */
  label?: string;
}

export async function fetchWithRetry(
  url: string,
  init: RequestInit,
  opts: RetryOptions = {},
): Promise<Response> {
  const attempts = opts.attempts ?? 4;
  const baseDelayMs = opts.baseDelayMs ?? 1000;
  const label = opts.label ?? url;
  let lastError = "";

  for (let attempt = 1; attempt <= attempts; attempt++) {
    try {
      const res = await fetch(url, init);
      // Success or a real client error (4xx) — return either; only 5xx is transient/retryable.
      if (res.status < 500) {
        return res;
      }
      lastError = `${res.status} ${await res.text()}`;
    } catch (err) {
      // Network-level failure (DNS, connection reset, …) — also transient.
      lastError = err instanceof Error ? err.message : String(err);
    }

    if (attempt < attempts) {
      const delayMs = baseDelayMs * attempt;
      console.log(
        `[fetch-retry] ${label} attempt ${attempt}/${attempts} failed (${lastError}); retrying in ${delayMs}ms`,
      );
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
  }

  throw new Error(`${label} failed after ${attempts} attempts: ${lastError}`);
}
