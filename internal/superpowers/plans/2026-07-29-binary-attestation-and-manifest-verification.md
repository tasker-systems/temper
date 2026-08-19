# Binary Attestation and Per-File Manifest Verification — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish a per-file hash manifest and a signed build-provenance attestation for every release, and make `install.sh` and `temper update` refuse to complete an install whose files do not match them.

**Architecture:** Extends the existing `EXPECTED_MODEL_SHA256` doctrine (`crates/temper-ingest/build.rs`, `embed.rs:313,382`) one level up: derive expected hashes at build time from the real artifact, compile the trust root in, verify at use, refuse on mismatch. The release gains a `.manifest.json` and a GitHub attestation. `install.sh` gains mandatory per-file verification and an `--archive <path>` flag; `temper update` verifies the attestation natively in Rust, then hands the *verified* archive to the script so verification and installation cannot describe different objects.

**Tech Stack:** Rust (clap, serde, sha2, reqwest), POSIX sh, GitHub Actions, sigstore (crate selected by Task 1).

**Spec:** `internal/superpowers/specs/2026-07-29-binary-attestation-and-manifest-verification-design.md` — read it before starting. This plan is an index over that spec, not a replacement for it.

## Global Constraints

- **Read the spec section each task cites.** The spec carries rationale this plan deliberately does not restate.
- **`cargo make check` and `cargo make test` must pass before every commit.** The pre-commit hook runs fmt, clippy, docs, OpenAPI, TS typecheck, and biome.
- **Never use `#[allow]`** — this repo uses `#[expect(lint_name, reason = "...")]`.
- **All public types implement `Debug`.**
- **Typed structs over `serde_json::json!()`** — a manifest has a known structure, so it gets a struct.
- **`install.sh` is POSIX `sh`, not bash.** No `[[ ]]`, no arrays, no `local`. It runs under `sh -s` from `update.rs:323`. Its only tools are `curl`, `tar`, `shasum`/`sha256sum`, and coreutils.
- **The dual-tool checksum pattern is mandatory:** macOS has `shasum -a 256`, Linux has `sha256sum`. See `install.sh:128-134` and `build-cli-binaries.yml:177,189`. Never assume one.
- **`temper-cli`'s `reqwest` is `default-features = false, features = ["rustls-tls"]`** (`crates/temper-cli/Cargo.toml`). Any new dependency that drags in `openssl` is a regression — check with `cargo tree`.
- **Adding a bash guard means adding its harness to the `guard-tests` job in the same PR** — `code-quality.yml:130-142` documents this as a past rot: two harnesses "were committed with a `bash …` usage line and wired into NO job — so they ran nowhere."
- **Windows is deferred, and must be declared, not silent.** `temper version --verify` on Windows reports `unverifiable`, never `verified`. See spec §"Out of scope → Deferred".

## ⚠️ Both-ends collision (audit before editing)

`update.rs:602-603` asserts on the *content of* `install.sh`:

```rust
assert!(INSTALL_SH.contains("REPO=\"tasker-systems/temper\""));
assert!(INSTALL_SH.contains("Verifying checksum"));
```

`install.sh` is embedded into the binary via `include_str!` (`update.rs:50`). **Editing the `echo "  Verifying checksum..."` line at `install.sh:126` breaks a test in a different crate.** Task 6 changes that region — it must keep the literal substring `Verifying checksum` intact or update the assertion in the same commit.

---

## Task 1: Spike — sigstore crate selection with a pinned trust root

**This task writes no production code. Its deliverable is a decision plus evidence.** Its outcome selects the architecture for Task 9. A BLOCKED result is a legitimate, expected outcome — not a failure.

**Files:**
- Create: `internal/superpowers/spikes/2026-07-29-sigstore-crate-evaluation.md`
- Scratch (do not commit): a throwaway crate outside the workspace

**Spec sections to read:** "The pinned trust root", "The spike, and its BLOCKED arm"

**Interfaces:**
- Produces: a decision recorded in the spike doc — `CRATE = sigstore-verification | sigstore-verify | BLOCKED`, plus, if not BLOCKED, the exact API call shape Task 9 will use and what artifact must be pinned (root cert / intermediate / Rekor key).

**The question, verbatim from the spec:**

> Can `sigstore-verification` or `sigstore-verify` verify a real GitHub attestation bundle against a **caller-supplied, pinned** trust root, with no network TUF fetch?

**Decision rule:**
- **Yes** → record the crate, version, and call shape. Task 9 proceeds as designed.
- **No** → record **BLOCKED**. Tasks 2–8 and 10 still ship; Task 9 degrades to the fallback (attestation published and documented for out-of-band `gh attestation verify`, no in-band verification).
- **Never** hand-roll certificate-chain verification with `x509-parser`/`p256` to rescue the "yes" arm.

- [ ] **Step 1: Produce a real attestation bundle to test against**

Do not fabricate a fixture. Use a real one from any public repo that publishes attestations:

```bash
gh attestation download --repo cli/cli --predicate-type https://slsa.dev/provenance/v1 \
  $(gh release download --repo cli/cli --pattern '*linux_amd64.tar.gz' --dir /tmp/att -O /tmp/att/a.tgz && echo /tmp/att/a.tgz) \
  --output-dir /tmp/att || gh attestation verify --repo cli/cli /tmp/att/a.tgz --format json > /tmp/att/verified.json
```

If `gh` is unavailable, fetch a bundle from the GitHub attestations API for any public repo. Record in the spike doc exactly which artifact and bundle you used.

- [ ] **Step 2: Scaffold a throwaway crate outside the workspace**

Outside the repo so it cannot pollute `Cargo.lock` or the workspace:

```bash
cargo new --bin /tmp/sigstore-spike && cd /tmp/sigstore-spike
cargo add sigstore-verification@0.2.8
```

- [ ] **Step 3: Determine whether a caller-supplied trust root is reachable**

Read the actual API surface — do not infer it from the crates.io blurb:

```bash
cargo doc --no-deps --open   # or:
cargo tree -p sigstore-verification
rg -n "trust_root|TrustRoot|trusted_root|with_root|Policy" ~/.cargo/registry/src/*/sigstore-verification-0.2.8/src/ | head -40
```

Answer three things in the spike doc, each with a quoted excerpt or command output:
1. Does `AttestationClientBuilder` (or `Policy`, or `verify_github_attestation`) accept a caller-supplied trusted root?
2. If not supplied, does verification perform a **network TUF fetch**? (Look for a TUF client or a hardcoded sigstore TUF URL.)
3. What concrete artifact would we pin — Fulcio root cert, an intermediate, the Rekor public key, or a full `TrustedRoot` protobuf?

- [ ] **Step 4: Attempt an actual offline verification**

Write a `main.rs` that verifies the Step 1 bundle with **networking disabled**, to prove no TUF fetch happens:

```rust
fn main() {
    // Verify the bundle from Step 1 against a pinned/offline trust root.
    // The exact call comes from Step 3's reading of the real API.
    // Run this with network egress blocked to prove no TUF fetch occurs.
}
```

Run it with egress blocked so a silent network fetch cannot masquerade as success:

```bash
# macOS: use Network Link Conditioner or a firewall rule.
# Linux:
sudo unshare -n cargo run
```

Expected on the "yes" arm: verification succeeds with no network. Expected on the "no" arm: it fails or hangs on a TUF fetch.

- [ ] **Step 5: Repeat Steps 2–4 for `sigstore-verify@0.11.0`**

Do not skip this because the first crate worked. The spec names both; the comparison is the deliverable.

- [ ] **Step 6: Check dependency hygiene for the winner**

```bash
cargo tree -p sigstore-verification 2>/dev/null | rg -i "openssl|native-tls"
```

Expected: **no matches**. `temper-cli` pins `reqwest` to `rustls-tls` with `default-features = false`; an `openssl` transitive dependency is a regression and must be recorded as a cost in the spike doc.

- [ ] **Step 7: Write the spike document**

Record, with evidence (quoted output, not narration):
- Which bundle was tested.
- Per crate: caller-supplied root reachable? network TUF fetch? what gets pinned?
- Offline verification result with egress blocked.
- Dependency hygiene result.
- **The decision:** `CRATE = <name>@<version>` or `BLOCKED`, and the exact call shape for Task 9.

- [ ] **Step 8: Commit**

```bash
git add internal/superpowers/spikes/2026-07-29-sigstore-crate-evaluation.md
git commit -m "spike: evaluate sigstore crates for pinned-root attestation verification"
```

---

## Task 2: `ReleaseManifest` type and verification logic

Pure Rust, no I/O beyond reading files. Fully unit-testable and independent of the workflow.

**Files:**
- Create: `crates/temper-cli/src/manifest.rs`
- Modify: `crates/temper-cli/src/lib.rs` (add `pub mod manifest;`)
- Create: `crates/temper-cli/tests/fixtures/manifest-golden.json`

**Spec sections to read:** "What the release publishes", "Honesty constraints"

**Interfaces:**
- Produces (relied on by Tasks 3, 7, 8, 9):
  - `pub struct ReleaseManifest { pub version: String, pub target: String, pub files: Vec<ManifestEntry> }`
  - `pub struct ManifestEntry { pub path: String, pub sha256: String, pub size: u64 }`
  - `pub enum Verdict { Verified, Mismatch(Vec<Mismatch>), Unverifiable(String) }`
  - `pub struct Mismatch { pub path: String, pub expected: String, pub actual: Option<String> }`
  - `pub fn verify_dir(manifest: &ReleaseManifest, dir: &Path) -> Verdict`
  - `pub fn load_from_dir(dir: &Path) -> Option<ReleaseManifest>`
  - `pub const MANIFEST_FILENAME: &str = ".temper-manifest.json";`

**Design note (CONFORM):** `Verdict` distinguishes `Mismatch` from `Unverifiable` because the codebase already treats "we cannot tell" and "it is wrong" as different claims — see `update.rs:58` (`CARGO_REFUSAL`) and `version.rs:32` (`CHECKSUM_NOTE`). A `cargo install` build must render `Unverifiable`, never `Mismatch`.

- [ ] **Step 1: Write the failing tests**

Create `crates/temper-cli/src/manifest.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &std::path::Path, rel: &str, bytes: &[u8]) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    fn sha_of(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(bytes))
    }

    /// A directory whose files match every manifest entry verifies clean.
    #[test]
    fn matching_dir_verifies() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "temper", b"binary-bytes");
        write(tmp.path(), "lib/libonnxruntime.so", b"ort-bytes");

        let m = ReleaseManifest {
            version: "0.3.0".into(),
            target: "x86_64-unknown-linux-gnu".into(),
            files: vec![
                ManifestEntry { path: "temper".into(), sha256: sha_of(b"binary-bytes"), size: 12 },
                ManifestEntry {
                    path: "lib/libonnxruntime.so".into(),
                    sha256: sha_of(b"ort-bytes"),
                    size: 9,
                },
            ],
        };
        assert!(matches!(verify_dir(&m, tmp.path()), Verdict::Verified));
    }

    /// A single altered byte is reported as a Mismatch naming that file — this
    /// is the bite: it must fail against the state it claims to detect.
    #[test]
    fn altered_file_is_a_mismatch_naming_the_path() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "temper", b"TAMPERED");

        let m = ReleaseManifest {
            version: "0.3.0".into(),
            target: "x86_64-unknown-linux-gnu".into(),
            files: vec![ManifestEntry {
                path: "temper".into(),
                sha256: sha_of(b"binary-bytes"),
                size: 12,
            }],
        };
        match verify_dir(&m, tmp.path()) {
            Verdict::Mismatch(ms) => {
                assert_eq!(ms.len(), 1);
                assert_eq!(ms[0].path, "temper");
                assert!(ms[0].actual.is_some(), "a present-but-wrong file has an actual hash");
            }
            other => panic!("expected Mismatch, got {other:?}"),
        }
    }

    /// A file the manifest lists but the directory lacks is a Mismatch with no
    /// actual hash — distinct from a present-but-wrong file.
    #[test]
    fn missing_file_is_a_mismatch_with_no_actual() {
        let tmp = tempfile::tempdir().unwrap();
        let m = ReleaseManifest {
            version: "0.3.0".into(),
            target: "x86_64-unknown-linux-gnu".into(),
            files: vec![ManifestEntry {
                path: "temper".into(),
                sha256: sha_of(b"binary-bytes"),
                size: 12,
            }],
        };
        match verify_dir(&m, tmp.path()) {
            Verdict::Mismatch(ms) => assert!(ms[0].actual.is_none()),
            other => panic!("expected Mismatch, got {other:?}"),
        }
    }

    /// No manifest in the directory (the `cargo install` shape) yields None,
    /// which callers must render as Unverifiable — never Mismatch.
    #[test]
    fn absent_manifest_loads_as_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load_from_dir(tmp.path()).is_none());
    }

    /// The golden fixture is the wire contract shared with the workflow bash
    /// that produces it (Task 3). If this fails, producer and consumer drifted.
    #[test]
    fn golden_fixture_round_trips() {
        let raw = include_str!("../tests/fixtures/manifest-golden.json");
        let m: ReleaseManifest = serde_json::from_str(raw).expect("golden fixture parses");
        assert_eq!(m.target, "x86_64-unknown-linux-gnu");
        assert!(m.files.iter().any(|f| f.path == "temper"));
        assert!(m.files.iter().all(|f| f.sha256.len() == 64));
    }
}
```

- [ ] **Step 2: Create the golden fixture**

Create `crates/temper-cli/tests/fixtures/manifest-golden.json`. **This exact shape is what Task 3's bash must emit** — the two are one wire format:

```json
{
  "version": "0.3.0",
  "target": "x86_64-unknown-linux-gnu",
  "files": [
    {
      "path": "temper",
      "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      "size": 0
    },
    {
      "path": "lib/libonnxruntime.so",
      "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      "size": 0
    }
  ]
}
```

- [ ] **Step 3: Run the tests to verify they fail**

```bash
cargo nextest run -p temper-cli --lib manifest
```

Expected: FAIL — `cannot find type ReleaseManifest in this scope`.

- [ ] **Step 4: Implement the module**

Write the implementation above the test module in `crates/temper-cli/src/manifest.rs`:

```rust
//! The release manifest: per-file sha256 for every artifact shipped in a
//! release archive, and the verdict of checking an install directory against it.
//!
//! # Why per-file, and not just the archive
//!
//! The release's `.sha256` sidecar is computed over the whole archive
//! (`build-cli-binaries.yml:177,189`), so it measures a different object than
//! the installed binary and cannot answer "is my temper the one you shipped?".
//! This manifest closes that gap.
//!
//! # What a verdict does and does not mean
//!
//! Checking an install dir against a manifest *in that same dir* detects
//! corruption and drift — NOT an active attacker, who could replace both. Only
//! attestation-backed verification (`temper update`, `--verify --online`)
//! carries provenance weight. Callers must not render `Verified` as more than
//! it earned.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The manifest as written into an install directory by `install.sh`.
pub const MANIFEST_FILENAME: &str = ".temper-manifest.json";

/// Per-file hashes for one release archive. Produced by the release workflow
/// and consumed here; `manifest-golden.json` pins the wire shape so the bash
/// producer and this consumer cannot drift.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub version: String,
    pub target: String,
    pub files: Vec<ManifestEntry>,
}

/// One shipped file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

/// A file that failed verification. `actual` is `None` when the file is absent
/// entirely, which is a different failure from present-but-wrong.
#[derive(Debug, Clone, Serialize)]
pub struct Mismatch {
    pub path: String,
    pub expected: String,
    pub actual: Option<String>,
}

/// The outcome of checking an install dir. `Unverifiable` and `Mismatch` are
/// deliberately distinct: "we cannot tell" is not "it is wrong".
#[derive(Debug, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    Verified,
    Mismatch(Vec<Mismatch>),
    Unverifiable(String),
}

/// Load the manifest an install dir carries, if any. `None` means this is not
/// a manifest-bearing install (e.g. a `cargo install` build) — callers render
/// that as [`Verdict::Unverifiable`], never as a mismatch.
pub fn load_from_dir(dir: &Path) -> Option<ReleaseManifest> {
    let raw = std::fs::read_to_string(dir.join(MANIFEST_FILENAME)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Hash every file the manifest lists and compare. Files present in `dir` but
/// absent from the manifest are ignored — the manifest states what we shipped,
/// not what the user may not add beside it.
pub fn verify_dir(manifest: &ReleaseManifest, dir: &Path) -> Verdict {
    let mut mismatches = Vec::new();
    for entry in &manifest.files {
        match std::fs::read(dir.join(&entry.path)) {
            Ok(bytes) => {
                let actual = format!("{:x}", Sha256::digest(&bytes));
                if actual != entry.sha256 {
                    mismatches.push(Mismatch {
                        path: entry.path.clone(),
                        expected: entry.sha256.clone(),
                        actual: Some(actual),
                    });
                }
            }
            Err(_) => mismatches.push(Mismatch {
                path: entry.path.clone(),
                expected: entry.sha256.clone(),
                actual: None,
            }),
        }
    }
    if mismatches.is_empty() {
        Verdict::Verified
    } else {
        Verdict::Mismatch(mismatches)
    }
}
```

- [ ] **Step 5: Register the module**

In `crates/temper-cli/src/lib.rs`, add alongside the existing `pub mod` declarations:

```rust
pub mod manifest;
```

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cargo nextest run -p temper-cli --lib manifest
cargo make check
```

Expected: PASS, clippy clean.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/manifest.rs crates/temper-cli/src/lib.rs crates/temper-cli/tests/fixtures/manifest-golden.json
git commit -m "feat(cli): ReleaseManifest type and install-dir verification verdicts"
```

---

## Task 3: Generate and publish the per-file manifest

**Files:**
- Modify: `.github/workflows/build-cli-binaries.yml:108-201`
- Create: `.github/scripts/release/emit-manifest.sh`
- Create: `.github/scripts/test-emit-manifest.sh`
- Modify: `.github/workflows/code-quality.yml:143-173` (wire the harness into `guard-tests`)
- Modify: `.github/scripts/release/create-github-release.sh:30-35`

**Spec sections to read:** "What the release publishes" (including the **Rejected: self-description** note)

**Interfaces:**
- Consumes: the JSON shape pinned by `crates/temper-cli/tests/fixtures/manifest-golden.json` (Task 2).
- Produces: `temper-v{ver}-{triple}.manifest.json` as a release asset.

**CONFORM — do not generate the manifest by running the built binary.** All three matrix runners are native, so it *would* work, and it is rejected anyway: a compromised artifact would describe itself. See spec §1 "Rejected: self-description".

**CONFORM — dual-tool hashing.** `install.sh:128-134` and `build-cli-binaries.yml:177,189` both branch on macOS `shasum -a 256` vs Linux `sha256sum`. The script must too.

- [ ] **Step 1: Write the failing guard harness**

Create `.github/scripts/test-emit-manifest.sh`. It feeds the generator a known directory and asserts the emitted JSON matches the golden shape:

```bash
#!/usr/bin/env bash
# Harness for emit-manifest.sh. Proves the generator emits the wire shape that
# crates/temper-cli/src/manifest.rs parses — the two are one contract.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EMIT="${SCRIPT_DIR}/release/emit-manifest.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# A staging dir shaped like a real unix release archive.
mkdir -p "$TMP/staging/lib" "$TMP/staging/models"
printf 'binary' > "$TMP/staging/temper"
printf 'ort'    > "$TMP/staging/lib/libonnxruntime.so"
printf 'model'  > "$TMP/staging/models/model_quantized.onnx"
printf 'lic'    > "$TMP/staging/LICENSE"

OUT="$TMP/out.json"
VERSION=0.3.0 TARGET=x86_64-unknown-linux-gnu STAGING="$TMP/staging" OUTPUT="$OUT" \
  bash "$EMIT"

[ -s "$OUT" ] || fail "no manifest emitted"

# 1. Valid JSON with the expected top-level keys.
jq -e '.version == "0.3.0"' "$OUT" >/dev/null || fail "version key wrong"
jq -e '.target == "x86_64-unknown-linux-gnu"' "$OUT" >/dev/null || fail "target key wrong"

# 2. Every shipped file is listed, with a paths-are-relative invariant.
for p in temper lib/libonnxruntime.so models/model_quantized.onnx LICENSE; do
  jq -e --arg p "$p" 'any(.files[]; .path == $p)' "$OUT" >/dev/null \
    || fail "missing entry for $p"
done

# 3. Hashes are real sha256 of the real bytes — the bite. A generator that
#    emitted a constant, or hashed the wrong file, fails here.
EXPECTED=$(printf 'binary' | { sha256sum 2>/dev/null || shasum -a 256; } | awk '{print $1}')
ACTUAL=$(jq -r '.files[] | select(.path == "temper") | .sha256' "$OUT")
[ "$EXPECTED" = "$ACTUAL" ] || fail "sha256 for temper is $ACTUAL, expected $EXPECTED"

# 4. Sizes are real.
SIZE=$(jq -r '.files[] | select(.path == "temper") | .size' "$OUT")
[ "$SIZE" = "6" ] || fail "size for temper is $SIZE, expected 6"

# 5. No absolute paths or staging-dir leakage.
jq -e 'all(.files[]; (.path | startswith("/")) | not)' "$OUT" >/dev/null \
  || fail "manifest contains absolute paths"

echo "PASS: emit-manifest emits the golden wire shape over real bytes"
```

- [ ] **Step 2: Run the harness to verify it fails**

```bash
bash .github/scripts/test-emit-manifest.sh
```

Expected: FAIL — `emit-manifest.sh` does not exist.

- [ ] **Step 3: Implement the generator**

Create `.github/scripts/release/emit-manifest.sh`:

```bash
#!/usr/bin/env bash
# Emit the per-file release manifest for one target's staging directory.
#
# The manifest is generated HERE, from the staging tree, rather than by running
# the freshly-built binary. All three matrix runners are native so the artifact
# COULD describe itself — and that is exactly why it must not: a compromised
# artifact would faithfully attest to its own compromise.
#
# Required env:
#   VERSION  — e.g. 0.3.0 (no leading v)
#   TARGET   — target triple
#   STAGING  — the assembled staging dir
#   OUTPUT   — path to write the manifest JSON to
set -euo pipefail

: "${VERSION:?VERSION required}"
: "${TARGET:?TARGET required}"
: "${STAGING:?STAGING required}"
: "${OUTPUT:?OUTPUT required}"

# macOS ships shasum, Linux ships sha256sum. Same branch as install.sh:128-134
# and build-cli-binaries.yml:177,189 — never assume one.
sha_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

size_of() {
    wc -c < "$1" | tr -d ' '
}

ENTRIES=""
# -print0/read -d '' so paths with spaces survive. Sorted for a stable manifest.
while IFS= read -r -d '' f; do
    REL="${f#"$STAGING"/}"
    SHA=$(sha_of "$f")
    SIZE=$(size_of "$f")
    ENTRY=$(jq -n --arg path "$REL" --arg sha256 "$SHA" --argjson size "$SIZE" \
        '{path: $path, sha256: $sha256, size: $size}')
    ENTRIES="${ENTRIES}${ENTRY}"
done < <(find "$STAGING" -type f -print0 | sort -z)

printf '%s' "$ENTRIES" | jq -s \
    --arg version "$VERSION" \
    --arg target "$TARGET" \
    '{version: $version, target: $target, files: .}' > "$OUTPUT"

echo "Wrote manifest: $OUTPUT"
cat "$OUTPUT"
```

- [ ] **Step 4: Run the harness to verify it passes**

```bash
bash .github/scripts/test-emit-manifest.sh
```

Expected: `PASS: emit-manifest emits the golden wire shape over real bytes`

- [ ] **Step 5: Wire the generator into the build workflow**

In `.github/workflows/build-cli-binaries.yml`, add a step **after** "Assemble archive contents" (which ends at `:166`) and **before** "Create archive":

```yaml
      - name: Emit per-file manifest
        shell: bash
        env:
          VERSION: ${{ inputs.version }}
          TARGET: ${{ matrix.target.triple }}
          STAGING: staging
          OUTPUT: temper-v${{ inputs.version }}-${{ matrix.target.triple }}.manifest.json
        run: bash .github/scripts/release/emit-manifest.sh
```

**Ordering constraint:** the manifest must be emitted after the staging tree is complete (`:166`) and before archiving (`:168`), so it describes exactly the bytes that get archived.

- [ ] **Step 6: Add the manifest to the uploaded artifacts**

In the same file, extend the "Upload artifact" `path` list (`:196-199`):

```yaml
          path: |
            temper-v${{ inputs.version }}-*.tar.gz
            temper-v${{ inputs.version }}-*.zip
            temper-v${{ inputs.version }}-*.sha256
            temper-v${{ inputs.version }}-*.manifest.json
```

- [ ] **Step 7: Upload the manifest to the GitHub Release**

In `.github/scripts/release/create-github-release.sh`, extend the glob loop at `:30-35`:

```bash
for f in "${ARTIFACT_DIR}"/temper-*.tar.gz \
         "${ARTIFACT_DIR}"/temper-*.zip \
         "${ARTIFACT_DIR}"/temper-*.sha256 \
         "${ARTIFACT_DIR}"/temper-*.manifest.json; do
```

- [ ] **Step 8: Wire the harness into `guard-tests`**

`code-quality.yml:142` — "Adding a guard means adding its harness HERE, in the same PR." Add to the `guard-tests` job's steps (after `:173`):

```yaml
      - name: Guard test — emit-manifest
        run: bash .github/scripts/test-emit-manifest.sh
```

- [ ] **Step 9: Commit**

```bash
git add .github/scripts/release/emit-manifest.sh .github/scripts/test-emit-manifest.sh \
        .github/workflows/build-cli-binaries.yml .github/workflows/code-quality.yml \
        .github/scripts/release/create-github-release.sh
git commit -m "feat(release): emit and publish a per-file manifest per target"
```

---

## Task 4: Publish a build-provenance attestation

**Files:**
- Modify: `.github/workflows/build-cli-binaries.yml` (job `permissions` + a new step)
- Modify: `.github/workflows/release.yml:44-51` (pass through the added permission)

**Spec sections to read:** "What the release publishes", "Trust chain"

**Interfaces:**
- Produces: a signed attestation covering **both** the archive and the manifest digests, verifiable via `gh attestation verify` and (pending Task 1) natively in Task 9.

**EXTEND — authorized by spec §1.** Nothing on disk does this today.

- [ ] **Step 1: Grant the job the required permissions**

`actions/attest-build-provenance` needs `id-token: write` (to get the OIDC token Fulcio signs against) and `attestations: write`. Add to the `build` job in `build-cli-binaries.yml` (after `timeout-minutes: 30`, `:24`):

```yaml
    permissions:
      contents: read
      id-token: write
      attestations: write
```

- [ ] **Step 2: Propagate the permission from the calling workflow**

`release.yml:46-48` currently grants only `contents: read` to the reusable-workflow call. A called workflow cannot hold more permission than its caller grants, so widen it:

```yaml
  build-cli-binaries:
    name: Build CLI Binaries
    needs: determine-version
    permissions:
      contents: read
      id-token: write
      attestations: write
    uses: ./.github/workflows/build-cli-binaries.yml
    with:
      version: ${{ needs.determine-version.outputs.version }}
```

- [ ] **Step 3: Attest both the archive and the manifest**

Add to `build-cli-binaries.yml` **after** the archive-creation steps (`:190`) and **before** "Upload artifact" (`:192`):

```yaml
      - name: Attest build provenance
        uses: actions/attest-build-provenance@v2
        with:
          subject-path: |
            temper-v${{ inputs.version }}-${{ matrix.target.triple }}.tar.gz
            temper-v${{ inputs.version }}-${{ matrix.target.triple }}.zip
            temper-v${{ inputs.version }}-${{ matrix.target.triple }}.manifest.json
```

**Load-bearing (spec §1):** the manifest must be a subject alongside the archive. If only the archive were attested, the manifest could be swapped independently of the artifact it describes, and the whole chain would have a hole in exactly the place this design exists to close.

`subject-path` tolerates non-matching globs across the matrix (a unix target has no `.zip`); confirm this in the first release dry-run and pin per-target paths if it errors.

- [ ] **Step 4: Verify with a dispatch run**

`build-cli-binaries.yml` supports `workflow_dispatch` (`:10-15`). Trigger it and confirm the attestation is created:

```bash
gh workflow run build-cli-binaries.yml -f version=dev
gh run watch
```

Then, on a real tag later, verify end-to-end:

```bash
gh attestation verify temper-v0.3.0-x86_64-unknown-linux-gnu.tar.gz --repo tasker-systems/temper
```

Expected: verification succeeds and names the `build-cli-binaries.yml` workflow as the signer identity.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/build-cli-binaries.yml .github/workflows/release.yml
git commit -m "feat(release): attest build provenance over archive and manifest"
```

---

## Task 5: `install.sh --archive <path>` (skip download only)

Isolated and independently testable. Task 9 depends on it; doing it first keeps that task small.

**Files:**
- Modify: `scripts/install/install.sh:24-39` (arg parsing), `:100-124` (download)
- Create: `scripts/install/test-install.sh`
- Modify: `.github/workflows/code-quality.yml` (`guard-tests`)

**Spec sections to read:** "`temper update` and the 'one installer, one truth' tension"

**Interfaces:**
- Produces: `install.sh --archive <path>` — uses the supplied archive verbatim and performs no archive download. Consumed by Task 9.

**AMEND — authorized by spec §4.** This changes a load-bearing script. The reason it is an amendment and not a rewrite: verification is *policy* (Rust's job), installation is *mechanism* (the script's job), per `update.rs:15-19`. `--archive` moves only the download, leaving extract/verify/swap/rollback single-sourced.

- [ ] **Step 1: Write the failing harness**

Create `scripts/install/test-install.sh`:

```bash
#!/usr/bin/env bash
# Harness for install.sh. Feeds it a locally-built archive so the download path
# is bypassed, and asserts the flags and gates behave.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL="${SCRIPT_DIR}/install.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# Build a fake release archive whose "binary" is a script that prints a version,
# so install.sh's run-gate (install.sh:171) can pass without a real build.
build_archive() {
    STAGE="$TMP/stage"; rm -rf "$STAGE"; mkdir -p "$STAGE/lib"
    printf '#!/bin/sh\necho "0.3.0"\n' > "$STAGE/temper"
    chmod +x "$STAGE/temper"
    printf 'ort' > "$STAGE/lib/libonnxruntime.so"
    tar -czf "$TMP/archive.tar.gz" -C "$STAGE" .
}

build_archive

# --archive uses the supplied file and never reaches the network. Unsetting the
# network is proven by pointing the repo at an unroutable host: if install.sh
# tried to download, it would fail.
INSTALL_DIR="$TMP/install"
TEMPER_INSTALL_DIR="$INSTALL_DIR" XDG_BIN_HOME="$TMP/bin" \
    sh "$INSTALL" --archive "$TMP/archive.tar.gz" --version v0.3.0 >/dev/null 2>&1 \
    || fail "--archive install failed"

[ -x "$INSTALL_DIR/temper" ] || fail "binary not installed"
[ -f "$INSTALL_DIR/lib/libonnxruntime.so" ] || fail "ort lib not installed"

echo "PASS: install.sh --archive installs from a local archive without downloading"
```

- [ ] **Step 2: Run the harness to verify it fails**

```bash
bash scripts/install/test-install.sh
```

Expected: FAIL — `unknown argument: --archive` (`install.sh:37`).

- [ ] **Step 3: Add the flag to arg parsing**

In `install.sh`, extend the loop at `:24-39`. Add `LOCAL_ARCHIVE=""` beside `REQUESTED_VERSION=""` at `:22`, then:

```sh
        --archive) LOCAL_ARCHIVE="$2"; shift 2 ;;
        --archive=*) LOCAL_ARCHIVE="${1#*=}"; shift ;;
```

And extend the `--help` text at `:30-34`:

```sh
  --archive PATH      Install from an already-downloaded, already-verified
                      archive instead of downloading one. Used by
                      `temper update`, which verifies provenance itself before
                      handing the archive over.
```

- [ ] **Step 4: Branch the download**

Replace the download block at `:108-124` so it is skipped when `--archive` is supplied. The checksum-verify block at `:126-134` is skipped too — the caller already verified, and re-verifying against a sidecar we did not download would prove nothing:

```sh
if [ -n "$LOCAL_ARCHIVE" ]; then
    # Handed a pre-verified archive by `temper update`, which has already
    # checked provenance (attestation) and integrity (manifest). Re-downloading
    # would reopen exactly the TOCTOU gap that flag exists to close: verify one
    # object, install another.
    [ -f "$LOCAL_ARCHIVE" ] || { echo "error: --archive file not found: $LOCAL_ARCHIVE" >&2; exit 1; }
    cp "$LOCAL_ARCHIVE" "$TMPDIR/$ARCHIVE"
    echo "  Using pre-verified archive: ${LOCAL_ARCHIVE}"
else
    echo "  Downloading ${ARCHIVE}..."
    # ... existing curl invocations from :120-124, unchanged ...

    echo "  Verifying checksum..."
    # ... existing dual-tool verify from :127-134, unchanged ...
fi
```

**⚠️ Both-ends collision:** the literal string `Verifying checksum` must survive — `update.rs:603` asserts on it. Keep the echo inside the `else` branch exactly as written.

- [ ] **Step 5: Run the harness to verify it passes**

```bash
bash scripts/install/test-install.sh
cargo nextest run -p temper-cli --lib update
```

Expected: harness PASS, and `embedded_installer_is_the_real_script` still passes.

- [ ] **Step 6: Wire the harness into `guard-tests`**

```yaml
      - name: Guard test — install.sh
        run: bash scripts/install/test-install.sh
```

- [ ] **Step 7: Commit**

```bash
git add scripts/install/install.sh scripts/install/test-install.sh .github/workflows/code-quality.yml
git commit -m "feat(install): --archive flag to install from a pre-verified archive"
```

---

## Task 6: `install.sh` mandatory manifest verification and widened rollback gate

**Files:**
- Modify: `scripts/install/install.sh` (download block, post-extract, `:209-220` post-swap gate)
- Modify: `scripts/install/test-install.sh`

**Spec sections to read:** "Failure posture — reuse the rollback that already exists", "Trust chain"

**AMEND — authorized by spec §6.**

**CONFORM — do not write a new rollback path.** `install.sh:209-220` already moves the old install aside, re-points the symlink, and rolls back if the post-install check fails. The gate *widens*; the machinery does not change. Adding a second rollback path is the failure mode this step exists to avoid.

- [ ] **Step 1: Add the failing assertions to the harness**

Append to `scripts/install/test-install.sh`, before the final `echo`:

```bash
# --- A tampered file must roll back, not install ------------------------------
# The bite: alter one byte of the binary AFTER the manifest is written, and the
# post-swap gate must reject the install and leave the PRIOR install in place.
build_manifest() {
    STAGE="$1"; OUT="$2"
    VERSION=0.3.0 TARGET=x86_64-unknown-linux-gnu STAGING="$STAGE" OUTPUT="$OUT" \
        bash "$(cd "$SCRIPT_DIR/../../.github/scripts/release" && pwd)/emit-manifest.sh" >/dev/null
}

# 1. Establish a known-good prior install.
GOOD_DIR="$TMP/install2"
build_archive
build_manifest "$TMP/stage" "$TMP/good.manifest.json"
TEMPER_INSTALL_DIR="$GOOD_DIR" XDG_BIN_HOME="$TMP/bin2" \
    sh "$INSTALL" --archive "$TMP/archive.tar.gz" --manifest "$TMP/good.manifest.json" \
       --version v0.3.0 >/dev/null 2>&1 || fail "baseline install failed"
BASELINE=$(cat "$GOOD_DIR/temper")

# 2. Build a TAMPERED archive whose manifest no longer describes it.
rm -rf "$TMP/stage"; mkdir -p "$TMP/stage/lib"
printf '#!/bin/sh\necho "0.3.0"\n' > "$TMP/stage/temper"; chmod +x "$TMP/stage/temper"
printf 'ort' > "$TMP/stage/lib/libonnxruntime.so"
build_manifest "$TMP/stage" "$TMP/tampered.manifest.json"
printf '#!/bin/sh\necho "0.3.0"\n# EVIL\n' > "$TMP/stage/temper"; chmod +x "$TMP/stage/temper"
tar -czf "$TMP/tampered.tar.gz" -C "$TMP/stage" .

# 3. Installing it must FAIL and leave the baseline intact.
if TEMPER_INSTALL_DIR="$GOOD_DIR" XDG_BIN_HOME="$TMP/bin2" \
     sh "$INSTALL" --archive "$TMP/tampered.tar.gz" --manifest "$TMP/tampered.manifest.json" \
        --version v0.3.0 >/dev/null 2>&1; then
    fail "tampered archive installed successfully — the manifest gate does not bite"
fi
[ "$(cat "$GOOD_DIR/temper")" = "$BASELINE" ] \
    || fail "prior install was clobbered by a rejected update"

echo "PASS: a manifest mismatch is rejected and the prior install survives"
```

- [ ] **Step 2: Run the harness to verify it fails**

```bash
bash scripts/install/test-install.sh
```

Expected: FAIL — `unknown argument: --manifest`.

- [ ] **Step 3: Add `--manifest` and fetch the published manifest by default**

Add `LOCAL_MANIFEST=""` at `:22` and to the arg loop:

```sh
        --manifest) LOCAL_MANIFEST="$2"; shift 2 ;;
        --manifest=*) LOCAL_MANIFEST="${1#*=}"; shift ;;
```

In the download branch (Task 5, Step 4), fetch the manifest beside the archive. Add near the `SHA_URL` definition at `:103`:

```sh
MANIFEST="temper-${VERSION}-${TARGET}.manifest.json"
MANIFEST_URL="${URL_BASE}/${MANIFEST}"
```

and in the download `else` branch, after the sidecar fetch:

```sh
    curl -fsSL --connect-timeout 10 --max-time 60 --retry 2 --retry-connrefused \
        "$MANIFEST_URL" -o "$TMPDIR/$MANIFEST"
```

When `--manifest` is supplied, copy it to `$TMPDIR/$MANIFEST` instead.

- [ ] **Step 4: Verify every extracted file against the manifest**

Insert after the extract at `:160` and **before** the run-gate at `:169`. Failing here means nothing has been touched yet:

```sh
# --- Manifest verification ---------------------------------------------------
# Per-file integrity, in addition to the archive-level sidecar. The sidecar
# proves the archive arrived intact; this proves each FILE is the one published,
# which is what makes "is my temper the one you shipped?" answerable at all.
echo "  Verifying file manifest..."
sha_of_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}
MANIFEST_FAILED=0
while IFS='	' read -r REL EXPECTED_SHA; do
    [ -n "$REL" ] || continue
    if [ ! -f "$STAGING/$REL" ]; then
        echo "error: manifest lists $REL but it is missing from the archive" >&2
        MANIFEST_FAILED=1
        continue
    fi
    ACTUAL_SHA=$(sha_of_file "$STAGING/$REL")
    if [ "$ACTUAL_SHA" != "$EXPECTED_SHA" ]; then
        echo "error: $REL has sha256 $ACTUAL_SHA, expected $EXPECTED_SHA" >&2
        MANIFEST_FAILED=1
    fi
done <<EOF
$(jq -r '.files[] | "\(.path)\t\(.sha256)"' "$TMPDIR/$MANIFEST")
EOF
if [ "$MANIFEST_FAILED" -ne 0 ]; then
    echo "error: file manifest verification failed; your existing install was left untouched." >&2
    exit 1
fi
```

**Dependency note:** this introduces `jq`. `jq` is present on all GitHub runners and most dev machines, but **not guaranteed on a user's box**. Before implementing, decide and record: either (a) add a `command -v jq` preflight with a clear error, or (b) parse the manifest with `sed`/`awk` to keep the zero-dependency property `install.sh` has today. **Option (b) is preferred** — `install.sh` currently depends only on curl/tar/shasum, and a curl|sh installer that fails on a missing `jq` is a regression. Implement (b) unless the reviewer approves (a).

- [ ] **Step 5: Widen the post-swap gate**

At `:209`, the live check is currently `--version` only. Widen it so a mismatch rolls back through the machinery already there:

```sh
ln -sf "$INSTALL_DIR/temper" "$BIN_DIR/temper"
if "$INSTALL_DIR/temper" --version >/dev/null 2>&1 && verify_installed_manifest; then
    cp "$TMPDIR/$MANIFEST" "$INSTALL_DIR/.temper-manifest.json"
    rm -rf "$OLD"
else
    echo "error: the installed binary failed its post-install check; rolling back..." >&2
    # ... existing rollback from :213-219, UNCHANGED ...
fi
```

Define `verify_installed_manifest()` to re-run the Step 4 comparison against `$INSTALL_DIR`.

**The manifest is written into the install dir only on the success arm** — a rolled-back install must not leave a manifest describing an install that is no longer there. `MANIFEST_FILENAME` is `.temper-manifest.json`, matching `manifest.rs` (Task 2).

- [ ] **Step 6: Run the harness to verify it passes**

```bash
bash scripts/install/test-install.sh
cargo nextest run -p temper-cli --lib update
```

- [ ] **Step 7: Commit**

```bash
git add scripts/install/install.sh scripts/install/test-install.sh
git commit -m "feat(install): mandatory per-file manifest verification with rollback"
```

---

## Task 7: `temper version --verify` (offline verdicts)

**Files:**
- Modify: `crates/temper-cli/src/commands/version.rs`
- Modify: `crates/temper-cli/src/cli.rs:374-378`
- Modify: `crates/temper-cli/src/main.rs:1271-1273`

**Spec sections to read:** "CLI surfaces", "Honesty constraints"

**Interfaces:**
- Consumes: `manifest::{load_from_dir, verify_dir, Verdict}` (Task 2).
- Produces: `temper version --verify` rendering a `Verdict` through the existing `OutputFormat` machinery.

**CONFORM — `version.rs:32` `CHECKSUM_NOTE` is the established honesty pattern.** The offline verdict gets its own note in the same spirit; do not let `Verified` render as unqualified.

- [ ] **Step 1: Write the failing tests**

Add to `version.rs`'s test module:

```rust
/// A `cargo install` build has no manifest beside it. That must render as
/// Unverifiable — never Mismatch. "We cannot tell" is not "it is wrong", the
/// same distinction CARGO_REFUSAL draws at update.rs:58.
#[test]
fn absent_manifest_renders_unverifiable_not_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let report = build_verify_report(tmp.path());
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("unverifiable"), "got: {json}");
    assert!(!json.contains("mismatch"), "must not claim a mismatch: {json}");
}

/// The offline verdict must carry its limitation. A `Verified` that reads as
/// unqualified provenance overclaims: an actor who replaced the binary could
/// replace the manifest beside it.
#[test]
fn offline_verdict_carries_its_limitation() {
    assert!(OFFLINE_VERIFY_NOTE.contains("same directory"));
    assert!(
        OFFLINE_VERIFY_NOTE.contains("not") && OFFLINE_VERIFY_NOTE.contains("attacker"),
        "the note must disclaim adversarial meaning"
    );
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo nextest run -p temper-cli --lib version
```

Expected: FAIL — `cannot find function build_verify_report`.

- [ ] **Step 3: Implement**

Add to `version.rs`:

```rust
use crate::manifest::{self, Verdict};

/// Disclaimer carried in offline `--verify` output. Load-bearing for the same
/// reason as CHECKSUM_NOTE: it stops a `verified` verdict from being read as
/// provenance it did not earn.
const OFFLINE_VERIFY_NOTE: &str = "Offline verification compares the installed files against a \
    manifest in the same directory. It detects corruption and drift, but not an attacker, who \
    could replace both. Run `temper version --verify --online` for attestation-backed provenance.";

/// Offline verification report.
#[derive(Debug, Serialize)]
pub struct VerifyReport {
    version: &'static str,
    install_dir: String,
    #[serde(flatten)]
    verdict: Verdict,
    note: &'static str,
}

/// Build the offline verdict for an install directory. Absent manifest =>
/// Unverifiable, never Mismatch.
fn build_verify_report(dir: &std::path::Path) -> VerifyReport {
    let verdict = match manifest::load_from_dir(dir) {
        Some(m) => manifest::verify_dir(&m, dir),
        None => Verdict::Unverifiable(
            "no release manifest beside this binary — this is not a release install \
             (e.g. a `cargo install` build), so there is nothing to verify against"
                .to_string(),
        ),
    };
    VerifyReport {
        version: VERSION,
        install_dir: dir.display().to_string(),
        verdict,
        note: OFFLINE_VERIFY_NOTE,
    }
}
```

Extend `run` to take a `verify: bool` and dispatch to `build_verify_report` using the install dir resolved from `std::env::current_exe()` (mirroring `compute_self_checksum` at `version.rs:61-68`, which already canonicalizes the running binary).

- [ ] **Step 4: Wire the flag**

`cli.rs:374-378` — add to the `Version` variant:

```rust
        /// Verify the installed files against the release manifest beside them.
        #[arg(long)]
        verify: bool,
```

`main.rs:1271-1273` — thread it:

```rust
        Commands::Version { checksum, verify } => {
            temper_cli::commands::version::run(checksum, verify, output_format)
        }
```

- [ ] **Step 5: Run to verify they pass**

```bash
cargo nextest run -p temper-cli --lib version
cargo make check
```

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/version.rs crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs
git commit -m "feat(cli): temper version --verify with offline manifest verdicts"
```

---

## Task 8: `temper version --verify --online`

**Files:**
- Modify: `crates/temper-cli/src/commands/version.rs`
- Modify: `crates/temper-cli/src/cli.rs`

**Spec sections to read:** "CLI surfaces"

**Interfaces:**
- Consumes: `manifest::ReleaseManifest` (Task 2), the release-asset URL shape from `install.sh:100-103`.
- Produces: `temper version --verify --online`, which re-fetches the published manifest for the running version and compares.

**CONFORM — reuse the HTTP posture already established.** `update.rs:271-313` builds a `reqwest::Client` with a `temper-cli/{VERSION}` user-agent, 10s connect / 30s total timeouts, and maps a 403 to an explicit rate-limit message. Match that; do not invent a second HTTP configuration.

- [ ] **Step 1: Write the failing test**

```rust
/// The published-manifest URL must match the asset naming the release actually
/// uses (install.sh:100-103) — a mismatch here 404s at runtime and is invisible
/// to a unit test that constructs its own expectation.
#[test]
fn published_manifest_url_matches_release_asset_naming() {
    let url = published_manifest_url("0.3.0", "x86_64-unknown-linux-gnu");
    assert_eq!(
        url,
        "https://github.com/tasker-systems/temper/releases/download/v0.3.0/\
         temper-v0.3.0-x86_64-unknown-linux-gnu.manifest.json"
    );
}

/// `--online` without `--verify` is a usage error, not a silent no-op.
#[test]
fn online_requires_verify() {
    // Asserted via clap's `requires` attribute; see cli.rs.
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo nextest run -p temper-cli --lib version
```

- [ ] **Step 3: Implement the URL builder and the online fetch**

```rust
/// The published manifest asset URL. Mirrors install.sh:100-103's naming
/// exactly — `temper-v{ver}-{triple}.manifest.json` under the tag's download
/// path. Kept as one function so the naming has a single definition.
fn published_manifest_url(version: &str, target: &str) -> String {
    format!(
        "https://github.com/tasker-systems/temper/releases/download/v{version}/\
         temper-v{version}-{target}.manifest.json"
    )
}
```

Fetch it with the `update.rs:271-313` client posture, parse into `ReleaseManifest`, and compare against the install dir with `manifest::verify_dir`. Resolve the target triple from compile-time `std::env::consts::{ARCH, OS}` mapped to the three shipped triples; an unmapped host renders `Unverifiable`, not an error.

- [ ] **Step 4: Wire the flag with a clap `requires`**

```rust
        /// Re-fetch the published manifest (and attestation) instead of trusting
        /// the local copy. Requires --verify.
        #[arg(long, requires = "verify")]
        online: bool,
```

- [ ] **Step 5: Run to verify they pass**

```bash
cargo nextest run -p temper-cli --lib version
cargo make check
```

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/version.rs crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs
git commit -m "feat(cli): temper version --verify --online against the published manifest"
```

---

## Task 9: Native attestation verification in `temper update`

**⚠️ Gated on Task 1.** If Task 1 returned **BLOCKED**, implement only Steps 5–7 (the verified-archive handoff) and skip attestation verification entirely; record the omission in the docs (Task 10) rather than leaving it unstated.

**Files:**
- Modify: `crates/temper-cli/src/commands/update.rs`
- Create: `crates/temper-cli/src/attest.rs`
- Modify: `crates/temper-cli/Cargo.toml`

**Spec sections to read:** "The pinned trust root", "`temper update` and the 'one installer, one truth' tension", "The spike, and its BLOCKED arm"

**Interfaces:**
- Consumes: the crate + call shape decided by Task 1; `install.sh --archive` (Task 5); `manifest` (Task 2).
- Produces: `attest::verify_release(archive: &Path, manifest: &Path, tag: &str) -> Result<()>`.

- [ ] **Step 1: Add the dependency chosen by Task 1**

```bash
cargo add -p temper-cli sigstore-verification@<version-from-spike>
cargo tree -p temper-cli | rg -i "openssl|native-tls"
```

Expected: no `openssl` matches (Global Constraints).

- [ ] **Step 2: Write the failing tests**

```rust
/// An unknown/expired trust root must be distinguishable from a bad signature.
/// The recovery differs: one means "cut a release / re-run the installer", the
/// other means "this artifact is not ours". Collapsing them makes the error
/// unactionable.
#[test]
fn unknown_root_is_distinct_from_bad_signature() {
    let e = AttestError::UnknownTrustRoot;
    assert!(e.to_string().contains("trust root"));
    assert!(!e.to_string().contains("signature is invalid"));
}

/// The failure must name the recovery command. A dead-end error here strands
/// the user on an un-updatable install.
#[test]
fn unknown_root_error_is_actionable() {
    assert!(AttestError::UnknownTrustRoot.to_string().contains("install.sh"));
}
```

- [ ] **Step 3: Run to verify they fail**

```bash
cargo nextest run -p temper-cli --lib attest
```

- [ ] **Step 4: Implement `attest.rs` with the pinned root**

Pin the artifact Task 1 identified as a compile-time constant, following the `EXPECTED_MODEL_SHA256` doctrine (`embed.rs:313`). Verify the bundle's certificate identity is issuer = GitHub Actions OIDC and SAN = `https://github.com/tasker-systems/temper/.github/workflows/build-cli-binaries.yml@refs/tags/{tag}`.

**Never degrade to a warning on failure** (spec §3). An unknown trust root fails loudly and names the recovery: re-run `install.sh`, which is hash-verified.

- [ ] **Step 5: Restructure `run` to download-verify-then-hand-off**

`update.rs:130-209`'s current shape resolves a tag and hands it to the installer, which downloads. Change the order so the archive Rust verifies is the archive that gets installed:

1. Resolve the tag (unchanged, `:142-147`).
2. Download the archive + manifest + attestation bundle to a `tempfile::TempDir`.
3. `attest::verify_release(...)` — **mandatory**, no bypass flag.
4. Verify the archive against the manifest.
5. Call `run_installer` with the verified local archive path.

- [ ] **Step 6: Pass `--archive` and `--manifest` to the installer**

`update.rs:320-358`'s `run_installer` currently passes only `--version`. Extend it:

```rust
    let mut child = Command::new("sh")
        .arg("-s")
        .arg("--")
        .arg("--version")
        .arg(tag)
        .arg("--archive")
        .arg(archive_path)
        .arg("--manifest")
        .arg(manifest_path)
        .env("TEMPER_INSTALL_DIR", install_dir)
        // ... rest unchanged from :330-334 ...
```

**This is what closes the TOCTOU gap** (spec §4): the script no longer downloads, so it cannot install an object different from the one verified.

- [ ] **Step 7: Run the tests**

```bash
cargo nextest run -p temper-cli --lib
bash scripts/install/test-install.sh
cargo make check
```

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/attest.rs crates/temper-cli/src/commands/update.rs crates/temper-cli/Cargo.toml Cargo.lock
git commit -m "feat(cli): verify release attestation natively before handing the archive to the installer"
```

---

## Task 10: Documentation and the Windows declared hole

**Files:**
- Modify: `docs/guides/install.md`, `docs/guides/releasing.md`
- Modify: `crates/temper-cli/src/commands/update.rs:71` (`WINDOWS_REFUSAL`)
- Modify: `CLAUDE.md`

**Spec sections to read:** "Out of scope → Deferred", "The pinned trust root" (release obligation)

**CONFORM — the active goal "Surface parity — no door offers less than another without saying so"** (`019fa618-ce41-7762-97dd-179132503ea2`). A deferral that is not stated reads as coverage.

- [ ] **Step 1: Document the verification surfaces in `install.md`**

Cover: what the manifest is, the three verdicts, the difference between offline and `--online`, and the out-of-band audit command:

```bash
gh attestation verify temper-v0.3.0-aarch64-apple-darwin.tar.gz --repo tasker-systems/temper
```

- [ ] **Step 2: Record the root-rotation release obligation in `releasing.md`**

State it as a standing obligation, not a footnote: when sigstore rotates its trust root, cut a release promptly, because a pinned old root cannot verify a newer attestation and `temper update` will fail loudly for anyone whose installed version predates the rotation. Name the escape hatch (re-run `install.sh`, which is hash-verified).

- [ ] **Step 3: Declare the Windows hole**

Update `WINDOWS_REFUSAL` (`update.rs:71`) to state that Windows installs are hash-verified only, with no attestation-verified update path. Confirm `temper version --verify` reports `unverifiable` on Windows — never `verified`.

Add a matching note to `install.md`.

- [ ] **Step 4: Update `CLAUDE.md`**

Add a Key Patterns entry covering the manifest, the pinned trust root, the verdict trichotomy, and the `--archive` handoff that keeps "one installer, one truth" intact.

- [ ] **Step 5: Verify and commit**

```bash
cargo nextest run -p temper-cli --lib
cargo make check
git add docs/guides/install.md docs/guides/releasing.md crates/temper-cli/src/commands/update.rs CLAUDE.md
git commit -m "docs: verification surfaces, root-rotation obligation, Windows declared hole"
```

---

## Self-review

**Spec coverage:**

| Spec section | Task |
|---|---|
| What the release publishes — manifest | 3 |
| What the release publishes — attestation | 4 |
| Rejected: self-description | 3 (Step 3 doc comment) |
| Trust chain | 4, 6, 9 |
| The pinned trust root | 1, 9, 10 |
| Root-rotation release obligation | 10 |
| "One installer, one truth" tension | 5, 9 |
| CLI surfaces — verdicts, `--verify` | 7 |
| CLI surfaces — `--online` | 8 |
| CLI surfaces — cargo build ⇒ `unverifiable` | 2, 7 |
| Failure posture / rollback | 6 |
| Honesty constraint 1 (offline is not adversarial) | 2, 7 |
| Honesty constraint 2 (manifest ≠ model's primary guard) | 10 (docs), spec |
| Spike + BLOCKED arm | 1, 9 (gate) |
| Windows declared hole | 10 |
| Testing table | 2, 3, 5, 6, 7, 8, 9 |

**Type consistency:** `ReleaseManifest`, `ManifestEntry`, `Verdict`, `Mismatch`, `load_from_dir`, `verify_dir`, `MANIFEST_FILENAME` are defined in Task 2 and used unchanged in 7, 8, 9. The manifest filename `.temper-manifest.json` is consistent between `manifest.rs` (Task 2) and `install.sh` (Task 6). The JSON wire shape is pinned by one fixture consumed by Task 2's test and produced by Task 3's script.

**Known open item carried deliberately, not a placeholder:** Task 6 Step 4 requires an implementer decision between `jq` and `sed`/`awk` for manifest parsing in `install.sh`, with `sed`/`awk` stated as preferred and the reason given (preserving the installer's zero-dependency property). This is a judgment that needs the implementer to see the real parse, and it is bounded with a default.
