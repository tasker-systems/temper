# CI workflows

## CI runs everything, by construction

Jobs are split by **intention** (what they need from the environment), never by feature
flag: **Unit** (no DB) · **Integration & E2E** (Postgres + LFS — the whole DB-backed
workspace in ONE `--workspace` command) · **Substrate Artifacts** (a different feature
set). Coverage is nightly (`coverage.yml`), out of the PR path, so an instrumented-build
OOM can never block a merge.

There is **no "the job with ONNX"** any more — that was a historical constraint and it is
gone. Confining `test-embed` to one job is precisely what let `streaming_ingest_test` rot:
its tests were *compiled out* of the integration job and *filtered out* of the embed job's
allowlist, so they ran **nowhere**, and a 484-second test hid behind a green tick for
months.

**Never add a `-E 'binary(...)'` filter to a CI test job.** Selection is `--workspace` so a
new crate or test is picked up with no CI edit. A filter that makes CI green is hiding a
test, not fixing one.

## Two jobs disagree about `VERCEL_GIT_COMMIT_SHA` on purpose — do not harmonize them

The **Unit** job sets it to a fixed synthetic sha; **Integration** and **Artifacts** leave
it unset. That is not drift. `crates/temper-api/build.rs` reads it at *compile* time and
bakes it into `/api/health`, so the two settings compile two different binaries and each
one is the only environment in which half of the health witness can fail:

- **told** (Unit) — the served commit must equal what the build was told. Catches a dead
  `build.rs`, a drifted `TEMPER_BUILD_COMMIT` name, a handler that stopped reading it.
- **not told** (Integration) — the body must carry `"commit": null`, **key present**. This
  is the only branch that can catch `skip_serializing_if = "Option::is_none"`, which drops
  `None` alone; with a commit always present the key is there no matter what.

Setting it everywhere looks tidy and silently retires the second branch. This witness spent
its whole existence passing in every environment that ran it — an early `return` on an unset
variable, so every job ran it and no job could fail it. Same failure as the `-E` filter
above, wearing an environment variable instead.

Unrelated to `.github/scripts/test-vercel-build.sh`, which `env -u`s the same name in
`guard-tests`. That clears a **runtime** value for a sandboxed `sh` run; this sets a
**build-time** value for a compiler. Both must keep doing what they do.

Shared CI behavior lives in composite actions (`.github/actions/install-onnx`,
`.github/actions/setup-rust`) rather than being copy-pasted per job — the ONNX install had
drifted into **five** near-identical copies.
