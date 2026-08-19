# SDD ledger — plan: docs/superpowers/plans/2026-07-29-binary-attestation-and-manifest-verification.md

BASE: eac7e5e6b0bfdff5f6b5b173fc1fa4545d0c0606
Execution model: subagents WRITE ONLY (no cargo); controller runs all gates and commits.
Review cadence: consolidated at arc boundary, not per-task (user override).

Task 3: implemented (emit-manifest.sh + harness + workflow/upload wiring) — harness PASS. NOT committed; guard-tests wiring owed by controller. Open: golden-fixture shape cross-check vs Task 2.
Task 5: implemented (install.sh --archive + harness) — harness PASS; "Verifying checksum" literal survived. NOT committed; guard-tests wiring owed by controller. Open: verify embedded_installer_is_the_real_script via cargo.
Task 1 (SPIKE): COMPLETE — decision CRATE = sigstore-verify@0.11.0 (+sigstore-trust-root, sigstore-types).
  INVERTS pre-spike expectation: sigstore-verification REJECTED (private TUF-fetching trust root, no
  injection point; 905-dep tree pulling native-tls, violating the reqwest rustls pin).
  sigstore-verify: Verifier::new(&TrustedRoot) required arg; TrustedRoot::from_json is include_str!-able;
  no TUF fetch; synchronous; 442 deps, no native-tls/openssl.
  Executed: real trusted_root.jsonl + real bundle parsed, verify ran offline with caller-supplied root.
  RESIDUAL (carried to Task 9): full sig verification NOT demonstrated — gh-downloaded bundle lacks a
  Rekor inclusion proof. Task 9 must source a complete bundle; skip_tlog only with written rationale.
Gate run: cargo nextest -p temper-cli --lib => 354/354 PASS, incl.
  embedded_installer_is_the_real_script (cross-crate include_str! assertion survived Task 5's edit).
CONTROLLER NOTE: pre-commit gates rustfmt --check tree-wide, so an in-flight subagent's unformatted
  file blocks ALL commits. Subagents cannot run cargo fmt (no-cargo rule) => controller must
  `cargo fmt -p <crate>` after each Rust task lands, BEFORE attempting any commit.
Task 2: implemented. PLAN DEFECT FOUND BY IMPLEMENTER (traced serde source, not guessed):
  Verdict used newtype variants Mismatch(Vec<_>)/Unverifiable(String) under #[serde(tag=...)]
  internal tagging => compiles, then fails at RUNTIME in TaggedSerializer::bad_type. The plan's own
  tests only pattern-matched, never serialized, so the suite could not catch it.
  CONTROLLER FIX (not a silent discard — flag at consolidated review): converted to struct variants
  Mismatch{mismatches}/Unverifiable{reason}; added every_verdict_variant_serializes and
  unverifiable_never_renders_as_mismatch as the missing bite tests.
  IMPACT ON LATER TASKS: Task 7's brief uses `#[serde(flatten)] verdict: Verdict` and constructs
  Verdict::Unverifiable("...") positionally — Task 7 dispatch MUST carry the struct-variant syntax.
PROBE (bite verification of the Task 2 fix): temporarily reintroduced the newtype shape in an
  isolated `mod probe`; serde returned, verbatim:
    "cannot serialize tagged newtype variant OldVerdict::Mismatch containing a sequence"
    "cannot serialize tagged newtype variant OldVerdict::Unverifiable containing a string"
  => defect confirmed real, fix correct, new tests genuinely bite. Probe restored from file copy
  (NOT git checkout), confirmed removed.
Task 1 + Task 3: COMMITTED 30776dcf (pre-commit green).
Task 2: complete (commit 7645c785). Gate: 361/361 lib tests, clippy --all-features --all-targets -D warnings clean.
Task 6: complete — isolated witness added. Evidence: disabling ONLY post-extract gate turned the NEW
  test red while the OLD tampered test stayed green (proving the old one could not distinguish gates).
Tasks 5+6: complete (commit 7057dee3), incl. guard-tests wiring for both harnesses.
Remaining: 4 (attestation workflow), 7 (in flight), 8, 9 (gated on spike residual), 10.
MINOR (deferred, for consolidated review): install.sh's awk manifest parser is line-oriented and so
  depends on emit-manifest.sh emitting PRETTY-printed JSON. Guarded in practice (the harness feeds the
  real generator's output through the real parser) but the coupling is implicit and uncommented.
Task 4: complete (commit ef606f01). Plan corrections grounded by implementer: action is @v4 (plan said
  @v2; latest v4.1.1, verified via gh api), and subject-path's mixed .tar.gz/.zip list is safe —
  traced to actions/attest subject.ts, @actions/glob resolves each line independently and errors only
  if the COMBINED set is empty. Both permission levels widened (callee AND release.yml call site).
Task 7: implemented, gate 365/365 incl. 3 new verify tests. FINDING SENT BACK: --checksum + --verify
  silently dropped checksum, violating active goal 019fa618-2d2a "No flag silently does nothing".
  Fix in flight: compose both into VerifyReport + a test asserting both appear; plus a module-doc note
  that Windows installs carry no manifest and therefore always report unverifiable.
Task 7: complete (commit 00cfc2c4) — composition fix landed, gate 367/367.
SPIKE RESIDUAL RESOLVED (controller, same session): re-ran against a real SLSA build-provenance
  bundle (predicate https://slsa.dev/provenance/v1). tlogEntries + inclusionProof PRESENT, and
  verification succeeds END-TO-END offline against the pinned root. First fixture had merely been a
  release-predicate attestation (TSA-only, no tlogEntries) — nothing wrong with crate or root.
  => Task 9 needs NO skip_tlog; VerificationPolicy::default() is correct; transparency guarantee kept.
  => Task 9 MUST pin the public-good root specifically (root[1] fails UnknownIssuer — correct
     discrimination, and the negative control that makes the pass meaningful) and MUST request the
     SLSA predicate explicitly rather than the API default.

=== ARC 2 (planned, user-requested): adversarial security review + tripwire buildout ===
Trigger: once tasks 8/9/10 land and the branch is testably good.
Shape: MULTIPLE DISTINCT-LENS adversarial reviewers (not one generalist pass), over BOTH the Rust
  code and the CI/workflow surface. Deliverable is findings AND recommendations for new tripwires /
  test-and-CI infra, in the style of the repo's existing .github/scripts guards (each guard paired
  with a harness in the guard-tests job that proves it still bites).
Candidate lenses (distinct failure modes, not redundant passes):
  1. Threat-model: does the chain actually resist the 4 declared threats? where does residual trust
     live? what is still self-asserted?
  2. Supply-chain/CI: workflow permission scope, token blast radius, action pinning posture,
     artifact upload/download paths, what a compromised runner or action could do.
  3. Fail-open hunt: every path where verification can be SKIPPED rather than FAILED — absent
     manifest, absent attestation, network error, unmapped triple, non-release install. A fail-open
     in a verifier is worth more to an attacker than a missing check.
  4. Tripwire coverage: which asserted invariants have NO guard, and which existing guards could no
     longer fail (the "green tick that means nothing" problem code-quality.yml already names).
Do NOT pre-judge findings for these reviewers or hand them my own candidate list — let them find it,
then adjudicate. (SDD: never instruct a reviewer not to flag something.)
Task 8: complete (commit 62b983aa). NOTE: rustdoc gate caught a private-intra-doc-link that clippy
  did NOT — reconfirms cargo check != clippy != rustdoc; always run the docs gate.
Task 9a: implemented — attest.rs with pinned public-good root (crates/temper-cli/trust/), real
  cli/cli build-provenance fixture, and the load-bearing NEGATIVE test wrong_repo_identity_is_rejected.
  Implementer confirmed via `openssl x509 -text` that the fixture's SAN is cli/cli's deployment.yml
  with the SAME issuer, so the negative test isolates IDENTITY, not issuer.
  Known limitation (implementer-declared, not hidden): the two error classes are separated by text
  matching, because the crate exposes no structured chain-vs-policy error variants.
OPEN TRADEOFF for Arc 2 / user decision: fundamentals.md describes temper-cli as a "lightweight CLI
  binary — no heavy deps", and this adds sigstore-verify/-types/-trust-root pulling aws-lc-rs (C
  crypto build). Measure binary-size and cold-build delta and report; it may be worth feature-gating.
Task 9a: complete (commit 52629128). Gate 383/383 incl. wrong_repo_identity_is_rejected.
  Dep check in REAL workspace after feature unification: native-tls 0, openssl 0 (constraint holds).
  Two reqwest majors coexist (0.12 + 0.13), both rustls — build weight, not a second TLS stack.
  2nd rustdoc private-intra-doc-link failure (clippy green, docs red). Pattern: subagents cannot run
  the docs gate, so this class recurs; controller must always run it.
CONTROLLER GROUNDING for Task 9b — GitHub attestations API (probed live, not assumed):
  GET /repos/{owner}/{repo}/attestations/sha256:{digest} -> {attestations: [...]}.
  * Returned TWO attestations for one digest => selecting by predicate type is MANDATORY.
  * Entry keys: bundle, bundle_url, initiator, repository_id. `bundle` was NULL and `bundle_url` was
    a short-lived PRESIGNED Azure blob URL — the bundle is NOT reliably inline. Code must handle
    both: use inline bundle when present, else follow bundle_url.
  * gh's ?predicate_type= query form failed through `gh api` URL parsing; filter client-side instead.
  * Arc 2 note: bundle_url points at a third-party blob host. Signature makes tampering detectable,
    which is the point — but worth a reviewer's eye.
Task 9b: implemented + TWO controller findings fixed.
  F1 (trivial): test's expected sha256 literal was 63 chars (truncated). Code was correct. Fixed and
    a len()==64 assertion added so truncation can never again present as a hash mismatch.
  F2 (SECURITY — most significant finding of the session): --online compared installed files against
    the MANIFEST but verified the attestation over the ARCHIVE's digest. Two different objects.
    Attack vs threat model #2 (compromised release artifact): serve a tampered manifest matching a
    tampered install, leave the genuine archive .sha256 untouched => manifest check passes,
    attestation check passes on an artifact never used => verdict "Verified" on a tampered install.
    Implementer independently verified the manifest IS its own attest subject in
    build-cli-binaries.yml before agreeing (asked to push back rather than comply).
    FIX: hash the EXACT manifest buffer used for the comparison (no re-fetch => no second TOCTOU),
    verify attestation over that digest; archive-sidecar path deleted, not left dead.
    Composition property added: attestation can only DOWNGRADE a Verified, never override a Mismatch.
    temper update's path confirmed already correct — there the archive IS the object relied on.
  LESSON FOR ARC 2: a chain of individually-valid checks need not compose into the intended
    guarantee. A checklist review would have ticked "attestation verified: yes". This is the
    strongest argument for the adversarial framing.
Task 9b: complete (commit 93c16fd1) after both findings fixed. Gate 406/406.
Task 10: complete (commit 8729b52f). Windows deferral now stated in WINDOWS_REFUSAL itself
  (verified by inspection: contains install.ps1, omits "cargo install", says hash-verified only) —
  the cfg(windows) test cannot run on darwin, so inspection is the evidence, not the test.
ARC 1 COMPLETE. Full `cargo make check` clean (353s): fmt --all, clippy --workspace --all-targets
  --all-features -D warnings, cargo doc --workspace, cargo machete, openapi chain, bun typecheck,
  biome. Both bash harnesses green (5 assertions), sh -n clean, guard-tests wiring verified present.
NOT YET DONE: branch is unpushed, no PR. Per standing preference, push/PR requires the user's ok.
ARC 2 (adversarial security review + tripwires) is the next move, triggered by user.

=== ARC 2 FINDINGS (3 of 4 lenses in) ===
CONFIRMED BY CONTROLLER REPRODUCTION (not taken on report):
 A1 [HIGH] Vacuous manifest = green install. `files: []` -> awk loop runs zero times -> CHECK_FAILED
   stays 0 -> install proceeds. Reproduced: exit 0, "# EVIL PAYLOAD" binary installed, "✓ Installed"
   printed. Needs NO attacker — a STAGING drift ships a vacuous manifest that CI then genuinely
   SIGNS, so --verify --online returns signature-backed `verified` over zero files.
 A2 [HIGH] Same root cause, different trigger: awk parser yields zero pairs on COMPACT json (the
   /"path":/ rule ends in `next`, so "sha256": never fires when both are on one line). A single
   `jq -c` in emit-manifest.sh would silently disable the gate for every user.
   => A1 and A2 share ONE root cause: ZERO PARSED ENTRIES IS TREATED AS SUCCESS. One fix closes both:
      fail hard when the parsed pair count is 0, and cross-check parsed count vs manifest entry count.
   CONTROLLER CORRECTION to the fail-open reviewer: it claimed test-install.sh cannot see A2. Not
   quite — if emit-manifest.sh went compact, the tampered-archive assertion would start passing an
   install it must reject, so the harness WOULD go red. It is a backstop for that route only; a
   compact manifest from any other source is uncovered. Severity stands, framing corrected.
 A3 [HIGH/MED] update.rs attests the ARCHIVE but the manifest install.sh writes to disk as the
   permanent offline baseline is never attested. version.rs deliberately attests the manifest digest;
   update.rs does not. Fix is one call — the workflow already attests that subject.
 A4 [MED] No path containment: a manifest entry `../outside/decoy` installs clean and `temper` is
   never hashed. Path::join with an absolute path discards the base.
 A5 [MED] Threat 4 composes for the model, NOT the ORT lib: model has compiled-in pin + manifest;
   libonnxruntime is dlopen'd with no load-time hash and an unverified ORT_DYLIB_PATH.
 A6 [DEAD TEST] windows_refusal_... is cfg(windows) and NO workflow uses a windows runner (verified:
   zero windows runs-on in .github/workflows). It executes nowhere. I relied on inspection for it
   earlier and assumed CI covered it — it does not.
 A7 [GAP] Manifest wire format proven twice but never against itself: Rust golden fixture is
   hand-authored, never regenerated from emit-manifest.sh; bash harness checks producer against its
   own jq queries. No test feeds real producer bytes to the real Rust deserializer.
 A8 [GAP] subject-path (archive + manifest) has no static guard; deleting the manifest line silently
   defeats online verification until a real release breaks.
RESIDUAL TRUST (threat-model lens): the attestation binds BUILDER and TAG, never SOURCE. Repo write
  => a tag whose workflow builds a backdoor verifies perfectly on every path. Beneath that: the
  unsigned `curl | main/install.sh` bootstrap named as the recovery in every trust-root error, and
  the hand-committed trust-root blob with no freshness check.
