# Arc 2 review: test and CI tripwire coverage — binary attestation & manifest verification

PR #573, `jct/binary-attestation-manifest-verification`. Reviewed against `code-quality.yml`'s
`guard-tests` philosophy: *a passing tripwire is worthless unless something proves it can still
FAIL.* Findings below are proven, not asserted — each gap and each "cannot fail" claim was
reproduced in a scratch copy (never the repo) and the harness/test was re-run against the broken
copy.

## Overall assessment

This is unusually well-guarded work. Of the eight invariants the task brief asked me to check, six
are already covered with genuine bite tests, including the two "known instances" called out
(`every_verdict_variant_serializes` / `unverifiable_never_renders_as_mismatch` in `manifest.rs`,
and the post-extract/post-swap gate isolation in `test-install.sh`) — I reproduced both defects in
scratch copies and confirmed the current tests catch them (see Half 2). Two real gaps remain, both
concrete and buildable.

## HALF 1 — Uncovered invariants

### GAP 1 (HIGH): the manifest wire format is proven twice, but never against each other

`manifest.rs::golden_fixture_round_trips` parses `tests/fixtures/manifest-golden.json` — a
**hand-authored** fixture (its two entries share the same `sha256`, the sha256 of an empty
string, confirming no one ever ran the real generator to produce it; `rg -rn manifest-golden`
across the repo finds no script that regenerates it). `test-emit-manifest.sh` proves the real
`emit-manifest.sh` hashes real bytes correctly, but checks its output with its own hardcoded `jq`
queries — it never touches the Rust `ReleaseManifest`/`ManifestEntry` deserializer.

I proved the gap directly: ran the real `emit-manifest.sh` against a synthetic staging dir and
diffed its output against the committed golden fixture — structurally similar, but nothing forces
them to agree.

```
$ VERSION=0.3.0 TARGET=x86_64-unknown-linux-gnu STAGING=... OUTPUT=... bash .github/scripts/release/emit-manifest.sh
{"version":"0.3.0","target":"x86_64-unknown-linux-gnu","files":[{"path":"lib/libonnxruntime.so",...}]}
$ cat crates/temper-cli/tests/fixtures/manifest-golden.json
{"version":"0.3.0","target":"x86_64-unknown-linux-gnu","files":[{"path":"temper","sha256":"e3b0c4...855","size":0}, ...]}
```

Concretely: if someone renamed a field in `emit-manifest.sh`'s `jq -n` call (e.g. `path` →
`file`), and updated `test-emit-manifest.sh`'s jq queries to match in the same commit, **nothing
would catch that the Rust `ReleaseManifest` struct now silently fails to deserialize real
producer output** — `golden_fixture_round_trips` keeps parsing the untouched, hand-written
fixture and stays green. `test-install.sh` doesn't help either: it drives `install.sh`'s own
awk-based manifest parser end-to-end, never the Rust consumer (`manifest.rs::load_from_dir` /
`verify_dir`, the code path `temper version --verify` actually runs). No test anywhere feeds real
bash-producer bytes into the real Rust deserializer.

**Proposed guard**: a new `#[test]` in `manifest.rs` (e.g.
`producer_output_deserializes_as_release_manifest`) that shells out to
`.github/scripts/release/emit-manifest.sh` against a synthetic staging dir (mirroring the pattern
`test-emit-manifest.sh` and `update.rs`'s own tests already use for shelling to `tar`), then feeds
the real stdout/file bytes through `serde_json::from_str::<ReleaseManifest>` and asserts the shape
(all four staged files present, `sha256.len() == 64`, etc.) — replacing or supplementing
`golden_fixture_round_trips`. **Bite fixture**: rename `path` → `file` in a scratch copy of
`emit-manifest.sh`'s `jq -n` call; the new test fails to find the expected key, while the old
static-fixture test stays green (proving the old test alone is insufficient). Wires into the
existing Unit test job (`test-rust.yml`) — no new CI job needed, since this is a plain crate test.

### GAP 2 (HIGH): nothing guards the CI attestation step covering both objects

`build-cli-binaries.yml`'s `Attest build provenance` step lists `subject-path` as three lines
(`.tar.gz`, `.zip`, `.manifest.json`) — this is what makes `manifest_attestation_identity`'s whole
"the manifest is attested as its own subject" claim in `version.rs` true. I grepped every
`.github/scripts/audit-*.sh` and found **zero** static checks over this workflow file or over
`subject-path`/`attest-build-provenance` at all. If someone "cleans up" that YAML and drops the
manifest line, every other check in this PR stays green — the regression only surfaces the next
time a human runs `temper version --verify --online` against a real release and gets a confusing
`Unverifiable` (fails closed, but silently defeats Task 9's whole purpose).

**Proposed guard**: a new `audit-attest-subject-path.sh` in the house static-enumerator style
(model: `audit-route-auth.sh`) that greps `build-cli-binaries.yml`'s `subject-path:` block and
asserts it still contains a `.manifest.json` line alongside an archive (`.tar.gz`/`.zip`) line.
**Bite fixture**: a copy of the workflow with the `.manifest.json` line deleted; the harness
(`test-audit-attest-subject-path.sh`) asserts the guard goes red against it. Wires in exactly like
the other four security tripwires: the audit script as a new step in `rust-quality` (needs no
cargo, but sits with its siblings for discoverability), its harness as a new step in `guard-tests`.

## HALF 2 — Guards proven to still bite (and one that never runs at all)

**Verified biting** (broke each in a scratch copy, confirmed red):
- `test-install.sh`'s post-extract/post-swap isolation: disabling *only* the post-extract call
  site (leaving post-swap intact) still passes the general "tampered archive rejected" test (the
  later gate catches it) but the new isolation assertion correctly fails with "post-extract gate
  did not fire" — proving the two gates are no longer sharing one witness.
- `manifest.rs::every_verdict_variant_serializes` / `unverifiable_never_renders_as_mismatch`: these
  exist specifically because pattern-matching alone can't see a `TaggedSerializer::bad_type`
  runtime panic; reasoning-verified against serde's documented internal-tagging behavior (not
  compiled, per the no-cargo constraint) — the fix is structurally sound (struct variants, not
  newtype).
- `test-emit-manifest.sh`'s sha256/size assertions are computed independently of the script under
  test (`sha256sum`/`shasum` on the literal input bytes) — a constant-hash regression would be
  caught.

**Found never running at all**: `crates/temper-cli/src/commands/update.rs`'s
`windows_refusal_is_actionable_and_not_the_cargo_hint` test and the `WINDOWS_REFUSAL` const it
covers are both `#[cfg(windows)]`. Every Rust CI job (`test-rust.yml`, `code-quality.yml`) runs on
`runs-on: ubuntu-latest` — there is no Windows runner anywhere in this repo's CI. This test compiles
and executes nowhere, ever — the exact "test no job runs" rot pattern `code-quality.yml`'s own
comment calls out (citing `streaming_ingest_test`). Low severity (it guards a refusal *message*,
not a security check), but cheap to fix: drop the `cfg(windows)` from the `WINDOWS_REFUSAL` const
itself (mirroring `has_windows_ort_sibling`'s existing "compiled everywhere, consulted only under
`cfg(windows)`" pattern) so the content assertion runs on the Linux CI that actually exists.

## Prioritized build list

1. **Build: Gap 1** (wire-format cross-check). Highest value — this is the one place a real
   producer/consumer drift could hide behind two independently-green suites.
2. **Build: Gap 2** (attest subject-path guard). Cheap, matches house pattern exactly, closes a
   silent-regression path in the release pipeline that nothing else in this PR touches.
3. **Build: Windows-test cfg fix.** Trivial (delete one `#[cfg(windows)]` line), and leaving it is
   exactly the anti-pattern this repo's CLAUDE.md calls out by name.

**Would NOT build:**
- A second Sigstore fixture proving `require_issuer` independently rejects a non-GitHub-issued
  bundle. The identity-URL check (`https://github.com/.../workflows/...@refs/tags/...`) already
  requires a Fulcio SAN no non-GitHub-Actions issuer could plausibly produce; a dedicated fixture
  would cost real effort to construct (a second real Sigstore-signed artifact) for marginal
  incremental coverage over what `wrong_repo_identity_is_rejected` already proves.
- A static guard pinning `RELEASE_WORKFLOW_FILE`'s literal against the actual workflow filename on
  disk. A rename would fail *loudly* the next release run (every attestation stops verifying) —
  that's an availability bug a maintainer notices immediately, not a silent security hole. Not
  worth a dedicated tripwire.
- Standing up a Windows CI runner to genuinely execute `WINDOWS_REFUSAL`'s test. Real cost for a
  message-content check; the cfg fix above gets 90% of the value for near-zero cost.
