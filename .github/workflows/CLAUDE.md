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

Shared CI behavior lives in composite actions (`.github/actions/install-onnx`,
`.github/actions/setup-rust`) rather than being copy-pasted per job — the ONNX install had
drifted into **five** near-identical copies.
