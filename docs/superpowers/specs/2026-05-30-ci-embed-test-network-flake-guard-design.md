# Design: timeout-isn't-failure guard for network-dependent integration tests

## Problem

The `test-typescript / Integration Tests` CI job intermittently fails on a single
test:

```
× fixture embedding > embeds chunked markdown to 768-dim vectors  120006ms
  → Test timed out in 120000ms.
```

Every other test in `packages/temper-cloud/tests/integration/pipeline.test.ts`
passes in milliseconds. The embedding test is the only one that reaches the
network: `embedTexts()` (`src/processing/embed.ts`) pulls the
`BAAI/bge-base-en-v1.5` tokenizer (`AutoTokenizer.from_pretrained`) and the
~400 MB ONNX model file (`@huggingface/hub` `downloadFile`) from the HuggingFace
Hub. On CI `/tmp` is cold every run, so it always downloads.

The test already has a guard:

```ts
try {
  embeddings = await embedTexts(texts);
} catch (err) {
  if (isNetworkConnectivityError(err)) {
    ctx.skip("HuggingFace Hub unreachable — embedding model could not be pulled");
  }
  throw err;
}
```

But that guard only fires when the network call **throws**. The actual failure
mode is different: when the Hub is slow, the download **hangs** with no error
thrown, the `await` never resolves, and vitest's 120 s `testTimeout` kills the
test — a hard red, not a caught network error. The catch never runs. That is the
gap this design closes.

This is environment flake, not a defect. A slow third-party model host should
degrade to a skipped test, not a failed build.

## Goal

Convert "the model pull hung past our patience" from a hard test failure into a
**skipped-with-reason** test, alongside the existing "the model pull threw a
network error" case — without weakening the assertion when the network behaves.

## Non-goals / rejected

- **Pre-warming the model in a CI step (cache).** Considered (it reduces flake
  frequency by warming `/tmp` before tests) but rejected for now: it adds CI
  machinery, the warm step can itself flake, and it doesn't address the "treat
  a hang as a skip, not a failure" ask directly. Can be layered on later if
  flake frequency warrants it; this design is independent of that choice.
- **Raising `testTimeout`.** Moves the threshold without changing failure-vs-skip
  semantics — the build still goes red on a true hang.
- **Gating the embedding test out of the default job.** Removes real coverage
  whenever the network *is* healthy.

## Approach

A small, reusable test helper that wraps any network-dependent async operation
and degrades both failure modes (throw and hang) to `ctx.skip`.

### Component: `tests/integration/helpers/network.ts`

New shared module. Two exports:

1. **`isNetworkConnectivityError(err: unknown): boolean`** — moved verbatim out
   of `pipeline.test.ts` (it is network-detection logic and belongs with the
   helper). Walks the `.cause` chain looking for known undici/DNS error codes
   (`UND_ERR_CONNECT_TIMEOUT`, `ECONNRESET`, `ENOTFOUND`, `EAI_AGAIN`,
   `ETIMEDOUT`, …) or `fetch failed` / `getaddrinfo` / `network` message
   patterns.

2. **`runOrSkipOnNetworkFlake<T>(ctx, label, fn, opts?): Promise<T>`**
   - `ctx`: the vitest test context (carries `.skip(reason)`).
   - `label`: human string used in the skip reason, e.g. `"embedding model pull"`.
   - `fn: () => Promise<T>`: the network operation.
   - `opts.budgetMs`: internal timeout, default **`NETWORK_BUDGET_MS = 90_000`**
     (constant in the module). Chosen below vitest's 120 s `testTimeout` so
     **our** timer wins the race and produces a skip before vitest produces a
     failure, with ~30 s of headroom for a healthy-but-slow download.

   Behavior:
   - Starts `fn()` and races it against a `budgetMs` timer.
   - **Timer wins** → `ctx.skip(\`${label}: exceeded ${budgetMs}ms budget — treating as infra flake, not a failure\`)`.
   - **`fn()` rejects with a network error** (`isNetworkConnectivityError`) →
     `ctx.skip(\`${label}: HuggingFace Hub unreachable — model could not be pulled\`)`.
   - **`fn()` resolves** → returns its value; the caller's assertions run.
   - **`fn()` rejects with any other error** → rethrows (real defects still fail).
   - Attaches a no-op `.catch()` to the work promise so a late rejection after
     the timer already won does not surface as an `unhandledRejection`.
   - `clearTimeout` in a `finally` so the timer never leaks an open handle that
     keeps the process alive.

`ctx.skip(reason)` aborts the test by throwing vitest's skip signal, so once it
is called the helper does not return past it; the trailing `throw` provides the
non-network fallthrough and keeps the function's return type sound.

### Consumer: `pipeline.test.ts`

The embedding test's inline try/catch is replaced by one call:

```ts
const embeddings = await runOrSkipOnNetworkFlake(
  ctx,
  "embedding model pull",
  () => embedTexts(texts),
);
```

All existing assertions (`embeddings.length`, per-vector `EMBEDDING_DIM`,
L2-norm ≈ 1.0) are unchanged. The local `isNetworkConnectivityError` definition
is deleted in favor of the import.

## Visibility (silent-skip mitigation)

The one cost of skip-on-flake is that a *genuine* embedding slowdown (real perf
regression) would also read as a skip rather than a failure. Mitigations:

- The skip always carries a descriptive **reason**, so vitest reports it as
  *skipped-with-reason*, not a silent pass.
- A real regression manifests as a **recurring** skip on that specific test/line
  — visible in CI output run over run, not invisible.
- The 90 s budget is generous enough that only a true hang (or a multi-minute
  regression) trips it; ordinary healthy downloads finish well under it.

## Testing

A network-free **unit** test for the helper (`tests/network.test.ts` — the
unit suite globs `tests/**/*.test.ts` minus `tests/integration/**`, so a file
here runs under `bun run test`), using a fake `ctx` (records `skip` calls) and
fast fake `fn`s with a tiny `budgetMs` so it runs in milliseconds. Cases:

- `fn` resolves → helper returns the value; `ctx.skip` not called.
- `fn` rejects with a synthetic network error (code `ENOTFOUND`) → `ctx.skip`
  called with an "unreachable" reason; value not returned.
- `fn` hangs past `budgetMs` → `ctx.skip` called with an "exceeded budget"
  reason.
- `fn` rejects with a non-network error → helper rethrows; `ctx.skip` not called.
- `isNetworkConnectivityError`: true for a wrapped `.cause` chain carrying a
  known code; false for an ordinary `Error`.

The integration test itself remains the live, end-to-end exercise of the real
model pull whenever the network cooperates.

## Delivery

- Lands as its **own branch/PR off `main`** (`jct/ci-embed-test-network-flake-guard`),
  unrelated to the in-flight dead-code-sweep PR (#108).
- #108's current red is the *same flake*; it is unblocked by re-running its
  failed job, independently of this hardening.

## Files touched

- `packages/temper-cloud/tests/integration/helpers/network.ts` — new helper +
  moved `isNetworkConnectivityError`.
- `packages/temper-cloud/tests/integration/pipeline.test.ts` — use the helper;
  drop the inline guard.
- `packages/temper-cloud/tests/network.test.ts` — new unit test.
