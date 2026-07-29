# Arc 2 — adversarial security review: consolidated adjudication

**Date:** 2026-07-29
**Subject:** PR #573, branch `jct/binary-attestation-manifest-verification`
**Method:** four independent adversarial reviewers, distinct lenses (threat model · supply chain/CI ·
fail-open hunt · tripwire coverage). No reviewer was given the others' findings, or mine.

**Individual reviews (committed, read these for the evidence behind each finding):**

- [`docs/superpowers/reviews/2026-07-29-arc2-threat-model.md`](../reviews/2026-07-29-arc2-threat-model.md)
- [`docs/superpowers/reviews/2026-07-29-arc2-supply-chain.md`](../reviews/2026-07-29-arc2-supply-chain.md)
- [`docs/superpowers/reviews/2026-07-29-arc2-fail-open.md`](../reviews/2026-07-29-arc2-fail-open.md)
- [`docs/superpowers/reviews/2026-07-29-arc2-tripwires.md`](../reviews/2026-07-29-arc2-tripwires.md)
- [`docs/superpowers/reviews/2026-07-29-arc1-arc2-ledger.md`](../reviews/2026-07-29-arc1-arc2-ledger.md) —
  the implementation ledger: every task, every gate result, every controller finding and correction,
  in order. This is the recovery map for a fresh session.

> **STATUS FOR WHOEVER PICKS THIS UP:** PR #573 has **confirmed HIGH-severity defects** (Tier 1
> below) and is marked draft for that reason. Do not merge it until at least T1.1 and T1.2 are fixed.
> The vacuous-manifest fail-open has been reproduced: an archive containing `# EVIL PAYLOAD`
> installed with `exit 0` and printed `✓ Installed`.

## What held up

Worth stating first, because it scopes everything below. **The cryptographic core is correct.** Both
the threat-model and supply-chain lenses independently concluded the same thing: digest binding, the
pinned root, issuer + exact tag-scoped SAN, Rekor inclusion kept, refusal to blindly take
`bundles[0]`, a real negative test, and `install.sh` embedded by `include_str!` so it cannot fork.
The fail-open lens found **no** error→pass arm anywhere in `attest.rs` or `attestation_fetch.rs`, and
confirmed `finish_online_verdict` is downgrade-only in both directions.

**The weakness is not the signature. It is everything around the signature** — what gets signed, who
can cause a signing, and what "verified" is allowed to mean when there is nothing to verify.

## Tier 1 — defects in this PR. Fix before merge.

### T1.1 Zero parsed entries is treated as success (HIGH, reproduced)

Two triggers, one root cause.

- **`files: []`** — `manifest.rs:105` returns `Verified` on an empty list; `install.sh`'s awk loop
  never executes; `CHECK_FAILED` stays `0`.
- **Compact JSON** — the awk `/"path":/` rule ends in `next`, so `"sha256":` never fires when both
  keys share a line. A single `jq -c` in `emit-manifest.sh` disables the gate for every user, with
  identical-looking output.

Reproduced by the controller: an archive whose binary contained `# EVIL PAYLOAD` installed with
`exit 0` and printed `✓ Installed`.

**Why this is worse than an ordinary bug: no attacker is required, and CI would sign it.** A build-side
`STAGING` drift emits a vacuous manifest, `attest-build-provenance` faithfully attests it, and
`--verify --online` then returns a genuine, *signature-backed* `verified` over zero files. Every
layer reports green while nothing is checked.

**Fix (one change, both triggers):** treat zero parsed entries as a hard failure, and cross-check the
parsed pair count against the manifest's declared entry count so a parse that silently drops entries
cannot pass either. Apply in **both** consumers — `install.sh` and `manifest.rs`.

> **Controller correction to the fail-open reviewer.** It reported that `test-install.sh` cannot see
> the compact-JSON variant. Not quite: if `emit-manifest.sh` went compact, the tampered-archive
> assertion would begin passing an install it must reject, so the harness *would* go red. It is a
> genuine backstop for that route. What is uncovered is a compact manifest arriving from any other
> source. Severity stands; the framing was too strong.

### T1.2 `update.rs` never attests the manifest it persists (HIGH/MED)

Found independently by two lenses. `version.rs` deliberately attests the **manifest's** digest —
there is a comment explaining exactly why — while `update.rs:518` attests only the **archive's**, and
then `install.sh` copies that unattested manifest into the install dir as `.temper-manifest.json`,
the permanent baseline for every future offline verdict.

Consequence: one hour of release-asset write, combined with T1.1, installs genuine bytes and
permanently blinds tamper detection.

**This is the same defect class already caught and fixed once in `version.rs` during implementation.
It survived in the sibling path.** Fixing one instance of a class and not sweeping for others is the
lesson worth keeping.

**Fix:** one additional call. The workflow already attests that subject.

### T1.3 No path containment in the manifest (MED)

A manifest entry of `../outside/decoy` installs cleanly and `temper` is never hashed —
`Path::join` with an absolute path discards the base. Lets an attacker forge the offline verdict by
editing only `path` strings.

**Fix:** reject any entry whose path is absolute, contains `..`, or escapes the install root, in both
consumers.

## Tier 2 — repo-wide, pre-existing, and more dangerous now. Separate PR, do it promptly.

These are **not** introduced by this PR. They matter more *because* of it: this PR adds the ability to
produce signed artifacts, so a path that reaches the release workflow now reaches a signing oracle.

### T2.1 `default_workflow_permissions: write` with no top-level `permissions:` (HIGHEST)

Verified by the controller: `gh api …/actions/permissions/workflow` returns
`{"default_workflow_permissions":"write"}`, and **no workflow in the repo declares a top-level
`permissions:` block**. Every job therefore runs with `contents: write` by default.

Chain: a compromised action or `build.rs` in *any* un-scoped job → `contents: write` →
`git push origin v9.9.9` → release fires → the backdoor is **signed** → `temper update` reports it
verified.

The reviewer grepped and found no un-scoped job that needs write, so **the fix is zero-breakage**: add
`permissions: contents: read` at the top of each workflow and grant write only where required.

### T2.2 Tag name interpolated into bash (MED)

`release.yml` interpolates `${{ github.ref_name }}` directly into a shell step. Verified locally:
`git check-ref-format` accepts `v1.0.0";id;#`, `` v1.0.0`id` ``, and `v1.0.0$(id)`. Pre-existing, but
the injected code now lands in the job holding `id-token: write` and `attestations: write`.

**Fix:** pass through `env:` and quote, rather than interpolating into the script body.

## Tier 3 — harden the signing job

### T3.1 The ONNX Runtime is downloaded with no checksum, then signed (MED — the sharpest irony here)

`build-cli-binaries.yml` `curl`s the ORT `.so`/`.dll` with **no integrity check**, copies it into
staging, hashes it into the manifest, and signs it. We then ship a binary that `dlopen`s it.

This repo already has the doctrine for exactly this, and `attest.rs` cites it: `EXPECTED_MODEL_SHA256`
is derived at build time from the real artifact and refuses to load on mismatch. The model is pinned;
the native library it loads beside it is not. **The threat-model lens reached the same conclusion from
the other direction:** threat 4 composes for the model (two guards) and not for the ORT lib (one
install-time check, plus an unverified `ORT_DYLIB_PATH` override).

**Fix:** pin the ORT archive's sha256 in the workflow and verify after download. Cheap, and it closes
both halves.

### T3.2 Cache and toolchain inside the trust boundary (MED/LOW)

`Swatinem/rust-cache@v2` restores a cache writable from `main` scope into the signing job — a poisoned
`target/` gets truthfully attested. `dtolnay/rust-toolchain@stable` resolves a **mutable branch** ref
inside that same job.

### T3.3 Action pinning — the open question, answered

The reviewer's recommendation is well-reasoned and I endorse it over blanket pinning:

- **SHA-pin only the actions inside the signing job**, not repo-wide. Repo-wide pinning generates ~20
  bump PRs a year that get rubber-stamped, which is net-negative for security.
- **Replace `dtolnay/rust-toolchain@stable` with a few lines of `rustup`** rather than pinning it: a
  branch ref has no release cadence, so pinning it means bumping blind.
- Cost: one setup PR, ~5 Dependabot PRs/yr, +15-30s per matrix leg.

## Tier 4 — tripwires to build

In the house style: each guard paired with a harness in `guard-tests` that feeds it a deliberately
broken fixture and proves it goes red.

1. **Cross-language manifest round-trip.** Today the wire format is proven twice and never against
   itself: the Rust golden fixture is hand-authored (never regenerated from `emit-manifest.sh`), and
   the bash harness checks the producer against its own `jq` queries. Nothing feeds **real producer
   bytes into the real Rust deserializer**. A coordinated rename on both bash sides ships while Rust
   silently stops parsing.
2. **`audit-attest-subject-path.sh`** — a static guard that the attestation step covers *both* the
   archive and the manifest. Deleting the manifest line currently defeats online verification
   silently until a real release breaks.
3. **Vacuity floor guard** — assert that an empty or zero-entry manifest fails, in both consumers.
   This is T1.1's regression test and should not be optional.

### The dead test

`windows_refusal_is_actionable_and_not_the_cargo_hint` is `#[cfg(windows)]`, and **no workflow uses a
Windows runner** (controller-verified: zero `runs-on: windows*` across `.github/workflows`). It
compiles and executes nowhere — the exact "a test no job runs is a test that runs nowhere" rot this
repo's own CI comment names.

Noted honestly: during implementation I verified that constant's properties **by inspection**, because
the test could not run on darwin, and assumed CI covered it. It does not. My inspection was the only
check that ever happened.

**Fix:** drop `cfg(windows)` from the constant so its test executes everywhere. Do **not** stand up a
Windows runner for one string assertion.

## Tier 5 — the honest statement, for the docs

**The attestation binds the builder and the tag. It never binds the source.**

Anyone with repo write can push a tag whose workflow builds a backdoor, and it will verify perfectly
on every path we built — correct signature, correct identity, correct inclusion proof. This is
inherent to build provenance, not a defect in this implementation, but it is the true answer to *"what
does this prove?"* and the docs currently imply more than it.

Beneath it sit two smaller residual-trust items worth naming in the same place: the **unsigned
`curl | main/install.sh` bootstrap** — which every trust-root error message names as the recovery
path — and the **hand-committed trust-root blob**, which has no freshness or provenance check of its
own.

## Not findings

Recorded so they are not re-raised:

- **Fresh installs perform no attestation check.** Declared and reasoned in the design doc; a
  `curl | sh` installer cannot hard-depend on `gh`/`cosign`.
- **`bundle_url` points at a third-party blob host.** The contents are cryptographically bounded by
  the signature check, so this is safe.

## Recommended order

1. **T1.1** — the vacuity fix. Highest value per line of change in the entire review.
2. **T1.2** — attest the manifest in `update.rs`. One call.
3. **T2.1** — workflow permissions. Highest absolute severity, zero-breakage, separate PR.
4. **T1.3**, **T3.1** — path containment; pin the ORT download.
5. **Tier 4** tripwires, including the vacuity regression guard.
6. **T2.2**, **T3.2/T3.3**, **Tier 5** docs.
