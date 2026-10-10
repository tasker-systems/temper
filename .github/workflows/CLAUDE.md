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

## Secret scan is unconditional, and the docs-only skip must never reach it

`secret-scan.yml` is invoked for every change with no scope gate — like CodeQL, but with a
starker reason: `detect-ci-scope.sh` lets pure-docs changes skip the whole pipeline, and **a
key pasted into a `.md` is exactly a leak**. The ci-success gate therefore reports it as
should-run `"true"` with the same "the job decides internally" reasoning CodeQL gets; adding
a scope output for it would duplicate the "never skips" decision in a second place and the
two copies would drift.

Its field of view is stated in the workflow header and in `.gitleaks.toml`: tracked content
at the checkout, **never history** (push protection is the history layer, and it is
server-side settings, not this tree). The binary is pinned by version **and checksum** —
bump `GITLEAKS_VERSION` and the checksum lookup together or the download step fails, which
is the point. Locally, the pre-commit hook runs the same config over staged content and
skips loudly when the binary is absent; CI is the backstop that does not depend on local
setup. The committed test-fixture keys are allowlisted in `.gitleaks.toml` with their
rationale beside them; inline source exceptions use `gitleaks:allow` on the same line (the
line-above form is NOT honored by gitleaks 8.30).

## The merge queue: `CI Success` must report on `merge_group` too

`ci.yml` triggers on `merge_group` so the queue can re-test each PR as the exact commit that
will land (main + every entry ahead of it + this PR) on a `gh-readonly-queue/main/pr-N-<sha>`
branch. The ruleset's one required context, `CI Success`, has to report on that commit or every
queued PR waits out the queue's timeout and is ejected — so **anything new that gates on PR
context must say what it does on a `merge_group` event too**. The payload has no
`pull_request`; what it has instead:

- **Base** — `github.event.merge_group.base_sha`, the group's parent. Behind other queued PRs
  that is *their* group commit, not main's tip, so diffing against it sees this PR alone.
  `merge-base origin/main HEAD` does not: it is main's tip, and the diff carries every PR ahead.
  `detect-scope` passes it as `--base`; `quality-gate.yml` passes it as `GITHUB_BASE_SHA`.
- **PR number / labels** — only through the branch name, `pr-<N>-`. The wire cross-check parses
  it for `GITHUB_PR_NUMBER`; `ci-success` parses it to read `codeql-override` off the PR, so a
  CodeQL stall in the queue is released the same way it is on the PR.

The `code_scanning` ruleset rule does not evaluate merge groups — it judges each PR before it
can be queued. CodeQL still runs in the queue (uniform, as above) and still fans into
`CI Success`.

The post-merge run on `main` stays. The queue makes it redundant as a merge-skew check (the
queue tested that exact commit), but it is still what refreshes the default branch's CodeQL
analyses that the `code_scanning` rule diffs every PR against. It does not run on every main
commit: a concurrency group holds one pending run, so a burst of merges skips the middle ones
(the comment on `concurrency:` in `ci.yml`). The queue verdict is what every commit carries.

### The queue's settings are load-bearing — they live in the ruleset, not this tree

Nothing in this repository can enforce these; they are set on the `no-merge-main` ruleset's
merge-queue rule, and each one is here because something in this tree depends on it.

- **Merge method: merge commit.** Not squash, not rebase. PRs land as regular merges so each
  PR's own commits stay reachable and every main commit is a `Merge pull request #N` that
  names its PR — the per-PR traceability the project relies on.
- **Maximum pull requests merged at once: 1.** Every PR lands as its own push to main. Three
  things assume one merge per push, and a multi-PR push breaks each of them:
  - `release-tag.yml` tags the commit the push lands on. A release PR batched with later
    entries would be tagged on the batch tip — the release would carry PRs its own
    "Merges since" list never showed, and the tag would no longer point at reviewed source.
  - `scripts/vercel-ignore-build.sh` falls back to `HEAD~1` on production; across a batch it
    would see the last merge only and could skip a build — and its migration apply.
  - `detect-ci-scope.sh` diffs a push to main against `HEAD~1`, the same way.

  This is NOT a queue of one. Build concurrency is separate: several entries are tested at
  once, each stacked on the ones ahead of it, and each still merges on its own. What a larger
  merge size would add is fewer CI runs per PR, and it is only safe once all three of the above
  stop assuming one merge per push.
- **Require branches to be up to date: off.** The queue replaces it; leaving it on brings the
  merge-main-back-in loop straight back.
- **Adding a PR to the queue is the merge.** The maintainer merges — merge is the review gate,
  not just a CI gate — so agent sessions do not add PRs to the queue or enable auto-merge, and
  nothing enables auto-merge for Dependabot. The supply-chain review's premise that a dependency
  bump reaches main only through a human merge depends on it.

### The release chain never runs on a queue ref

`release-tag.yml`, `release.yml` and `build-cli-binaries.yml` get no `merge_group` trigger, and
`check-release-chain-triggers.sh` (in `guard-tests`) fails if any of them gains a trigger at all.
Installed clients accept an attestation only when it was signed on `refs/heads/main`, and npm
and crates.io trusted publishing match `release-tag.yml` as the entry workflow. A queue run
signs as `gh-readonly-queue/main/...` and runs before the merge. The queue changes none of
this as long as releases keep entering through a VERSION push to main — which a queue merge
still is.

Vercel skips previews of `gh-readonly-queue/*` branches (`scripts/vercel-ignore-build.sh`):
the PR had its own preview and the commit is on main minutes later.
