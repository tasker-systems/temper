# Arc 2 Security Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close every Tier 1–5 finding from the Arc 2 adversarial security review on branch `jct/binary-attestation-manifest-verification`, so PR #573 ships with no known holes.

**Architecture:** Three defect classes, fixed at their source rather than at one consumer. (1) *Vacuity* — "nothing to check" is currently "everything checks out"; fixed at the producer and at both consumers independently, because each is separately reachable. (2) *Unattested persistence* — `update.rs` persists a manifest it never attested; fixed additively, keeping the existing archive attestation. (3) *Signing-oracle reach* — CI jobs hold more token scope than they need; fixed by scoping the seven unscoped workflows.

**Tech Stack:** Rust (`temper-cli`), POSIX sh (`install.sh`), bash (`.github/scripts/`), GitHub Actions YAML.

**Source of truth:** [`docs/superpowers/specs/2026-07-29-arc2-security-review-adjudication.md`](../specs/2026-07-29-arc2-security-review-adjudication.md) and the five reviews in [`docs/superpowers/reviews/`](../reviews/). Read the adjudication before starting — **but see "Adjudication corrections" below; three of its claims are wrong on disk and following them verbatim would cause a regression.**

## Execution model — who runs what

**Implementer subagents do NOT run `cargo`. At all.** No `cargo check`, `nextest`, `clippy`, `fmt`,
`cargo make`, or `sqlx prepare`. A cold build in this workspace takes 4–12 minutes against a 120s
Bash tool timeout; a subagent that backgrounds it can never observe completion (only the main loop
is re-invoked on the notification), so it parks forever. This has burned 100k+ tokens across
stop/resume cycles more than once.

| Actor | Runs |
|---|---|
| **Implementer subagent** | Writes code and tests. Runs *non-cargo* verification only: `bash`/`sh` harnesses, `sh -n`, `shellcheck`, `python3 -c yaml.safe_load`, `rg`, `git diff`. Reports what it wrote. |
| **Controller (main loop)** | Every `cargo` invocation — `nextest`, `check`, `clippy`, **`fmt`** — and **every `git commit`**. |

Two corollaries the controller must hold:

- **A subagent reporting DONE has compiled nothing.** Never treat "it compiles" or "tests pass" from
  a subagent as evidence for Rust work. Run the gate.
- **`cargo fmt` is a distinct gate.** `cargo make check` runs `cargo fmt --check` and fails with exit
  105 on unformatted code. Hand-written plan code rarely matches rustfmt's reflow of multi-line call
  sites and assertions, so run `cargo fmt -p temper-cli` before each Rust commit, not once at the end.

The per-task "Run:" and "Commit" steps below are written as the *work to be done*, not as an
instruction to the subagent. The controller executes them.

## Global Constraints

- **`install.sh` must not gain a `jq` dependency.** It is a `curl | sh` installer whose only tools are `curl`, `tar`, and `shasum`/`sha256sum` (`install.sh:215-217`). Every bash-side fix uses `awk`/`grep`/`case` only.
- **`install.sh` is POSIX sh, not bash.** No `[[ ]]`, no arrays, no `local`.
- **The verdict trichotomy is load-bearing: `verified` / `mismatch` / `unverifiable`.** "We cannot tell" is never rendered as "it is wrong", and never as "it is right" (`manifest.rs:54-55`). New failure modes pick the honest variant, never a convenient one.
- **`install.sh` is embedded into the binary via `include_str!`** so it cannot fork (`update.rs`). Changes to it are compiled in; a Rust test may assert on its text.
- **Every new bash guard gets a harness in the `guard-tests` job** (`code-quality.yml:143`) that feeds it a deliberately broken fixture and proves it goes red. "Adding a guard means adding its harness HERE, in the same PR" — `code-quality.yml:142`.
- **All Rust work: `cargo make check` must stay clean**, and use `#[expect(lint, reason = "...")]` over `#[allow]`.
- Commit per task. Do not push or open/convert the PR — the user does that.

## Adjudication corrections — read before Task 4 and Task 6

The adjudication is the spec, but three of its claims do not survive contact with disk. These were verified in the planning session:

1. **T1.2 is additive, NOT a swap.** The adjudication reads as "`version.rs` attests the manifest, `update.rs` attests the archive, the sibling was missed." But `CLAUDE.md` documents the archive-digest choice on the `update` path as **correct** ("the archive's digest for `update`, since the archive is what gets installed"), and `update.rs:516-521` implements exactly that. The defect is that `update.rs` *additionally* persists the downloaded manifest as the permanent offline baseline without ever attesting it. **Attest both. Do not remove or replace the archive attestation** — doing so regresses the install path.

2. **T2.1's blast radius is narrower than stated, and the chain runs through a different scope.** The adjudication says "no workflow in the repo declares a top-level `permissions:` block ⇒ every job runs with `contents: write`." The first half is true; the second is false. Verified per-workflow at every indentation level:
   - **Already correctly scoped:** `build-cli-binaries.yml:25-28` (`contents: read` + `id-token: write` + `attestations: write`), `release.yml:47-50` and `:61-62`, `release-tag.yml:19-20` and `:60-61`.
   - **Unscoped (the actual seven):** `ci.yml`, `code-quality.yml`, `coverage.yml`, `test-agents-ts.yml`, `test-ruby.yml`, `test-rust.yml`, `test-typescript.yml`.

   The signing job itself is **not** unscoped. The real chain is: a compromised action or `build.rs` in one of the seven → `default_workflow_permissions: write` grants **all** scopes including `actions: write` and `contents: write` → create a tag at an attacker commit → **`workflow_dispatch` `release.yml`** with that tag → signed backdoor. Note the dispatch step is required: a tag pushed with `GITHUB_TOKEN` does **not** trigger `release.yml`, because GitHub suppresses workflow runs from `GITHUB_TOKEN`-authored events. Scoping to `contents: read` removes `actions: write` too, since declaring any `permissions:` block zeroes every unlisted scope.

3. **Vacuity has three sites, not two.** The adjudication says "fix in both consumers." The **producer** is where vacuity is born: `emit-manifest.sh:35-49` — an empty `find` leaves `ENTRIES=""`, and `printf '' | jq -s '{... files: .}'` emits `files: []` and exits 0. Fix the producer too (Task 1); it is the cheapest place to stop a vacuous manifest from ever being signed.

## File Structure

| File | Responsibility | Tasks |
|---|---|---|
| `.github/scripts/release/emit-manifest.sh` | Producer: refuse to emit a vacuous manifest | 1 |
| `.github/scripts/test-emit-manifest.sh` | Producer harness (exists; extend) | 1 |
| `crates/temper-cli/src/manifest.rs` | Rust consumer: vacuity floor + path containment | 2, 5 |
| `scripts/install/install.sh` | sh consumer: vacuity floor, parse-drop cross-check, path containment | 3, 5 |
| `scripts/install/test-install.sh` | Installer harness (exists; extend) | 3, 5 |
| `crates/temper-cli/src/commands/update.rs` | Attest the manifest it persists; drop `cfg(windows)` | 4, 12 |
| `.github/workflows/*.yml` | Token scoping, tag interpolation, ORT pin, action pinning | 6, 7, 8, 9 |
| `.github/scripts/audit-attest-subject-path.sh` (new) | Static guard: attestation covers archive **and** manifest | 11 |
| `.github/scripts/test-audit-attest-subject-path.sh` (new) | Its harness | 11 |
| `.github/scripts/test-manifest-roundtrip.sh` (new) | Cross-language round-trip: real producer bytes → real Rust deserializer | 10 |
| `docs/guides/releasing.md`, `docs/guides/*` | Tier 5 honest statement | 13 |

---

## Task 1: Producer refuses to emit a vacuous manifest

**Files:**
- Modify: `.github/scripts/release/emit-manifest.sh:35-52`
- Test: `.github/scripts/test-emit-manifest.sh`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: the guarantee that a published manifest always has `files | length >= 1`. Tasks 2 and 3 do **not** rely on this — they defend independently, because a manifest can reach a consumer from a source other than this producer.

**Grounding (CONFORM):** `emit-manifest.sh:37-44` builds `ENTRIES` in a `while read` loop over `find "$STAGING" -type f -print0`. If that loop body never runs, `ENTRIES` stays `""` and line 46's `printf '%s' "" | jq -s '{version:…, target:…, files: .}'` emits `{"files": []}` with exit 0.

- [ ] **Step 1: Write the failing harness assertion**

Append to `.github/scripts/test-emit-manifest.sh`. That file already defines `EMIT` (`:7`), `TMP` (`:8`), and `fail()` (`:11`) — use them; do not introduce new ones.

```bash
# 6. A staging dir with no files must not produce a manifest at all. A vacuous
#    manifest is worse than no manifest: CI would faithfully SIGN it, and
#    `--verify --online` would then return a signature-backed `verified` over
#    zero files. Every layer green, nothing checked.
mkdir -p "$TMP/empty-staging"
EMPTY_OUT="$TMP/empty.json"
if VERSION=0.0.0 TARGET=x86_64-unknown-linux-gnu STAGING="$TMP/empty-staging" OUTPUT="$EMPTY_OUT" \
     bash "$EMIT" >/dev/null 2>&1; then
  fail "emit-manifest.sh exited 0 on an empty staging dir"
fi
[ ! -s "$EMPTY_OUT" ] || fail "emit-manifest.sh wrote a manifest for an empty staging dir"

echo "PASS: an empty staging dir is refused rather than emitting a vacuous manifest"
```

Note the harness runs `set -euo pipefail` (`:4`), so the `if !` form is required around the intentionally-failing invocation — a bare call would abort the harness.

- [ ] **Step 2: Run the harness to verify the new assertion fails**

```bash
bash .github/scripts/test-emit-manifest.sh
```

Expected: FAIL on "emit-manifest.sh exited 0 on an empty staging dir".

- [ ] **Step 3: Add the producer floor**

In `.github/scripts/release/emit-manifest.sh`, insert after the `while` loop closes (after line 44) and before the `printf … | jq -s` on line 46:

```bash
# Refuse to emit a manifest that asserts nothing. An empty STAGING (a path
# typo, a rename upstream, a build that produced no artifacts) would otherwise
# emit `files: []` with exit 0 — and the release workflow would sign it,
# handing every consumer a signature-backed manifest covering zero files.
# No attacker is required for this; it is one drift away at any time.
if [ -z "$ENTRIES" ]; then
    echo "error: no files found under STAGING ($STAGING) — refusing to emit a vacuous manifest" >&2
    exit 1
fi
```

Then add a post-write self-check after line 49's redirect, before the `echo`/`cat`:

```bash
# Cross-check what we wrote against what we found: a jq change that silently
# dropped entries must not pass as a smaller-but-valid manifest.
FOUND_COUNT=$(find "$STAGING" -type f | wc -l | tr -d ' ')
WROTE_COUNT=$(jq '.files | length' "$OUTPUT")
if [ "$FOUND_COUNT" != "$WROTE_COUNT" ]; then
    echo "error: manifest lists $WROTE_COUNT files but $FOUND_COUNT were found under $STAGING" >&2
    exit 1
fi
```

`jq` is available here — this runs on a GitHub runner, unlike `install.sh`.

- [ ] **Step 4: Run the harness to verify it passes**

```bash
bash .github/scripts/test-emit-manifest.sh
```

Expected: all assertions PASS, including the new one.

- [ ] **Step 5: Verify the script is still syntactically valid**

```bash
sh -n .github/scripts/release/emit-manifest.sh && bash -n .github/scripts/release/emit-manifest.sh
```

Expected: no output, exit 0.

- [ ] **Step 6: Commit**

```bash
git add .github/scripts/release/emit-manifest.sh .github/scripts/test-emit-manifest.sh
git commit -m "fix(release): refuse to emit a vacuous manifest (T1.1, producer)"
```

---

## Task 2: Rust consumer — vacuity floor in `verify_dir`

**Files:**
- Modify: `crates/temper-cli/src/manifest.rs:84-110`
- Test: `crates/temper-cli/src/manifest.rs` (the `mod tests` block at `:112`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `manifest::verify_dir` returns `Verdict::Unverifiable { reason }` when `manifest.files` is empty. **No call-site changes are needed** — verified in planning: `update.rs:468-470` already maps `Unverifiable` to a hard `CliError::Install`, and `version.rs:497` already gates on `matches!(verdict, Verdict::Verified)`. Task 5 modifies this same function; expect to rebase on it.

**Grounding (CONFORM):** `manifest.rs:105-109` reads

```rust
    if mismatches.is_empty() {
        Verdict::Verified
    } else {
        Verdict::Mismatch { mismatches }
    }
```

With `manifest.files == []` the `for` loop at `:86` never executes, `mismatches` is empty, and the function returns `Verified`.

**Why `Unverifiable` and not `Mismatch`:** nothing mismatched — the manifest asserts nothing. `Mismatch` would be a false accusation, and `manifest.rs:54-55` states the trichotomy is deliberate. `Unverifiable` is both honest for offline `temper version --verify` and fatal on the install path, which is exactly the split we want.

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block in `crates/temper-cli/src/manifest.rs`:

```rust
    /// A manifest listing zero files asserts nothing, so it cannot verify
    /// anything. Returning `Verified` here was a reproduced fail-open: an
    /// archive containing `# EVIL PAYLOAD` installed with exit 0. It is not a
    /// `Mismatch` either — nothing disagreed; there was simply nothing to check.
    #[test]
    fn empty_manifest_is_unverifiable_not_verified() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = ReleaseManifest {
            version: "0.0.0".to_string(),
            target: "x86_64-unknown-linux-gnu".to_string(),
            files: vec![],
        };
        match verify_dir(&manifest, tmp.path()) {
            Verdict::Unverifiable { reason } => {
                assert!(
                    reason.contains("no files"),
                    "reason should name the vacuity, got: {reason}"
                );
            }
            other => panic!("empty manifest must be unverifiable, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest run -p temper-cli empty_manifest_is_unverifiable_not_verified
```

Expected: FAIL — panics with `empty manifest must be unverifiable, got Verified`.

- [ ] **Step 3: Add the floor**

In `crates/temper-cli/src/manifest.rs`, insert at the very top of `verify_dir`'s body, before `let mut mismatches = Vec::new();`:

```rust
    // A manifest with no entries asserts nothing about this install, so the
    // loop below cannot fail and the function would return `Verified` over
    // zero checked files. That is a fail-open, and no attacker is needed to
    // reach it: a build-side staging drift emits `files: []`, CI faithfully
    // attests it, and `--verify --online` then returns a genuine,
    // signature-backed `verified` covering nothing. Not a `Mismatch` — nothing
    // disagreed; we simply cannot tell.
    if manifest.files.is_empty() {
        return Verdict::Unverifiable {
            reason: "manifest lists no files — it asserts nothing about this install".to_string(),
        };
    }
```

Also extend the module doc at `manifest.rs:11-17` ("What a verdict does and does not mean") with one sentence naming the vacuity floor, so the invariant is documented where the trichotomy is.

- [ ] **Step 4: Run the test to verify it passes**

```bash
cargo nextest run -p temper-cli empty_manifest_is_unverifiable_not_verified
```

Expected: PASS.

- [ ] **Step 5: Run the full CLI test suite for regressions**

```bash
cargo nextest run -p temper-cli --lib
```

Expected: all pass (410/410 at Arc 1 close, plus the new one). If any existing test constructed an empty manifest and expected `Verified`, that test encoded the bug — fix the test and say so in the commit body.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/manifest.rs
git commit -m "fix(cli): an empty manifest is unverifiable, never verified (T1.1, Rust consumer)"
```

---

## Task 3: sh consumer — vacuity floor and parse-drop cross-check in `install.sh`

**Files:**
- Modify: `scripts/install/install.sh:232-272`
- Test: `scripts/install/test-install.sh`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `verify_manifest_against_dir` fails when the manifest yields zero parsed pairs, or when the parsed pair count disagrees with the count of `"path":` occurrences in the manifest file. Task 5 modifies this same function.

**Grounding (CONFORM):** `install.sh:232-248` defines `manifest_pairs`, whose awk `/"path":/` rule ends in `next` (`:239`) — so on compact JSON, where `"path"` and `"sha256"` share a line, the `/"sha256":/` rule at `:241` never fires and **zero pairs** are emitted. `verify_manifest_against_dir` at `:253-272` then loops over nothing, `CHECK_FAILED` stays `0` (`:255`), and `:271` returns success. The `[ -n "$REL" ] || continue` at `:257` means even a blank read is skipped silently.

**Two triggers, one fix.** The zero-pair floor catches `files: []`. The count cross-check catches compact JSON: a compact manifest with 3 entries yields 3 `"path":` occurrences but 0 parsed pairs, so the counts disagree even before the floor is consulted.

- [ ] **Step 1: Write the failing harness assertions**

Append to `scripts/install/test-install.sh`, after the post-extract-gate block that ends at `:121`. That file already defines `INSTALL` (`:7`), `TMP` (`:8`), `fail()` (`:11`), `build_archive()` (`:15`), and `build_manifest STAGE OUT` (`:29`), and drives the installer as
`TEMPER_INSTALL_DIR=… XDG_BIN_HOME=… sh "$INSTALL" --archive … --manifest … --version v0.3.0` (`:42-44`). Use those; do not invent new helpers.

```bash
# --- A manifest that verifies nothing must not verify everything -------------
# Reproduced during Arc 2: an archive whose binary contained `# EVIL PAYLOAD`
# installed with exit 0 and printed "✓ Installed", because zero parsed entries
# left CHECK_FAILED at 0. Both variants below reach that same fail-open.

# Rebuild a known-good archive+manifest pair to test against.
build_archive
build_manifest "$TMP/stage" "$TMP/vacuity.manifest.json"

# 1. An empty file list: the manifest genuinely asserts nothing.
printf '{"version":"0.3.0","target":"x86_64-unknown-linux-gnu","files":[]}\n' \
  > "$TMP/empty.manifest.json"
if TEMPER_INSTALL_DIR="$TMP/install-empty" XDG_BIN_HOME="$TMP/bin-empty" \
     sh "$INSTALL" --archive "$TMP/archive.tar.gz" --manifest "$TMP/empty.manifest.json" \
        --version v0.3.0 >/dev/null 2>&1; then
  fail "installer accepted a manifest listing zero files"
fi
[ ! -e "$TMP/install-empty" ] || fail "a rejected empty-manifest install left files behind"

echo "PASS: a manifest listing zero files is refused"

# 2. Compact JSON: the manifest declares real entries, but the awk pair parser
#    emits none (the /"path":/ rule ends in `next`, so /"sha256":/ never fires
#    when both keys share a line). The bytes are otherwise IDENTICAL to the
#    good manifest — same files, same hashes — so this is not "compact JSON is
#    invalid"; it is "a partial parse must refuse, not silently verify nothing."
#    The zero floor alone would catch this too, but the count cross-check is
#    what makes the error message true.
jq -c . < "$TMP/vacuity.manifest.json" > "$TMP/compact.manifest.json"
if TEMPER_INSTALL_DIR="$TMP/install-compact" XDG_BIN_HOME="$TMP/bin-compact" \
     sh "$INSTALL" --archive "$TMP/archive.tar.gz" --manifest "$TMP/compact.manifest.json" \
        --version v0.3.0 >/dev/null 2>&1; then
  fail "installer accepted a compact manifest whose entries it could not parse"
fi

echo "PASS: a manifest the parser cannot read is refused, not silently accepted"
```

`jq` is fine *in the harness* (it runs in bash/CI, and `build_manifest` at `:29-33` already relies on it); it is only `install.sh` itself that must stay jq-free.

- [ ] **Step 2: Run the harness to verify the new assertions fail**

```bash
bash scripts/install/test-install.sh
```

Expected: FAIL on "installer accepted a manifest listing zero files".

- [ ] **Step 3: Add the floor and the cross-check**

In `scripts/install/install.sh`, add a declared-count helper beside `manifest_pairs` (after line 248):

```sh
# How many entries the manifest *claims*, counted independently of the awk
# pair parser. `"path"` appears exactly once per entry and nowhere else in the
# manifest shape ({version, target, files:[{path, sha256, size}]}), so
# occurrence count is entry count. Counting OCCURRENCES rather than LINES is
# what makes this catch compact JSON, where every entry shares one line.
manifest_declared_count() {
    grep -o '"path":' "$1" | wc -l | tr -d ' '
}
```

Then replace the tail of `verify_manifest_against_dir` (`install.sh:271`, the `[ "$CHECK_FAILED" -eq 0 ]` line) with:

```sh
    # Nothing parsed means nothing was checked — and an unconditional success
    # return here is a fail-open, not a pass. Two ways in, one floor: a
    # manifest that genuinely lists no files, and a manifest whose entries we
    # failed to parse (compact JSON defeats the awk pass above). The count
    # cross-check separates them so the error message is true in both cases.
    DECLARED=$(manifest_declared_count "$TMPDIR/$MANIFEST")
    if [ "$PAIR_COUNT" -eq 0 ]; then
        echo "error: manifest lists no verifiable files (declared entries: $DECLARED) — refusing to install" >&2
        return 1
    fi
    if [ "$PAIR_COUNT" != "$DECLARED" ]; then
        echo "error: parsed $PAIR_COUNT of $DECLARED manifest entries — refusing to install on a partial parse" >&2
        return 1
    fi
    [ "$CHECK_FAILED" -eq 0 ]
```

`PAIR_COUNT` must be incremented inside the `while` loop. Add `PAIR_COUNT=0` beside `CHECK_FAILED=0` at `:255`, and `PAIR_COUNT=$((PAIR_COUNT + 1))` immediately after the `[ -n "$REL" ] || continue` at `:257`.

**Subshell warning:** the loop at `:256-270` is fed by a here-document (`<<EOF`), not a pipe, so the loop body runs in the *current* shell and `PAIR_COUNT` survives. Do **not** restructure it into `manifest_pairs … | while read`, which would put the loop in a subshell and silently zero both counters.

Update the explanatory comment at `install.sh:219-223` — it currently states the pretty-print assumption as a fact the parser relies on. It is now a fact the parser *verifies*.

- [ ] **Step 4: Run the harness to verify it passes**

```bash
bash scripts/install/test-install.sh
```

Expected: all assertions PASS, including both new ones.

- [ ] **Step 5: Verify POSIX sh compliance**

```bash
sh -n scripts/install/install.sh
```

Expected: no output, exit 0. If `shellcheck` is available, run `shellcheck -s sh scripts/install/install.sh` and address anything new.

- [ ] **Step 6: Commit**

```bash
git add scripts/install/install.sh scripts/install/test-install.sh
git commit -m "fix(install): floor and cross-check manifest parsing (T1.1, sh consumer)"
```

---

## Task 4: `update.rs` attests the manifest it persists

**Files:**
- Modify: `crates/temper-cli/src/commands/update.rs:494-530`
- Test: `crates/temper-cli/src/commands/update.rs` (`mod tests`)

**Interfaces:**
- Consumes: `attestation_fetch::verify_release_attestation_online(&client, &digest, tag)` and `sha256_of_file(&Path) -> Result<String, CliError>` (`update.rs:400-410`) — both already exist and are already used on this path.
- Produces: no new public surface.

**⚠️ Plan/reality gap — read "Adjudication corrections" #1 above.** The adjudication reads as though `update.rs` attests the *wrong* object. It does not. `CLAUDE.md` documents the archive digest as the **correct** subject for `update` ("the archive's digest for `update`, since the archive is what gets installed"). **This task is purely additive: keep the archive attestation exactly as it is, and add a second one for the manifest.** Removing the archive attestation is a regression, not a fix.

**Grounding (CONFORM):** `update.rs:508-530` — `download_to_file` fetches the archive (`:513`) and the manifest (`:514`); `:516` computes `sha256_of_file(&archive_path)`; `:521` verifies the attestation for that digest. The manifest is never hashed and never attested, yet `:528` uses it for the per-file comparison and `install.sh:332` then copies it into `INSTALL_DIR/.temper-manifest.json` — the permanent baseline for every future offline verdict.

`version.rs:38-53` documents precisely why the manifest is its own attested subject, and `build-cli-binaries.yml:215` lists the manifest in `subject-path` alongside the archive, so `/attestations/sha256:{manifest_digest}` resolves independently. The subject already exists; only the call is missing.

- [ ] **Step 1: Write the failing test**

`download_and_verify_release` performs network I/O, so assert on the *structure* rather than driving it. Add to `mod tests` in `update.rs`:

```rust
    /// `update` persists the downloaded manifest into the install dir as the
    /// permanent offline baseline (install.sh copies it to
    /// `.temper-manifest.json`), so it must be attested — not merely
    /// downloaded beside an attested archive. A genuine, still-attested
    /// archive says nothing about a tampered manifest sitting next to it.
    ///
    /// Structural assertion: the download path must verify an attestation for
    /// the manifest's own digest, not only the archive's.
    #[test]
    fn download_path_attests_both_archive_and_manifest() {
        let src = include_str!("update.rs");
        let body = src
            .split("fn download_and_verify_release")
            .nth(1)
            .expect("download_and_verify_release exists");
        let calls = body.matches("verify_release_attestation_online").count();
        assert!(
            calls >= 2,
            "expected attestation checks for BOTH the archive and the manifest, found {calls}"
        );
        assert!(
            body.contains("sha256_of_file(&manifest_path)"),
            "the manifest's own digest must be computed and attested"
        );
    }
```

A source-text assertion is a weak test and is used here only because the real path needs the network. If the task's implementer finds an existing seam in this file that allows injecting a fake attestation client, prefer that and write a behavioral test instead — check how `attestation_fetch`'s own tests do it before settling for the textual form.

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest run -p temper-cli download_path_attests_both_archive_and_manifest
```

Expected: FAIL — `expected attestation checks for BOTH the archive and the manifest, found 1`.

- [ ] **Step 3: Add the manifest attestation**

In `update.rs`, immediately after the existing archive attestation block (around `:516-521`), add:

```rust
        // The archive attestation above covers what gets INSTALLED. This one
        // covers what gets PERSISTED: install.sh copies this manifest into the
        // install dir as `.temper-manifest.json`, where it becomes the
        // permanent baseline for every future offline `--verify`. An
        // unattested baseline means one hour of release-asset write blinds
        // tamper detection for the life of the install — and combined with a
        // vacuous manifest, it blinds it while reporting `verified`.
        //
        // Keyed on the manifest's OWN digest, computed from the bytes just
        // downloaded — never the archive's. `build-cli-binaries.yml:215` lists
        // the manifest as its own `subject-path` entry, so this resolves
        // independently. Same reasoning as `version.rs`'s online path; this is
        // the sibling call that was missed.
        let manifest_digest = sha256_of_file(&manifest_path)?;
        attestation_fetch::verify_release_attestation_online(&client, &manifest_digest, tag)
            .await
            .map_err(|e| {
                CliError::Install(format!(
                    "release attestation for the published manifest failed: {e}"
                ))
            })?;
```

Match the exact error-mapping idiom of the archive call directly above it — read it and mirror it rather than copying this sketch verbatim. Also update the function's doc comment (`update.rs:474-484`) and the module doc (`:19-22`), both of which currently describe attesting only the archive.

- [ ] **Step 4: Run the test to verify it passes**

```bash
cargo nextest run -p temper-cli download_path_attests_both_archive_and_manifest
```

Expected: PASS.

- [ ] **Step 5: Full CLI suite + check**

```bash
cargo nextest run -p temper-cli --lib && cargo make check
```

Expected: all pass, check clean.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/update.rs
git commit -m "fix(cli): attest the manifest update persists as the offline baseline (T1.2)"
```

---

## Task 5: Path containment in both consumers

**Files:**
- Modify: `crates/temper-cli/src/manifest.rs` (`verify_dir`)
- Modify: `scripts/install/install.sh` (`verify_manifest_against_dir`)
- Test: both files' test/harness blocks

**Interfaces:**
- Consumes: the vacuity floors from Tasks 2 and 3 (rebase on them).
- Produces: any manifest entry whose `path` is absolute, contains a `..` component, or is empty is rejected as a hard failure in both consumers.

**Grounding (CONFORM):** `manifest.rs:87` calls `dir.join(&entry.path)`. Rust's `Path::join` **discards the base when the argument is absolute** — so an entry of `/etc/passwd` reads that file, and `temper` itself is never hashed. Equally, `../outside/decoy` escapes the install root. The offline verdict can then be forged by editing only `path` strings. `install.sh:258` has the same shape (`"$CHECK_DIR/$REL"`), where an absolute `$REL` yields `//etc/passwd` — which still resolves.

- [ ] **Step 1: Write the failing Rust test**

```rust
    /// A manifest entry may only name a file *inside* the install dir. An
    /// absolute path makes `Path::join` discard the base entirely, and `..`
    /// escapes it — either way the real `temper` is never hashed and the
    /// offline verdict is forgeable by editing only `path` strings.
    #[test]
    fn escaping_paths_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "temper", b"real");
        for bad in ["/etc/passwd", "../outside/decoy", "a/../../escape", ""] {
            let manifest = ReleaseManifest {
                version: "0.0.0".to_string(),
                target: "t".to_string(),
                files: vec![ManifestEntry {
                    path: bad.to_string(),
                    sha256: sha_of(b"real"),
                    size: 4,
                }],
            };
            assert!(
                !matches!(verify_dir(&manifest, tmp.path()), Verdict::Verified),
                "path {bad:?} must not verify"
            );
        }
    }
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cargo nextest run -p temper-cli escaping_paths_are_rejected
```

Expected: FAIL — at minimum the `""` and `/etc/passwd` cases verify today.

- [ ] **Step 3: Add Rust containment**

Add a private helper to `manifest.rs`:

```rust
/// Whether a manifest entry's `path` names something strictly inside the
/// install dir. Only ordinary path components are allowed: `RootDir` means an
/// absolute path (which `Path::join` honors by discarding the base), `ParentDir`
/// escapes upward, `Prefix` is a Windows drive or UNC root, and `CurDir` is
/// noise no producer emits. `emit-manifest.sh` only ever writes paths relative
/// to `STAGING`, so nothing legitimate is excluded.
fn is_contained_relative(rel: &str) -> bool {
    !rel.is_empty()
        && Path::new(rel)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}
```

In `verify_dir`'s loop, before the `std::fs::read`, reject uncontained entries by pushing a `Mismatch` with `actual: None` — reusing the existing failure channel so the caller's error rendering needs no change:

```rust
        if !is_contained_relative(&entry.path) {
            mismatches.push(Mismatch {
                path: entry.path.clone(),
                expected: entry.sha256.clone(),
                actual: None,
            });
            continue;
        }
```

Add a unit test for `is_contained_relative` directly, covering `foo..bar` (a legal filename that merely *contains* dots and must be **accepted**) alongside the rejections.

- [ ] **Step 4: Run the Rust tests**

```bash
cargo nextest run -p temper-cli --lib
```

Expected: all pass.

- [ ] **Step 5: Write the failing sh harness assertion**

Append to `scripts/install/test-install.sh`, using the same helpers as Task 3 (`INSTALL`, `TMP`, `fail`, `build_archive`, `build_manifest`):

```bash
# --- A manifest entry may not name a file outside the install dir ------------
# `$CHECK_DIR/$REL` with an absolute $REL resolves to the absolute path, so the
# real `temper` is never hashed and the verdict is forgeable by editing only
# `path` strings. Note the sha256 is left correct for /etc/hosts' ACTUAL
# content being irrelevant — the entry must be rejected on its path alone,
# before anything is read.
build_archive
build_manifest "$TMP/stage" "$TMP/containment.manifest.json"

for BAD_PATH in "/etc/hosts" "../escape" "a/../../escape"; do
  jq --arg p "$BAD_PATH" '.files[0].path = $p' \
    < "$TMP/containment.manifest.json" > "$TMP/bad-path.manifest.json"
  if TEMPER_INSTALL_DIR="$TMP/install-badpath" XDG_BIN_HOME="$TMP/bin-badpath" \
       sh "$INSTALL" --archive "$TMP/archive.tar.gz" --manifest "$TMP/bad-path.manifest.json" \
          --version v0.3.0 >/dev/null 2>&1; then
    fail "installer accepted a manifest entry with an escaping path: $BAD_PATH"
  fi
  rm -rf "$TMP/install-badpath"
done

echo "PASS: manifest entries that escape the install root are refused"
```

- [ ] **Step 6: Add sh containment**

In `verify_manifest_against_dir`, immediately after the `[ -n "$REL" ] || continue` line, add:

```sh
        # Reject anything that is not strictly inside CHECK_DIR. `case` on the
        # path wrapped in slashes tests for a `..` COMPONENT, so an ordinary
        # filename that merely contains dots (foo..bar) is still accepted.
        case "$REL" in
            /*)
                echo "error: manifest entry has an absolute path: $REL" >&2
                CHECK_FAILED=1
                continue
                ;;
        esac
        case "/$REL/" in
            *"/../"*)
                echo "error: manifest entry escapes the install root: $REL" >&2
                CHECK_FAILED=1
                continue
                ;;
        esac
```

Note this runs *after* `PAIR_COUNT` is incremented (Task 3), so a rejected entry still counts as parsed — the cross-check stays meaningful and the failure is reported by `CHECK_FAILED`.

- [ ] **Step 7: Run both harnesses and check**

```bash
bash scripts/install/test-install.sh && sh -n scripts/install/install.sh && cargo make check
```

Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/manifest.rs scripts/install/install.sh scripts/install/test-install.sh
git commit -m "fix: reject manifest entries that escape the install root (T1.3)"
```

---

## Task 6: Scope the seven unscoped workflows

**Files:**
- Modify: `.github/workflows/ci.yml`, `code-quality.yml`, `coverage.yml`, `test-agents-ts.yml`, `test-ruby.yml`, `test-rust.yml`, `test-typescript.yml`

**Interfaces:** none — pure CI configuration.

**⚠️ Read "Adjudication corrections" #2 above.** The signing jobs are **already correctly scoped**; do not touch `build-cli-binaries.yml`, `release.yml`, or `release-tag.yml`. Exactly these seven files are unscoped.

**Grounding (CONFORM), verified in planning:**
- `gh api repos/:owner/:repo/actions/permissions/workflow` → `{"default_workflow_permissions":"write", …}`. Unscoped jobs therefore hold **every** scope at write, including `actions: write`.
- `rg '^\s*permissions:' -A6` across `.github/workflows/` shows blocks only in `build-cli-binaries.yml:25`, `release.yml:47` and `:61`, `release-tag.yml:19` and `:60`.
- Zero-breakage confirmed: no unscoped workflow posts PR comments, pushes, or otherwise uses `GITHUB_TOKEN` for writes. `coverage.yml:97-99` uploads via `codecov/codecov-action@v5` using `secrets.CODECOV_TOKEN`, not `GITHUB_TOKEN`.
- `ci.yml` invokes the others via `workflow_call`; a called workflow inherits the caller's permissions unless it declares its own, so scoping `ci.yml` propagates — but scope each file anyway, since `code-quality.yml` and the `test-*.yml` set are independently callable.

- [ ] **Step 1: Add the block to each workflow**

Insert immediately after the `on:` block (and before `concurrency:`/`jobs:`) in each of the seven files:

```yaml
# Least privilege. The org default is `default_workflow_permissions: write`,
# which grants EVERY scope — including `actions: write` and `contents: write` —
# to any job that does not say otherwise. This repo now publishes SIGNED
# release artifacts, so a compromised action or build.rs in a test job would
# reach a signing oracle: create a tag at an attacker commit, then
# workflow_dispatch release.yml against it, and the backdoor is attested.
# (A tag pushed with GITHUB_TOKEN does not itself trigger release.yml — GitHub
# suppresses workflow runs from GITHUB_TOKEN-authored events — which is why the
# dispatch step matters and why `actions: write` is the scope that completes
# the chain.) Declaring any permissions block zeroes every unlisted scope.
permissions:
  contents: read
```

- [ ] **Step 2: Verify every workflow now declares a scope**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
for f in .github/workflows/*.yml; do
  printf '%-28s ' "$(basename "$f")"
  rg -q '^\s*permissions:' "$f" && echo "scoped" || echo "UNSCOPED"
done
```

Expected: every file reports `scoped`.

- [ ] **Step 3: Validate the YAML parses**

```bash
for f in .github/workflows/*.yml; do python3 -c "import sys,yaml; yaml.safe_load(open(sys.argv[1]))" "$f" || echo "BAD: $f"; done
```

Expected: no `BAD:` lines.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/
git commit -m "fix(ci): scope the seven unscoped workflows to contents: read (T2.1)"
```

> **Note for the reviewer at plan close:** this is the one change whose verification is only complete after CI runs green on the PR. If any job fails on a permissions error, grant the single scope it needs on that job — never revert the top-level block.

---

## Task 7: Pass the tag through `env:` instead of interpolating into bash

**Files:**
- Modify: `.github/workflows/release.yml:38`

**Grounding (CONFORM):** `release.yml:38` reads `TAG="${{ github.ref_name }}"` inside a `run:` block. GitHub expands `${{ }}` into the script text *before* bash sees it, so a crafted ref name executes. Verified in the review that `git check-ref-format` accepts `v1.0.0";id;#`, `` v1.0.0`id` ``, and `v1.0.0$(id)`. The job this lands in holds `id-token: write` and `attestations: write`.

- [ ] **Step 1: Move the interpolation to `env:`**

Add an `env:` block to that step and read the value as a shell variable, which is never re-parsed as script text:

```yaml
        env:
          REF_NAME: ${{ github.ref_name }}
        run: |
          TAG="$REF_NAME"
```

Read the surrounding step first — if `github.ref_name` is referenced more than once in that `run:` body, convert every occurrence. Quote every use of `$REF_NAME`/`$TAG`.

- [ ] **Step 2: Verify no `${{ }}` remains inside any `run:` body in the release path**

```bash
rg -n 'run: \|' -A20 .github/workflows/release.yml | rg -n 'github\.(ref_name|event|head_ref)'
```

Expected: no matches. If any remain, convert them the same way.

- [ ] **Step 3: Validate YAML and commit**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"
git add .github/workflows/release.yml
git commit -m "fix(ci): pass the tag through env rather than interpolating into bash (T2.2)"
```

---

## Task 8: Pin the ONNX Runtime download's sha256

**Files:**
- Modify: `.github/workflows/build-cli-binaries.yml:29-59` (matrix) and `:90-110` (download step)

**Grounding (CONFORM):** `build-cli-binaries.yml:100` runs `curl -fsSL -o "ort-staging/ort.${ORT_EXT}" "$ORT_URL"` with **no integrity check**. The extracted library is then copied into staging, hashed into the manifest, and signed — and the shipped binary `dlopen`s it. `attest.rs` already cites `EXPECTED_MODEL_SHA256` as this repo's doctrine for exactly this problem: the *model* is pinned, the *native library loaded beside it* is not.

- [ ] **Step 1: Obtain the real digests**

For each of the three targets, download the archive named by the matrix (`onnxruntime-osx-arm64`, `onnxruntime-linux-x64`, `onnxruntime-win-x64` at `env.ONNX_RUNTIME_VERSION`) and record its sha256. Read `ONNX_RUNTIME_VERSION` from the workflow's `env:` block first — do not assume a version.

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
ORT_VER=$(rg -n 'ONNX_RUNTIME_VERSION' .github/workflows/build-cli-binaries.yml | head -1)
echo "$ORT_VER"   # read the literal, then substitute below
for n in onnxruntime-osx-arm64:tgz onnxruntime-linux-x64:tgz onnxruntime-win-x64:zip; do
  name="${n%%:*}"; ext="${n##*:}"
  url="https://github.com/microsoft/onnxruntime/releases/download/v<VER>/${name}-<VER>.${ext}"
  printf '%s  ' "$name"
  curl -fsSL "$url" | shasum -a 256 | awk '{print $1}'
done
```

Record all three. **These are load-bearing constants — paste the real output into the workflow, never a placeholder.**

- [ ] **Step 2: Add `ort_sha256` to each matrix entry**

Add one field per target at `build-cli-binaries.yml:35-58`, beside the existing `ort_archive`/`ort_archive_ext`:

```yaml
            ort_sha256: <the digest recorded in Step 1 for this target>
```

- [ ] **Step 3: Verify after download**

In the "Download ONNX Runtime" step, insert immediately after the `curl` at `:100` and before the extraction at `:103`:

```bash
          # Pin the native library we are about to sign. This archive is
          # copied into staging, hashed into the manifest, attested, and then
          # dlopen'd by the shipped binary — so an unpinned fetch means we
          # faithfully sign whatever the network handed us. The embedding model
          # beside it is already pinned via EXPECTED_MODEL_SHA256 (attest.rs
          # cites it as the doctrine); this closes the other half.
          EXPECTED_ORT_SHA="${{ matrix.target.ort_sha256 }}"
          if command -v sha256sum >/dev/null 2>&1; then
            ACTUAL_ORT_SHA=$(sha256sum "ort-staging/ort.${ORT_EXT}" | awk '{print $1}')
          else
            ACTUAL_ORT_SHA=$(shasum -a 256 "ort-staging/ort.${ORT_EXT}" | awk '{print $1}')
          fi
          if [ "$ACTUAL_ORT_SHA" != "$EXPECTED_ORT_SHA" ]; then
            echo "error: ONNX Runtime archive sha256 mismatch" >&2
            echo "  expected: $EXPECTED_ORT_SHA" >&2
            echo "  actual:   $ACTUAL_ORT_SHA" >&2
            exit 1
          fi
          echo "ONNX Runtime archive sha256 verified: $ACTUAL_ORT_SHA"
```

The `sha256sum`/`shasum` branch matches the one already used at `emit-manifest.sh:23-29` and `install.sh:224-230` — the matrix spans macOS and Linux runners, so never assume one.

- [ ] **Step 4: Document the bump obligation**

Pinning creates a maintenance obligation exactly like the Sigstore root rotation. Add a short subsection to `docs/guides/releasing.md` stating that bumping `ONNX_RUNTIME_VERSION` requires recomputing all three `ort_sha256` values, with the command from Step 1. Link it from the workflow comment.

- [ ] **Step 5: Validate YAML and commit**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/build-cli-binaries.yml'))"
git add .github/workflows/build-cli-binaries.yml docs/guides/releasing.md
git commit -m "fix(ci): pin the ONNX Runtime download's sha256 before signing it (T3.1)"
```

---

## Task 9: Harden the signing job's toolchain and cache

**Files:**
- Modify: `.github/workflows/build-cli-binaries.yml:62-80`

**Grounding (CONFORM):** `build-cli-binaries.yml:71` uses `dtolnay/rust-toolchain@stable` — a **mutable branch ref**, resolved fresh inside the job holding `id-token: write`. `:76` uses `Swatinem/rust-cache@v2`, which restores a cache writable from `main` scope into that same job, so a poisoned `target/` gets truthfully attested. `:62` `actions/checkout@v6`, `:206` `actions/attest-build-provenance@v4`, `:221` `actions/upload-artifact@v7` are all mutable major-version tags.

**Adjudication's endorsed position (T3.3), follow it:** SHA-pin **only** the actions inside the signing job, not repo-wide — repo-wide pinning generates ~20 rubber-stamped bump PRs a year, which is net-negative. And **replace** `dtolnay/rust-toolchain@stable` with `rustup` rather than pinning it, because a branch ref has no release cadence, so pinning it means bumping blind.

- [ ] **Step 1: Replace the toolchain action with rustup**

Substitute the `dtolnay/rust-toolchain@stable` step at `:71` with a `run:` step that uses the runner's preinstalled rustup. Read the existing step first for the exact `targets:`/`components:` it requests and carry every one across:

```yaml
      - name: Install Rust toolchain
        shell: bash
        run: |
          # rustup is preinstalled on all three runner images. Used directly
          # rather than dtolnay/rust-toolchain@stable because that action
          # resolves a MUTABLE branch ref inside the job that holds
          # `id-token: write` — and a branch ref has no release cadence, so
          # SHA-pinning it would mean bumping blind forever.
          rustup toolchain install stable --profile minimal --no-self-update
          rustup default stable
          rustup target add "${{ matrix.target.triple }}"
```

- [ ] **Step 2: SHA-pin the remaining actions in this workflow**

Resolve each to its current commit SHA and pin with the version in a trailing comment:

```bash
for a in actions/checkout:v6 actions/attest-build-provenance:v4 actions/upload-artifact:v7 Swatinem/rust-cache:v2; do
  repo="${a%%:*}"; ver="${a##*:}"
  sha=$(gh api "repos/$repo/git/ref/tags/$ver" --jq '.object.sha' 2>/dev/null \
     || gh api "repos/$repo/commits/$ver" --jq '.sha')
  echo "$repo@$sha # $ver"
done
```

If a tag resolves to an annotated tag object, dereference it to the commit (`gh api repos/$repo/git/tags/$sha --jq '.object.sha'`). Apply each as `uses: owner/repo@<sha> # <version>`.

- [ ] **Step 3: Decide the cache question explicitly**

`Swatinem/rust-cache@v2` restores a cache that jobs on `main` can write, into the signing job. Two defensible options — **pick one and record the reason in a comment**:
- **Drop the cache from this job.** Release builds are infrequent; the cost is build time on a path that must be trustworthy. Strongest.
- **Keep it, pinned, with a release-scoped `key`** so it never shares a namespace with PR/`main` jobs.

Prefer dropping it unless the measured build-time cost is unacceptable; a release job that runs on tags does not need a warm cache. Whichever is chosen, state the tradeoff in the comment — a future reader must not have to re-derive it.

- [ ] **Step 4: Validate YAML and commit**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/build-cli-binaries.yml'))"
git add .github/workflows/build-cli-binaries.yml
git commit -m "fix(ci): pin the signing job's actions and drop the mutable toolchain ref (T3.2, T3.3)"
```

---

## Task 10: Cross-language manifest round-trip guard

**Files:**
- Create: `.github/scripts/test-manifest-roundtrip.sh`
- Modify: `.github/workflows/code-quality.yml` (the `guard-tests` job, after the existing release-integrity harnesses at `:179-183`)

**Grounding (CONFORM):** the wire format is proven twice and never against itself. `manifest.rs:112`'s golden fixture is `crates/temper-cli/tests/fixtures/manifest-golden.json`, **hand-authored and never regenerated** from `emit-manifest.sh`. `.github/scripts/test-emit-manifest.sh` checks the producer against its own `jq` queries. Nothing feeds **real producer bytes into the real Rust deserializer**, so a coordinated field rename on both bash sides ships while Rust silently stops parsing.

- [ ] **Step 1: Write the guard**

Create `.github/scripts/test-manifest-roundtrip.sh`. It must:
1. Build a small staging dir with two or three real files.
2. Run the **real** `.github/scripts/release/emit-manifest.sh` against it.
3. Feed the resulting bytes to the **real** Rust deserializer, and assert it parses and verifies clean.

For step 3, prefer an existing binary entry point over a throwaway crate. Check first whether `temper version --verify` can be pointed at a directory (read `version.rs`'s `--verify` handling and `load_from_dir`); if it can, copy the manifest in as `.temper-manifest.json` and assert a `verified` verdict. If no such seam exists, add a `#[test]` in `manifest.rs` that shells out to `emit-manifest.sh` and deserializes its output — a Rust-side round-trip is equally valid and cheaper to wire, in which case this task produces a Rust test plus a one-line CI note instead of a bash guard. **Read the code and choose; do not build a new binary for this.**

- [ ] **Step 2: Prove the guard bites**

Whichever form Step 1 took, feed it a deliberately broken fixture — rename `sha256` to `hash` in the producer's output before handing it to Rust — and confirm it goes red. This is the whole point of the house style (`code-quality.yml:130-137`): a passing tripwire that can no longer fail is worth nothing.

```bash
bash .github/scripts/test-manifest-roundtrip.sh   # or: cargo nextest run -p temper-cli roundtrip
```

Expected: PASS on real bytes, and RED on the renamed-field fixture.

- [ ] **Step 3: Regenerate the golden fixture from the real producer**

Replace `crates/temper-cli/tests/fixtures/manifest-golden.json` with output from an actual `emit-manifest.sh` run, so the fixture stops being hand-authored. Add a comment (or a sibling `README`) recording the exact command that generated it.

- [ ] **Step 4: Wire it into CI**

If Step 1 produced a bash guard, add to the `guard-tests` job in `code-quality.yml` after line 183:

```yaml
      - name: Guard test — manifest round-trip (producer bytes → Rust deserializer)
        run: bash .github/scripts/test-manifest-roundtrip.sh
```

If it needs cargo, it belongs in `rust-quality` instead — `code-quality.yml:186` notes `guard-tests` is deliberately toolchain-free, and `:170-172` shows the precedent for a guard that needs cargo living elsewhere.

- [ ] **Step 5: Commit**

```bash
git add .github/scripts/ crates/temper-cli/ .github/workflows/code-quality.yml
git commit -m "test: cross-language manifest round-trip guard (Tier 4)"
```

---

## Task 11: `audit-attest-subject-path.sh` — the attestation covers both subjects

**Files:**
- Create: `.github/scripts/audit-attest-subject-path.sh`
- Create: `.github/scripts/test-audit-attest-subject-path.sh`
- Modify: `.github/workflows/code-quality.yml` (add the guard to `rust-quality`'s tripwire list at `:92-111`, and the harness to `guard-tests` at `:158-183`)

**Grounding (CONFORM):** `build-cli-binaries.yml:206-218` — `actions/attest-build-provenance@v4` with a `subject-path:` block listing both the archive and `…manifest.json`. Deleting the manifest line silently defeats `--verify --online` and `update`'s new manifest attestation (Task 4) until a real release breaks. Nothing checks it today.

- [ ] **Step 1: Write the guard**

Create `.github/scripts/audit-attest-subject-path.sh`, modeled on the existing enumerators (read `.github/scripts/audit-signature-secrets.sh` first and match its structure, exit conventions, and output style). It must assert that the `attest-build-provenance` step's `subject-path` block names **both** an archive pattern and a `.manifest.json` pattern, and exit non-zero otherwise. Accept an optional `$1` for the workflow path so the harness can point it at a fixture; default to `.github/workflows/build-cli-binaries.yml`.

- [ ] **Step 2: Write its harness**

Create `.github/scripts/test-audit-attest-subject-path.sh`, modeled on `.github/scripts/test-audit-signature-secrets.sh`. It must:
1. Copy the real workflow to a temp file, delete the `manifest.json` line from `subject-path`, run the guard against it, and assert it exits **non-zero**.
2. Run the guard against the real workflow and assert it exits **zero**.

- [ ] **Step 3: Prove both directions**

```bash
bash .github/scripts/audit-attest-subject-path.sh          # expect exit 0
bash .github/scripts/test-audit-attest-subject-path.sh     # expect all PASS
```

- [ ] **Step 4: Wire both into CI**

Guard into `rust-quality`'s tripwire list (beside `audit-credential-debug.sh` at `:111`):

```yaml
      - name: Audit — attestation subject paths
        run: bash .github/scripts/audit-attest-subject-path.sh
```

Harness into `guard-tests` (beside the release-integrity harnesses at `:179-183`):

```yaml
      - name: Guard test — audit-attest-subject-path
        run: bash .github/scripts/test-audit-attest-subject-path.sh
```

It is pure bash, so `guard-tests` is the right home for the harness.

- [ ] **Step 5: Commit**

```bash
chmod +x .github/scripts/audit-attest-subject-path.sh .github/scripts/test-audit-attest-subject-path.sh
git add .github/scripts/ .github/workflows/code-quality.yml
git commit -m "test: guard that the attestation covers both archive and manifest (Tier 4)"
```

---

## Task 12: Make the Windows refusal test actually execute

**Files:**
- Modify: `crates/temper-cli/src/commands/update.rs:101-106`, `:168-169`, `:813-819`

**Grounding (CONFORM):** `WINDOWS_REFUSAL` at `:102` is `#[cfg(windows)]` (`:101`), and its test at `:816` is `#[cfg(windows)]` too (`:814-815`). **No workflow uses a Windows runner** for tests — verified: zero `runs-on: windows*` outside the release build matrix. The constant's properties were checked by inspection during Arc 1 and by nothing since. This is the repo's own named rot: "a test no job runs is a test that runs nowhere" (`code-quality.yml:138`).

**Do not stand up a Windows runner for one string assertion.** Drop the `cfg` from the constant so its test compiles and runs everywhere.

- [ ] **Step 1: Drop `cfg(windows)` from the constant and its test**

Remove `#[cfg(windows)]` at `:101` (the constant) and at `:814` (the test). **Leave** the `#[cfg(windows)]` at `:168` on the `InstallLayout::WindowsScript` match arm — that arm genuinely is Windows-only, and removing it would break the non-Windows build unless `InstallLayout` also carries the variant unconditionally. Check `InstallLayout`'s definition before touching it.

- [ ] **Step 2: Handle the now-unused warning**

On non-Windows the constant is referenced only by its test, so a `dead_code` warning is likely. Do **not** silence it with `#[allow]` — use the repo convention:

```rust
#[expect(dead_code, reason = "referenced by the Windows refusal arm and by its test, which must \
    run on every host: no CI job uses a Windows runner, so a cfg(windows) test runs nowhere")]
```

Prefer restructuring so no suppression is needed if a clean option exists.

- [ ] **Step 3: Verify the test now runs on this host**

```bash
cargo nextest run -p temper-cli windows_refusal_is_actionable_and_not_the_cargo_hint
```

Expected: PASS — and, critically, **that it is listed at all**, which it is not today. Confirm with:

```bash
cargo nextest list -p temper-cli | rg windows_refusal
```

- [ ] **Step 4: Full check and commit**

```bash
cargo nextest run -p temper-cli --lib && cargo make check
git add crates/temper-cli/src/commands/update.rs
git commit -m "test: run the Windows refusal assertion on every host (Tier 4)"
```

---

## Task 13: Tier 5 — the honest statement in the docs

**Files:**
- Modify: `docs/guides/releasing.md`
- Modify: whichever guide documents `temper update` / `--verify` for users — locate with `rg -l 'verify --online|attestation' docs/`
- Modify: `crates/temper-cli/src/attest.rs` module doc if it overstates

**Grounding (AMEND — the adjudication's Tier 5 authorizes this):** the docs currently imply the attestation proves more than it does.

**The statement to make, in the docs' own voice:**

> **The attestation binds the builder and the tag. It never binds the source.**
> Anyone with repo write can push a tag whose workflow builds a backdoor, and it will verify
> perfectly on every path — correct signature, correct identity, correct inclusion proof. This is
> inherent to build provenance, not a defect in this implementation.

Plus the two residual-trust items named in the same place:
1. **The `curl | main/install.sh` bootstrap is unsigned** — and it is what every trust-root error message names as the recovery path.
2. **The trust root is a hand-committed blob** (`crates/temper-cli/trust/sigstore-public-good-trusted-root.json`) with no freshness or provenance check of its own.

- [ ] **Step 1: Locate every place that describes what verification proves**

```bash
rg -n 'attestation|provenance|verified' docs/ --glob '*.md' -l
```

Read each hit. Note any sentence that a reader could take as "this proves the source is what you see on GitHub" — that is the class to amend.

- [ ] **Step 2: Add the honest statement**

Add a "What the attestation does and does not prove" section to `docs/guides/releasing.md`, adjacent to the existing "Standing obligation: Sigstore root rotation" section (which is the established precedent for recording a limitation in this file). Carry the statement above verbatim, then the two residual-trust items.

- [ ] **Step 3: Amend any overstating sentence found in Step 1**

Rewrite each in place. Do not delete the claim and leave a gap — state the narrower true thing. Cross-link to the new section rather than restating it in full (one source of truth for the limitation).

- [ ] **Step 4: Check the Rust docs**

```bash
rg -n 'proves|guarantees|ensures' crates/temper-cli/src/attest.rs crates/temper-cli/src/attestation_fetch.rs
```

Amend any that overstate. `attest.rs`'s pinned-root rationale is already honest; verify rather than assume.

- [ ] **Step 5: Lint and commit**

```bash
cargo make check
git add docs/ crates/temper-cli/src/
git commit -m "docs: the attestation binds builder and tag, never source (Tier 5)"
```

---

## Close-out

After Task 13:

- [ ] `cargo make check` clean
- [ ] `cargo nextest run -p temper-cli --lib` — all pass
- [ ] `bash scripts/install/test-install.sh` — all assertions pass
- [ ] `bash .github/scripts/test-emit-manifest.sh` — all assertions pass
- [ ] Every new guard proven RED against a broken fixture, not merely green
- [ ] `git merge origin/main` before any push
- [ ] Single consolidated review pass over the whole branch (per the user's standing review cadence — not per task)
- [ ] Update the adjudication doc with a resolution line per finding, and note the three corrections this plan made to it
- [ ] PR #573 out of draft — **the user's call, not the implementer's**

## Deliberately not in this plan

- **A Windows runner.** Task 12 makes the assertion run everywhere instead; standing up a runner for one string is not warranted, and the Windows manifest hole stays a *declared* hole (`update.rs:93-106`).
- **Repo-wide action SHA-pinning.** Task 9 pins the signing job only, per the adjudication's endorsed reasoning (T3.3): repo-wide pinning yields ~20 rubber-stamped bump PRs a year and is net-negative.
- **Re-litigating the two "Not findings"** (`adjudication.md:192-199`): fresh installs performing no attestation check, and `bundle_url` pointing at a third-party blob host. Both were reasoned and closed.
