# Temper CLI Binary Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship prebuilt `temper` CLI binaries for macOS arm64, Linux x86_64, and Windows x86_64 as GitHub Release artifacts, with a `curl | sh` + `irm | iex` installer flow. No registry publishing.

**Architecture:** Adapts tasker-core's release pipeline (cargo-make tasks, version bumping, `release-prepare` PR flow) stripped of registry-publish logic. Each release archive bundles `temper[.exe]` + platform-matching `libonnxruntime.{dylib,so,dll}`; a new binary-adjacent fallback in `temper-ingest/src/embed.rs` finds the bundled lib at runtime.

**Tech Stack:** Bash (POSIX sh for installer), PowerShell 5.1+ (Windows installer), GitHub Actions, cargo-make, Rust (`std::env::current_exe`, `dirs`), `gh` CLI.

**Spec:** `docs/superpowers/specs/2026-04-17-temper-cli-binary-release-design.md`

---

## File Structure

**New files:**
- `VERSION` — single-scalar `0.0.1`
- `scripts/install/install.sh` — POSIX sh installer (macOS + Linux)
- `scripts/install/install.ps1` — PowerShell installer (Windows)
- `docs/guides/install.md` — user-facing install documentation
- `tools/cargo-make/release-tasks.toml` — cargo-make tasks (`release-prepare`, `release-check`)
- `tools/scripts/release/lib/common.sh` — shared bash helpers (adapted from tasker-core)
- `tools/scripts/release/read-version.sh` — read VERSION + cli Cargo.toml
- `tools/scripts/release/detect-changes.sh` — detect temper-cli + workspace-dep changes since last `v*` tag
- `tools/scripts/release/calculate-version.sh` — suggest next version
- `tools/scripts/release/update-version.sh` — write VERSION + Cargo.toml
- `tools/scripts/release/release-prepare.sh` — top-level orchestrator
- `.github/scripts/release/create-github-release.sh` — create GH Release
- `.github/scripts/release/generate-summary.sh` — print per-platform summary
- `.github/scripts/release/check-failures.sh` — detect any matrix failure
- `.github/workflows/build-cli-binaries.yml` — reusable 3-platform build
- `.github/workflows/release-tag.yml` — on VERSION change, push tag
- `.github/workflows/release.yml` — on tag push, build + release

**Modified files:**
- `crates/temper-ingest/src/embed.rs` — new binary-adjacent + XDG fallback steps
- `crates/temper-cli/Cargo.toml` — track VERSION via `update-version.sh` (initial state unchanged in this plan)
- `tools/cargo-make/main.toml` — extend `release-tasks.toml`
- `README.md` — rewrite install section, update elevator pitch

**Responsibility split:** `embed.rs` is the only Rust change — keeps runtime dep-resolution logic in one place. Release scripts are small and single-purpose (read/detect/calculate/update/prepare). Workflow YAML is split: `build-cli-binaries.yml` is a reusable subworkflow, `release.yml` orchestrates, `release-tag.yml` is standalone for the VERSION-file-change trigger.

---

## Task 1: `embed.rs` fallback chain — binary-adjacent + XDG paths

**Files:**
- Modify: `crates/temper-ingest/src/embed.rs:82-108` (the `dylib_path` search block inside `init_ort_runtime`)
- Test: `crates/temper-ingest/src/embed.rs:350-500` (new tests appended to existing `mod tests`)

Current search chain (file:`crates/temper-ingest/src/embed.rs:82-96`):

```rust
// Search order:
//   1. ORT_DYLIB_PATH env var (explicit override)
//   2. Homebrew ARM64: /opt/homebrew/lib/libonnxruntime.dylib
//   3. Homebrew Intel: /usr/local/lib/libonnxruntime.dylib
//   4. Linux system: /usr/lib/libonnxruntime.so
let dylib_path = std::env::var("ORT_DYLIB_PATH").ok().or_else(|| {
    [
        "/opt/homebrew/lib/libonnxruntime.dylib",
        "/usr/local/lib/libonnxruntime.dylib",
        "/usr/lib/libonnxruntime.so",
    ]
    .iter()
    .find(|p| std::path::Path::new(p).exists())
    .map(|p| p.to_string())
});
```

The new chain adds two steps after the env var and before the system paths: (2) binary-adjacent (`<exe_dir>/lib/libonnxruntime.*` + flat `<exe_dir>/onnxruntime.dll` for Windows) and (3) XDG data dir (`~/.local/share/temper/lib/libonnxruntime.*`).

- [ ] **Step 1: Write the failing tests**

Append to `crates/temper-ingest/src/embed.rs`, inside the existing `mod tests { ... }` block (around line 450, at the end):

```rust
    // -- Dylib discovery fallback chain --
    //
    // These tests exercise `resolve_dylib_from_candidates`, the pure function
    // extracted from the ORT init logic. They don't actually load ONNX — just
    // verify path selection against a constructed set of candidate paths.

    use std::fs;

    fn make_dummy_lib(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"not a real library").expect("write dummy lib");
        path
    }

    #[test]
    fn resolve_picks_first_existing_candidate() {
        let tmp = tempfile::tempdir().unwrap();
        let first = make_dummy_lib(tmp.path(), "first.dylib");
        let second = make_dummy_lib(tmp.path(), "second.dylib");

        let picked = super::resolve_dylib_from_candidates(&[
            first.clone(),
            second.clone(),
        ]);

        assert_eq!(picked, Some(first));
    }

    #[test]
    fn resolve_skips_missing_candidates() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist.dylib");
        let exists = make_dummy_lib(tmp.path(), "real.dylib");

        let picked = super::resolve_dylib_from_candidates(&[
            missing,
            exists.clone(),
        ]);

        assert_eq!(picked, Some(exists));
    }

    #[test]
    fn resolve_returns_none_when_all_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let candidates = vec![
            tmp.path().join("a.dylib"),
            tmp.path().join("b.dylib"),
        ];

        let picked = super::resolve_dylib_from_candidates(&candidates);

        assert_eq!(picked, None);
    }

    #[test]
    fn binary_adjacent_candidates_include_lib_subdir_and_flat() {
        // Given an exe at /opt/tool/bin/temper, candidates should include both
        // /opt/tool/bin/lib/libonnxruntime.{dylib,so} (installed-tree layout)
        // and /opt/tool/bin/onnxruntime.dll (Windows flat layout).
        let fake_exe = std::path::PathBuf::from("/opt/tool/bin/temper");
        let candidates = super::binary_adjacent_candidates(&fake_exe);

        let as_str: Vec<String> = candidates
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();

        assert!(as_str.iter().any(|s| s.ends_with("lib/libonnxruntime.dylib")),
            "missing lib/libonnxruntime.dylib: {as_str:?}");
        assert!(as_str.iter().any(|s| s.ends_with("lib/libonnxruntime.so")),
            "missing lib/libonnxruntime.so: {as_str:?}");
        assert!(as_str.iter().any(|s| s.ends_with("onnxruntime.dll")),
            "missing onnxruntime.dll: {as_str:?}");
    }

    #[test]
    fn xdg_data_candidates_point_at_temper_lib() {
        let candidates = super::xdg_data_candidates();
        let as_str: Vec<String> = candidates
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert!(as_str.iter().any(|s| s.contains("/temper/lib/")),
            "candidates should include ~/.local/share/temper/lib/: {as_str:?}");
    }
```

The first three tests target a pure helper `resolve_dylib_from_candidates(&[PathBuf])`. The last two target two more pure helpers: `binary_adjacent_candidates(&Path) -> Vec<PathBuf>` and `xdg_data_candidates() -> Vec<PathBuf>`. We'll implement all three helpers in Step 3.

Also add `tempfile` to `[dev-dependencies]` in `crates/temper-ingest/Cargo.toml` — grep to confirm first:

```bash
grep -A2 '^\[dev-dependencies\]' crates/temper-ingest/Cargo.toml
```

If `tempfile` isn't already listed, add:

```toml
[dev-dependencies]
tempfile = "3"
```

(Check the existing `Cargo.toml:22`-ish area — `tempfile` is already listed in many crates in this repo, so it likely is.)

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo nextest run -p temper-ingest --features embed-download resolve_picks_first_existing_candidate
```

Expected: FAIL with `no function or associated item named resolve_dylib_from_candidates` (or similar — module path error).

- [ ] **Step 3: Implement the three helpers + integrate into the search chain**

Replace lines 82-96 of `crates/temper-ingest/src/embed.rs` (the existing "Search order:" comment + `dylib_path` block) with:

```rust
            // Search order:
            //   1. ORT_DYLIB_PATH env var (explicit override)
            //   2. Binary-adjacent: <exe_dir>/lib/libonnxruntime.{dylib,so} OR
            //      <exe_dir>/onnxruntime.dll (installer-bundled layout)
            //   3. XDG data: ~/.local/share/temper/lib/libonnxruntime.{dylib,so}
            //   4. Homebrew ARM64: /opt/homebrew/lib/libonnxruntime.dylib
            //   5. Homebrew Intel: /usr/local/lib/libonnxruntime.dylib
            //   6. Linux system: /usr/lib/libonnxruntime.so
            let dylib_path = std::env::var("ORT_DYLIB_PATH")
                .ok()
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    std::env::current_exe()
                        .ok()
                        .and_then(|exe| resolve_dylib_from_candidates(&binary_adjacent_candidates(&exe)))
                })
                .or_else(|| resolve_dylib_from_candidates(&xdg_data_candidates()))
                .or_else(|| {
                    resolve_dylib_from_candidates(&[
                        std::path::PathBuf::from("/opt/homebrew/lib/libonnxruntime.dylib"),
                        std::path::PathBuf::from("/usr/local/lib/libonnxruntime.dylib"),
                        std::path::PathBuf::from("/usr/lib/libonnxruntime.so"),
                    ])
                })
                .map(|p| p.to_string_lossy().into_owned());
```

Then add these three module-private helpers just above `fn init_ort_runtime` (around line 49, before `static ORT_INIT:`):

```rust
/// Pick the first existing path from a candidate list. Extracted so
/// distribution-layout tests can exercise the selection logic without a real
/// ONNX Runtime installed.
fn resolve_dylib_from_candidates(candidates: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    candidates.iter().find(|p| p.exists()).cloned()
}

/// Candidate paths relative to the running executable. Covers the two archive
/// layouts the release installer produces:
///   - mac/linux:  <exe_dir>/lib/libonnxruntime.{dylib,so}
///   - windows:    <exe_dir>/onnxruntime.dll (flat)
fn binary_adjacent_candidates(exe_path: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Some(exe_dir) = exe_path.parent() else {
        return Vec::new();
    };
    vec![
        exe_dir.join("lib").join("libonnxruntime.dylib"),
        exe_dir.join("lib").join("libonnxruntime.so"),
        exe_dir.join("onnxruntime.dll"),
    ]
}

/// Candidate paths under the user's XDG data dir. Covers `temper` symlinked
/// onto PATH while the actual install lives in `~/.local/share/temper/`.
fn xdg_data_candidates() -> Vec<std::path::PathBuf> {
    let Some(data_dir) = dirs::data_local_dir() else {
        return Vec::new();
    };
    let lib_dir = data_dir.join("temper").join("lib");
    vec![
        lib_dir.join("libonnxruntime.dylib"),
        lib_dir.join("libonnxruntime.so"),
    ]
}
```

Add `dirs = "5"` to `[dependencies]` in `crates/temper-ingest/Cargo.toml` (if not present) — other crates in this workspace already depend on `dirs 5`, so align on that version. Grep first:

```bash
grep '^dirs' crates/temper-ingest/Cargo.toml
```

If absent, add under `[dependencies]`:

```toml
dirs = "5"
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo nextest run -p temper-ingest --features embed-download resolve_
cargo nextest run -p temper-ingest --features embed-download binary_adjacent_candidates
cargo nextest run -p temper-ingest --features embed-download xdg_data_candidates
```

Expected: all 5 new tests PASS.

- [ ] **Step 5: Run full temper-ingest test suite**

```bash
cargo nextest run -p temper-ingest --features embed-download
```

Expected: existing tests still pass, no regressions.

- [ ] **Step 6: Run `cargo make check`**

```bash
cargo make check
```

Expected: PASS. (Specifically: fmt is clean, clippy passes with `-D warnings`, machete finds no unused deps.)

- [ ] **Step 7: Commit**

```bash
git add crates/temper-ingest/Cargo.toml crates/temper-ingest/src/embed.rs
git commit -m "feat(ingest): bundled-layout fallback for libonnxruntime lookup

Adds binary-adjacent and XDG data dir to the dylib search chain so
installer-placed onnxruntime libs are discovered without ORT_DYLIB_PATH."
```

---

## Task 2: `VERSION` file + `tools/scripts/release/lib/common.sh`

**Files:**
- Create: `VERSION`
- Create: `tools/scripts/release/lib/common.sh`

- [ ] **Step 1: Create VERSION file**

```bash
mkdir -p tools/scripts/release/lib
echo "0.0.1" > VERSION
```

- [ ] **Step 2: Write `common.sh` (adapted from tasker-core, stripped of Ruby/Python/TS/FFI helpers)**

Create `tools/scripts/release/lib/common.sh`:

```bash
#!/usr/bin/env bash
# tools/scripts/release/lib/common.sh
# Shared functions for Temper release tooling.
#
# Source this from other release scripts:
#   source "$(dirname "$0")/lib/common.sh"
#
# Expects callers to set DRY_RUN=true|false before calling file-update functions.

set -euo pipefail

# Resolve repo root relative to this file (lib/ -> release/ -> scripts/ -> tools/ -> repo root)
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

# ---------------------------------------------------------------------------
# Logging
# ---------------------------------------------------------------------------
log_info()    { echo "  [info] $*"; }
log_warn()    { echo "  [warn] $*" >&2; }
log_error()   { echo "  [error] $*" >&2; }
log_header()  { echo ""; echo "== $* =="; echo ""; }
log_section() { echo ""; echo "-- $* --"; }

die() { log_error "$*"; exit 1; }

confirm() {
    read -p "  $1 (y/N) " -n 1 -r
    echo
    [[ $REPLY =~ ^[Yy]$ ]] || exit 1
}

# ---------------------------------------------------------------------------
# Portable sed -i (GNU vs BSD/macOS)
# ---------------------------------------------------------------------------
sed_i() {
    if sed --version 2>/dev/null | grep -q 'GNU'; then
        sed -i "$@"
    else
        sed -i '' "$@"
    fi
}

# ---------------------------------------------------------------------------
# Version arithmetic
# ---------------------------------------------------------------------------

# Bump the patch component: 0.1.8 -> 0.1.9
bump_patch() {
    local version="$1"
    local major minor patch
    IFS='.' read -r major minor patch <<< "$version"
    echo "${major}.${minor}.$((patch + 1))"
}

# Bump the minor component: 0.1.8 -> 0.2.0
bump_minor() {
    local version="$1"
    local major minor _patch
    IFS='.' read -r major minor _patch <<< "$version"
    echo "${major}.$((minor + 1)).0"
}

# Bump the major component: 0.1.8 -> 1.0.0
bump_major() {
    local version="$1"
    local major _minor _patch
    IFS='.' read -r major _minor _patch <<< "$version"
    echo "$((major + 1)).0.0"
}

# ---------------------------------------------------------------------------
# File update helpers
#
# All functions respect the DRY_RUN variable from the caller's scope.
# ---------------------------------------------------------------------------

update_version_file() {
    local version="$1"
    local file="${REPO_ROOT}/VERSION"
    if [[ "${DRY_RUN:-false}" == "true" ]]; then
        log_info "Would update VERSION -> $version"
    else
        echo "$version" > "$file"
        log_info "Updated VERSION -> $version"
    fi
}

# Update the top-level `version = "..."` in a Cargo.toml.
# Only touches the first occurrence (the [package] version).
update_cargo_version() {
    local file="$1" version="$2"

    # Resolve relative to repo root if not absolute
    [[ "$file" != /* ]] && file="${REPO_ROOT}/${file}"

    if [[ ! -f "$file" ]]; then
        log_warn "File not found: $file"
        return
    fi

    if [[ "${DRY_RUN:-false}" == "true" ]]; then
        local current
        current=$(grep -m1 '^version = ' "$file" | sed 's/version = "\(.*\)"/\1/')
        log_info "Would update $file version: $current -> $version"
    else
        local line_num
        line_num=$(grep -n -m1 '^version = ' "$file" | cut -d: -f1)
        if [[ -n "$line_num" ]]; then
            sed_i "${line_num}s/^version = \".*\"/version = \"${version}\"/" "$file"
        fi
        log_info "Updated $file -> $version"
    fi
}
```

- [ ] **Step 3: Smoke-test the helpers**

```bash
chmod +x tools/scripts/release/lib/common.sh
bash -c 'source tools/scripts/release/lib/common.sh && bump_patch 0.1.9'
```

Expected output: `0.1.10`

```bash
bash -c 'source tools/scripts/release/lib/common.sh && bump_minor 0.1.9'
```

Expected output: `0.2.0`

```bash
bash -c 'source tools/scripts/release/lib/common.sh && bump_major 1.9.3'
```

Expected output: `2.0.0`

- [ ] **Step 4: Commit**

```bash
git add VERSION tools/scripts/release/lib/common.sh
git commit -m "chore(release): add VERSION file + common bash helpers"
```

---

## Task 3: `tools/scripts/release/read-version.sh`

**Files:**
- Create: `tools/scripts/release/read-version.sh`

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
# tools/scripts/release/read-version.sh
#
# Read committed VERSION from the repo root.
#
# Output (suitable for `eval` and `>> $GITHUB_OUTPUT`):
#   VERSION=0.1.0

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"

VERSION_FILE="${REPO_ROOT}/VERSION"
if [[ ! -f "$VERSION_FILE" ]]; then
    echo "ERROR: VERSION file not found at ${VERSION_FILE}" >&2
    exit 1
fi

VERSION=$(tr -d '[:space:]' < "$VERSION_FILE")
echo "VERSION=${VERSION}"
```

- [ ] **Step 2: Smoke-test**

```bash
chmod +x tools/scripts/release/read-version.sh
./tools/scripts/release/read-version.sh
```

Expected output: `VERSION=0.0.1`

- [ ] **Step 3: Commit**

```bash
git add tools/scripts/release/read-version.sh
git commit -m "chore(release): add read-version.sh"
```

---

## Task 4: `tools/scripts/release/detect-changes.sh`

**Files:**
- Create: `tools/scripts/release/detect-changes.sh`

Detects whether `temper-cli` or any of its workspace deps (`temper-core`, `temper-client`, `temper-ingest`, `temper-llm`) changed since the last `v*` tag. If nothing changed → `CLI_CHANGED=false`, release-prepare exits cleanly.

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
# tools/scripts/release/detect-changes.sh
#
# Detect whether temper-cli or its workspace deps changed since the last
# `v*` tag.
#
# Usage:
#   ./tools/scripts/release/detect-changes.sh [--from TAG]
#
# Output (eval-safe KEY=VALUE):
#   CLI_CHANGED=true|false
#   CHANGES_BASE_REF=<tag|commit>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "${SCRIPT_DIR}/lib/common.sh"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
FROM_REF=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --from) FROM_REF="$2"; shift 2 ;;
        --from=*) FROM_REF="${1#*=}"; shift ;;
        *) die "Unknown argument: $1" ;;
    esac
done

# ---------------------------------------------------------------------------
# Determine base reference
# ---------------------------------------------------------------------------
if [[ -n "$FROM_REF" ]]; then
    BASE_REF="$FROM_REF"
elif BASE_REF=$(git describe --tags --match 'v*' --abbrev=0 HEAD 2>/dev/null); then
    : # Found a v* tag
else
    # No release tags exist yet — compare against the initial commit
    BASE_REF=$(git rev-list --max-parents=0 HEAD 2>/dev/null | head -n1)
fi

log_info "Comparing HEAD to ${BASE_REF}" >&2

# ---------------------------------------------------------------------------
# Get changed files
# ---------------------------------------------------------------------------
CHANGED_FILES=$(git diff "${BASE_REF}" HEAD --name-only 2>/dev/null || true)

if [[ -z "$CHANGED_FILES" ]]; then
    log_info "No files changed since ${BASE_REF}" >&2
fi

# ---------------------------------------------------------------------------
# Classify changes
# ---------------------------------------------------------------------------
changes_match() {
    local pattern="$1"
    grep -qE "$pattern" <<< "$CHANGED_FILES"
}

# temper-cli and its workspace deps — changes in any of these mean the CLI
# binary behavior may have changed and a release is warranted.
CLI_CHANGED=false
if changes_match '^crates/(temper-cli|temper-core|temper-client|temper-ingest|temper-llm)/'; then
    CLI_CHANGED=true
fi

# Release tooling and installer changes also warrant a release so users get
# fixes to the install flow.
if changes_match '^(scripts/install/|tools/scripts/release/|\.github/workflows/(release|build-cli-binaries|release-tag)\.yml|\.github/scripts/release/)'; then
    CLI_CHANGED=true
fi

# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------
echo "CHANGES_BASE_REF=${BASE_REF}"
echo "CLI_CHANGED=${CLI_CHANGED}"
```

- [ ] **Step 2: Smoke-test**

```bash
chmod +x tools/scripts/release/detect-changes.sh
./tools/scripts/release/detect-changes.sh
```

Expected output (roughly): `CHANGES_BASE_REF=<some-sha>` + `CLI_CHANGED=true` (since we're adding the release tooling itself on this branch).

- [ ] **Step 3: Commit**

```bash
git add tools/scripts/release/detect-changes.sh
git commit -m "chore(release): add detect-changes.sh for temper-cli deps"
```

---

## Task 5: `tools/scripts/release/calculate-version.sh`

**Files:**
- Create: `tools/scripts/release/calculate-version.sh`

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
# tools/scripts/release/calculate-version.sh
#
# Calculate the next temper-cli version.
#
# Usage:
#   ./tools/scripts/release/calculate-version.sh [--bump patch|minor|major] [--from TAG]
#
# Defaults to patch bump. Prompts interactively unless --bump is given.
#
# Reads: VERSION file, git tags, output from detect-changes.sh.
#
# Output (eval-safe KEY=VALUE):
#   CURRENT_VERSION=0.1.0
#   NEXT_VERSION=0.1.1
#   CLI_CHANGED=true|false
#   CHANGES_BASE_REF=<tag|commit>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "${SCRIPT_DIR}/lib/common.sh"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
BUMP=""
DETECT_ARGS=()

while [[ $# -gt 0 ]]; do
    case $1 in
        --bump) BUMP="$2"; shift 2 ;;
        --bump=*) BUMP="${1#*=}"; shift ;;
        --from) DETECT_ARGS+=(--from "$2"); shift 2 ;;
        --from=*) DETECT_ARGS+=(--from "${1#*=}"); shift ;;
        *) die "Unknown argument: $1" ;;
    esac
done

# ---------------------------------------------------------------------------
# Change detection
# ---------------------------------------------------------------------------
# shellcheck disable=SC2046
eval "$("${SCRIPT_DIR}/detect-changes.sh" ${DETECT_ARGS[@]+"${DETECT_ARGS[@]}"})"

# ---------------------------------------------------------------------------
# Read current version
# ---------------------------------------------------------------------------
eval "$("${SCRIPT_DIR}/read-version.sh")"
CURRENT_VERSION="$VERSION"
echo "CURRENT_VERSION=${CURRENT_VERSION}"

# ---------------------------------------------------------------------------
# Calculate next version
# ---------------------------------------------------------------------------
if [[ "$CLI_CHANGED" != "true" ]]; then
    NEXT_VERSION="$CURRENT_VERSION"
else
    case "$BUMP" in
        major) NEXT_VERSION=$(bump_major "$CURRENT_VERSION") ;;
        minor) NEXT_VERSION=$(bump_minor "$CURRENT_VERSION") ;;
        patch|"") NEXT_VERSION=$(bump_patch "$CURRENT_VERSION") ;;
        *) die "Unknown --bump level: $BUMP (expected patch|minor|major)" ;;
    esac
fi
echo "NEXT_VERSION=${NEXT_VERSION}"

# ---------------------------------------------------------------------------
# Re-emit detect-changes variables
# ---------------------------------------------------------------------------
echo "CHANGES_BASE_REF=${CHANGES_BASE_REF}"
echo "CLI_CHANGED=${CLI_CHANGED}"
```

- [ ] **Step 2: Smoke-test**

```bash
chmod +x tools/scripts/release/calculate-version.sh
./tools/scripts/release/calculate-version.sh --bump patch
```

Expected output:
```
CURRENT_VERSION=0.0.1
NEXT_VERSION=0.0.2
CHANGES_BASE_REF=<sha>
CLI_CHANGED=true
```

- [ ] **Step 3: Commit**

```bash
git add tools/scripts/release/calculate-version.sh
git commit -m "chore(release): add calculate-version.sh"
```

---

## Task 6: `tools/scripts/release/update-version.sh`

**Files:**
- Create: `tools/scripts/release/update-version.sh`

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
# tools/scripts/release/update-version.sh
#
# Update the VERSION file and temper-cli/Cargo.toml version.
#
# Usage:
#   ./tools/scripts/release/update-version.sh --version 0.1.0 [--dry-run]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "${SCRIPT_DIR}/lib/common.sh"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
VERSION=""
DRY_RUN=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --version)   VERSION="$2"; shift 2 ;;
        --version=*) VERSION="${1#*=}"; shift ;;
        --dry-run)   DRY_RUN=true; shift ;;
        *) die "Unknown argument: $1" ;;
    esac
done

if [[ -z "$VERSION" ]]; then
    die "Usage: $0 --version VERSION [--dry-run]"
fi

export DRY_RUN

log_section "Updating version to ${VERSION}"

update_version_file "$VERSION"
update_cargo_version "crates/temper-cli/Cargo.toml" "$VERSION"
```

- [ ] **Step 2: Smoke-test with dry-run**

```bash
chmod +x tools/scripts/release/update-version.sh
./tools/scripts/release/update-version.sh --version 9.9.9 --dry-run
```

Expected output:
```
-- Updating version to 9.9.9 --
  [info] Would update VERSION -> 9.9.9
  [info] Would update /Users/.../crates/temper-cli/Cargo.toml version: 0.1.0 -> 9.9.9
```

Verify no actual file changes:
```bash
cat VERSION
grep -m1 '^version = ' crates/temper-cli/Cargo.toml
```

Expected: `VERSION` still `0.0.1`, Cargo.toml version unchanged.

- [ ] **Step 3: Commit**

```bash
git add tools/scripts/release/update-version.sh
git commit -m "chore(release): add update-version.sh"
```

---

## Task 7: `tools/scripts/release/release-prepare.sh`

**Files:**
- Create: `tools/scripts/release/release-prepare.sh`

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
# tools/scripts/release/release-prepare.sh
#
# Prepare a release branch with a version bump, then open a PR to main.
#
# Usage:
#   ./tools/scripts/release/release-prepare.sh [--bump patch|minor|major] \
#       [--dry-run] [--yes] [--from TAG]
#
# Flow:
#   1. Pre-flight: clean tree, on main, up-to-date, gh available
#   2. Detect changes + calculate next version
#   3. Display summary, confirm
#   4. Create release/v<N.N.N> branch
#   5. Bump VERSION + temper-cli/Cargo.toml
#   6. cargo check as a sanity gate
#   7. Commit, push, open PR

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "${SCRIPT_DIR}/lib/common.sh"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
BUMP="patch"
DRY_RUN=false
YES=false
CALC_ARGS=()

while [[ $# -gt 0 ]]; do
    case $1 in
        --bump)    BUMP="$2"; CALC_ARGS+=(--bump "$2"); shift 2 ;;
        --bump=*)  BUMP="${1#*=}"; CALC_ARGS+=(--bump "${1#*=}"); shift ;;
        --dry-run) DRY_RUN=true; shift ;;
        --yes|-y)  YES=true; shift ;;
        --from)    CALC_ARGS+=(--from "$2"); shift 2 ;;
        --from=*)  CALC_ARGS+=(--from "${1#*=}"); shift ;;
        *) die "Unknown argument: $1" ;;
    esac
done

log_header "Temper Release Preparation"

# ---------------------------------------------------------------------------
# Pre-flight checks
# ---------------------------------------------------------------------------
log_section "Pre-flight checks"

if ! git diff-index --quiet HEAD -- 2>/dev/null; then
    if [[ "$DRY_RUN" == "true" ]]; then
        log_warn "Uncommitted changes detected (ignored in dry-run mode)"
    else
        die "Uncommitted changes detected. Commit or stash first."
    fi
else
    log_info "Working tree is clean"
fi

BRANCH=$(git branch --show-current)
if [[ "$BRANCH" != "main" ]]; then
    if [[ "$DRY_RUN" == "true" ]]; then
        log_warn "On branch '$BRANCH', not 'main' (ignored in dry-run mode)"
    else
        die "Must be on main branch (currently on '$BRANCH')"
    fi
else
    log_info "On main branch"
fi

git fetch origin --quiet
LOCAL_SHA=$(git rev-parse HEAD)
REMOTE_SHA=$(git rev-parse origin/main 2>/dev/null || echo "unknown")
if [[ "$LOCAL_SHA" != "$REMOTE_SHA" ]]; then
    if [[ "$DRY_RUN" == "true" ]]; then
        log_warn "Local branch is not up-to-date with origin/main (ignored in dry-run mode)"
    else
        die "Local main is not up-to-date with origin/main. Run: git pull"
    fi
else
    log_info "main is up-to-date with origin"
fi

if ! command -v gh &>/dev/null; then
    die "gh CLI not found. Install: https://cli.github.com/"
fi
log_info "gh CLI available"

# ---------------------------------------------------------------------------
# Change detection + version calculation
# ---------------------------------------------------------------------------
log_section "Detecting changes and calculating version"

# shellcheck disable=SC2046
eval "$("${SCRIPT_DIR}/calculate-version.sh" ${CALC_ARGS[@]+"${CALC_ARGS[@]}"})"

log_info "Base ref: ${CHANGES_BASE_REF}"
log_info "CLI changed: ${CLI_CHANGED}"

if [[ "$CLI_CHANGED" != "true" ]]; then
    log_warn "No changes to temper-cli or its deps since ${CHANGES_BASE_REF} — nothing to release"
    exit 0
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
log_section "Release Summary"

echo ""
echo "  Version: ${CURRENT_VERSION} -> ${NEXT_VERSION} (${BUMP})"
echo ""

# ---------------------------------------------------------------------------
# Dry-run: exit here
# ---------------------------------------------------------------------------
if [[ "$DRY_RUN" == "true" ]]; then
    log_info "DRY RUN -- no branch created, no PR opened"
    exit 0
fi

# ---------------------------------------------------------------------------
# Confirm
# ---------------------------------------------------------------------------
if [[ "$YES" != "true" ]]; then
    echo ""
    confirm "Create release branch and prepare PR?"
fi

# ---------------------------------------------------------------------------
# Create release branch
# ---------------------------------------------------------------------------
RELEASE_BRANCH="release/v${NEXT_VERSION}"
log_section "Creating branch: ${RELEASE_BRANCH}"
git checkout -b "$RELEASE_BRANCH"

# ---------------------------------------------------------------------------
# Bump version
# ---------------------------------------------------------------------------
log_section "Bumping version"
"${SCRIPT_DIR}/update-version.sh" --version "${NEXT_VERSION}"

# ---------------------------------------------------------------------------
# Sanity check: verify workspace compiles
# ---------------------------------------------------------------------------
log_section "Sanity check (cargo check)"
SQLX_OFFLINE=true cargo check --workspace

# ---------------------------------------------------------------------------
# Commit
# ---------------------------------------------------------------------------
log_section "Committing changes"
git add -u
git commit -m "release: v${NEXT_VERSION}"

# ---------------------------------------------------------------------------
# Push + PR
# ---------------------------------------------------------------------------
log_section "Pushing and creating PR"
git push -u origin "$RELEASE_BRANCH"

PR_TITLE="release: v${NEXT_VERSION}"
PR_BODY="## Release v${NEXT_VERSION}"$'\n\n'
PR_BODY+="Prepared by \`cargo make release-prepare\`."$'\n\n'
PR_BODY+="### Changes since ${CHANGES_BASE_REF}"$'\n\n'
PR_BODY+="\`\`\`"$'\n'
PR_BODY+="$(git log "${CHANGES_BASE_REF}..HEAD" --oneline --no-decorate)"$'\n'
PR_BODY+="\`\`\`"$'\n\n'
PR_BODY+="### On merge"$'\n\n'
PR_BODY+="The \`release-tag\` workflow will automatically push the \`v${NEXT_VERSION}\` tag, which triggers \`release.yml\` to build and publish the binaries."$'\n'

gh pr create \
    --title "$PR_TITLE" \
    --body "$PR_BODY" \
    --base main \
    --head "$RELEASE_BRANCH"

log_section "Done"
echo ""
echo "  Release branch: ${RELEASE_BRANCH}"
echo "  PR created — merge to main to trigger the release build."
echo ""
```

- [ ] **Step 2: Smoke-test with dry-run (on main-tracking branch, uncommitted state will warn)**

```bash
chmod +x tools/scripts/release/release-prepare.sh
./tools/scripts/release/release-prepare.sh --dry-run
```

Expected output includes:
- Pre-flight warnings about branch/uncommitted state (normal in dry-run)
- `CLI changed: true`
- `Version: 0.0.1 -> 0.0.2 (patch)`
- `DRY RUN -- no branch created, no PR opened`

- [ ] **Step 3: Commit**

```bash
git add tools/scripts/release/release-prepare.sh
git commit -m "chore(release): add release-prepare.sh orchestrator"
```

---

## Task 8: `tools/cargo-make/release-tasks.toml` + main.toml wiring

**Files:**
- Create: `tools/cargo-make/release-tasks.toml`
- Modify: `tools/cargo-make/main.toml` (add extend)

- [ ] **Step 1: Read current `main.toml` to find the right insertion point**

```bash
head -40 tools/cargo-make/main.toml
```

Note whether `main.toml` currently loads `base-tasks.toml` via `extend` or `[env]` — we want to mirror the pattern.

- [ ] **Step 2: Write `release-tasks.toml`**

```toml
# tools/cargo-make/release-tasks.toml
#
# Release preparation tasks. Wires the scripts under
# tools/scripts/release/ into cargo-make for easy invocation.
#
# Primary entrypoints:
#   cargo make release-check    — dry-run: what would release-prepare do?
#   cargo make release-prepare  — real run: bump version, open release PR
#
# See: docs/superpowers/specs/2026-04-17-temper-cli-binary-release-design.md

[tasks.release-check]
description = "Dry-run release preparation — detect changes, show next version"
category = "Release"
script = [
    "./tools/scripts/release/release-prepare.sh --dry-run"
]

[tasks.release-prepare]
description = "Bump version, create release branch, and open release PR"
category = "Release"
script = [
    "./tools/scripts/release/release-prepare.sh"
]

[tasks.release-prepare-minor]
description = "Release-prepare with a minor bump (0.1.x -> 0.2.0)"
category = "Release"
script = [
    "./tools/scripts/release/release-prepare.sh --bump minor"
]

[tasks.release-prepare-major]
description = "Release-prepare with a major bump (0.x.y -> 1.0.0)"
category = "Release"
script = [
    "./tools/scripts/release/release-prepare.sh --bump major"
]
```

- [ ] **Step 3: Wire into `main.toml`**

Open `tools/cargo-make/main.toml`. At the top (after any existing `extend` line but before task definitions), add:

```toml
[config]
default_to_workspace = false

# Release tasks (release-check, release-prepare)
[env.CARGO_MAKE_EXTEND_WORKSPACE_MAKEFILE]
value = "true"

[tasks.default]
alias = "help"
```

**WAIT — verify first.** `cargo-make` supports `extend` at the top-level for merging external makefile files. Read the existing `main.toml` in full before editing:

```bash
cat tools/cargo-make/main.toml
```

The safe edit pattern is to add a single line near the top:

```toml
extend = [
    { path = "./base-tasks.toml" },
    { path = "./release-tasks.toml" },
]
```

If `main.toml` already uses `extend = "./base-tasks.toml"` (single-value form), convert to the array form shown above. If it uses some other inclusion pattern, mirror that pattern for `release-tasks.toml`.

- [ ] **Step 4: Smoke-test via cargo-make**

```bash
cargo make release-check
```

Expected: same output as running `./tools/scripts/release/release-prepare.sh --dry-run` directly (Task 7 Step 2). Confirms cargo-make is loading the new tasks.

- [ ] **Step 5: Commit**

```bash
git add tools/cargo-make/release-tasks.toml tools/cargo-make/main.toml
git commit -m "chore(release): wire release-prepare into cargo-make"
```

---

## Task 9: `scripts/install/install.sh` (POSIX sh installer)

**Files:**
- Create: `scripts/install/install.sh`

- [ ] **Step 1: Write the installer**

```bash
#!/usr/bin/env sh
# scripts/install/install.sh
#
# Install the latest `temper` CLI binary on macOS (Apple Silicon) or Linux
# (x86_64). Usage:
#
#   curl -fsSL https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.sh | sh
#
# Or to install a specific version:
#
#   curl -fsSL https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.sh \
#     | sh -s -- --version v0.1.0
#
# Installs to:
#   ${XDG_DATA_HOME:-$HOME/.local/share}/temper/
# with a symlink at:
#   ${XDG_BIN_HOME:-$HOME/.local/bin}/temper

set -eu

REPO="tasker-systems/temper"
REQUESTED_VERSION=""

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
while [ $# -gt 0 ]; do
    case $1 in
        --version) REQUESTED_VERSION="$2"; shift 2 ;;
        --version=*) REQUESTED_VERSION="${1#*=}"; shift ;;
        -h|--help)
            cat <<EOF
Usage: install.sh [--version VERSION]

  --version VERSION   Install a specific release tag (e.g. v0.1.0).
                      Defaults to the latest release.
EOF
            exit 0
            ;;
        *) echo "error: unknown argument: $1" >&2; exit 1 ;;
    esac
done

# ---------------------------------------------------------------------------
# Detect OS + architecture
# ---------------------------------------------------------------------------
OS=$(uname -s)
ARCH=$(uname -m)

case "$OS" in
    Darwin)
        if [ "$ARCH" != "arm64" ]; then
            cat >&2 <<EOF
error: no prebuilt binary for macOS ${ARCH}.

Temper v1 only ships macOS arm64 (Apple Silicon) binaries. On Intel Macs,
build from source:

  git clone https://github.com/${REPO}
  cd temper
  cargo install --path crates/temper-cli

If you are on Apple Silicon and seeing this message, you may be running
under Rosetta. Run the installer in a native arm64 terminal.
EOF
            exit 1
        fi
        TARGET="aarch64-apple-darwin"
        ;;
    Linux)
        if [ "$ARCH" != "x86_64" ]; then
            cat >&2 <<EOF
error: no prebuilt binary for Linux ${ARCH}.

Temper v1 only ships Linux x86_64 binaries. Build from source:

  git clone https://github.com/${REPO}
  cd temper
  cargo install --path crates/temper-cli --features embed,extract,hnsw
EOF
            exit 1
        fi
        TARGET="x86_64-unknown-linux-gnu"
        ;;
    *)
        echo "error: unsupported OS: $OS" >&2
        exit 1
        ;;
esac

# ---------------------------------------------------------------------------
# Determine version (latest or explicit)
# ---------------------------------------------------------------------------
if [ -z "$REQUESTED_VERSION" ]; then
    VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' \
        | head -n1)
    if [ -z "$VERSION" ]; then
        echo "error: could not determine latest release from GitHub API" >&2
        exit 1
    fi
else
    VERSION="$REQUESTED_VERSION"
fi

echo "Installing temper ${VERSION} (${TARGET})..."

# ---------------------------------------------------------------------------
# Download + verify
# ---------------------------------------------------------------------------
ARCHIVE="temper-${VERSION}-${TARGET}.tar.gz"
URL_BASE="https://github.com/${REPO}/releases/download/${VERSION}"
ARCHIVE_URL="${URL_BASE}/${ARCHIVE}"
SHA_URL="${URL_BASE}/${ARCHIVE}.sha256"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

echo "  Downloading ${ARCHIVE}..."
curl -fsSL "$ARCHIVE_URL" -o "$TMPDIR/$ARCHIVE"
curl -fsSL "$SHA_URL" -o "$TMPDIR/$ARCHIVE.sha256"

echo "  Verifying checksum..."
cd "$TMPDIR"
if [ "$OS" = "Darwin" ]; then
    EXPECTED=$(awk '{print $1}' "$ARCHIVE.sha256")
    ACTUAL=$(shasum -a 256 "$ARCHIVE" | awk '{print $1}')
    [ "$EXPECTED" = "$ACTUAL" ] || { echo "error: checksum mismatch"; exit 1; }
else
    sha256sum -c "$ARCHIVE.sha256" >/dev/null
fi
cd - >/dev/null

# ---------------------------------------------------------------------------
# Extract into ~/.local/share/temper/
# ---------------------------------------------------------------------------
INSTALL_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/temper"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"

mkdir -p "$INSTALL_DIR" "$BIN_DIR"
echo "  Extracting to ${INSTALL_DIR}..."
tar -xzf "$TMPDIR/$ARCHIVE" -C "$INSTALL_DIR"

ln -sf "$INSTALL_DIR/temper" "$BIN_DIR/temper"

# ---------------------------------------------------------------------------
# PATH check
# ---------------------------------------------------------------------------
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
        cat <<EOF

⚠️  $BIN_DIR is not on your PATH. Add it by running ONE of the following,
   depending on your shell:

   # bash
   echo 'export PATH="\$PATH:$BIN_DIR"' >> ~/.bashrc

   # zsh
   echo 'export PATH="\$PATH:$BIN_DIR"' >> ~/.zshrc

   # fish
   fish_add_path $BIN_DIR
EOF
        ;;
esac

cat <<EOF

✓ Installed temper ${VERSION} to ${INSTALL_DIR}
  Run:  temper --help
EOF
```

- [ ] **Step 2: Smoke-test (syntax + help flag)**

```bash
chmod +x scripts/install/install.sh
sh -n scripts/install/install.sh  # POSIX sh syntax check
sh scripts/install/install.sh --help
```

Expected: `sh -n` exits 0, `--help` prints the usage block.

No real install test yet — requires a real release to exist. We'll test end-to-end in Task 17 (first release).

- [ ] **Step 3: Commit**

```bash
git add scripts/install/install.sh
git commit -m "feat(install): add POSIX sh installer for macOS + Linux"
```

---

## Task 10: `scripts/install/install.ps1` (Windows installer)

**Files:**
- Create: `scripts/install/install.ps1`

- [ ] **Step 1: Write the installer**

```powershell
# scripts/install/install.ps1
#
# Install the latest `temper` CLI binary on Windows x86_64. Usage:
#
#   irm https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.ps1 | iex
#
# Or to install a specific version:
#
#   $script = irm https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.ps1
#   & ([scriptblock]::Create($script)) -Version v0.1.0
#
# Installs to:
#   $env:LOCALAPPDATA\Programs\temper\
# and appends that directory to the user PATH.

[CmdletBinding()]
param(
    [string]$Version = ""
)

$ErrorActionPreference = 'Stop'

$Repo = "tasker-systems/temper"

# ---------------------------------------------------------------------------
# PowerShell version check
# ---------------------------------------------------------------------------
if ($PSVersionTable.PSVersion.Major -lt 5) {
    Write-Error "PowerShell 5.1 or later is required. Found: $($PSVersionTable.PSVersion)"
    exit 1
}

# ---------------------------------------------------------------------------
# Architecture check
# ---------------------------------------------------------------------------
if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
    Write-Error @"
No prebuilt binary for Windows $($env:PROCESSOR_ARCHITECTURE).

Temper v1 only ships Windows x86_64 binaries. Build from source requires
installing Rust (https://rustup.rs) and running:

  git clone https://github.com/$Repo
  cd temper
  cargo install --path crates/temper-cli --features embed,extract,hnsw
"@
    exit 1
}

$Target = "x86_64-pc-windows-msvc"

# ---------------------------------------------------------------------------
# Determine version
# ---------------------------------------------------------------------------
if (-not $Version) {
    try {
        $latest = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
        $Version = $latest.tag_name
    } catch {
        Write-Error "Could not determine latest release: $_"
        exit 1
    }
}

if (-not $Version) {
    Write-Error "Could not determine a version to install."
    exit 1
}

Write-Host "Installing temper $Version ($Target)..."

# ---------------------------------------------------------------------------
# Download + verify
# ---------------------------------------------------------------------------
$Archive = "temper-$Version-$Target.zip"
$UrlBase = "https://github.com/$Repo/releases/download/$Version"
$ArchiveUrl = "$UrlBase/$Archive"
$ShaUrl = "$UrlBase/$Archive.sha256"

$TmpDir = Join-Path $env:TEMP "temper-install-$(Get-Random)"
New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null

try {
    $ArchivePath = Join-Path $TmpDir $Archive
    $ShaPath = "$ArchivePath.sha256"

    Write-Host "  Downloading $Archive..."
    Invoke-WebRequest -Uri $ArchiveUrl -OutFile $ArchivePath -UseBasicParsing
    Invoke-WebRequest -Uri $ShaUrl -OutFile $ShaPath -UseBasicParsing

    Write-Host "  Verifying checksum..."
    $expected = (Get-Content $ShaPath -Raw).Trim().Split()[0].ToLowerInvariant()
    $actual = (Get-FileHash $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($expected -ne $actual) {
        Write-Error "Checksum mismatch. Expected: $expected, got: $actual"
        exit 1
    }

    # -----------------------------------------------------------------------
    # Extract to %LOCALAPPDATA%\Programs\temper\
    # -----------------------------------------------------------------------
    $InstallDir = Join-Path $env:LOCALAPPDATA "Programs\temper"

    if (Test-Path $InstallDir) {
        Write-Host "  Removing previous install at $InstallDir..."
        Remove-Item -Recurse -Force $InstallDir
    }

    Write-Host "  Extracting to $InstallDir..."
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Expand-Archive -Path $ArchivePath -DestinationPath $InstallDir -Force

    # -----------------------------------------------------------------------
    # Append to user PATH
    # -----------------------------------------------------------------------
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($userPath -notlike "*$InstallDir*") {
        $newPath = if ($userPath) { "$userPath;$InstallDir" } else { $InstallDir }
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        Write-Host "  Added $InstallDir to user PATH (restart your shell to take effect)"
    } else {
        Write-Host "  User PATH already contains $InstallDir"
    }
} finally {
    Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "Installed temper $Version to $InstallDir"
Write-Host "Run: temper --help"
Write-Host ""
Write-Host "Note: restart your terminal (or sign out and back in) for the PATH"
Write-Host "change to take effect."
```

- [ ] **Step 2: Syntax validate locally (if PowerShell is available, else defer to CI)**

```bash
# If pwsh is installed on the dev machine:
if command -v pwsh >/dev/null; then
    pwsh -NoProfile -Command "Get-Content scripts/install/install.ps1 | Out-Null; [scriptblock]::Create((Get-Content -Raw scripts/install/install.ps1)) | Out-Null; Write-Host 'OK'"
fi
```

Expected: prints `OK`, or this step is skipped if pwsh isn't present. Real test happens on windows-2022 runner during Task 15.

- [ ] **Step 3: Commit**

```bash
git add scripts/install/install.ps1
git commit -m "feat(install): add PowerShell installer for Windows"
```

---

## Task 11: `docs/guides/install.md`

**Files:**
- Create: `docs/guides/install.md`

- [ ] **Step 1: Write the guide**

```markdown
# Installing Temper

Temper is distributed as a self-contained binary for macOS (Apple Silicon),
Linux (x86_64), and Windows (x86_64). The installer drops a `temper` binary
and a bundled ONNX Runtime library into your home directory and adds `temper`
to your PATH.

No Rust toolchain, no system package manager, no homebrew tap required.

## Quick install

### macOS and Linux

```sh
curl -fsSL https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.sh | sh
```

### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.ps1 | iex
```

> If PowerShell warns about the execution policy, run:
> ```powershell
> powershell -ExecutionPolicy Bypass -c "irm https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.ps1 | iex"
> ```

## What the installer does

1. Detects your OS and CPU architecture.
2. Queries GitHub for the latest release tag.
3. Downloads the matching archive (a `.tar.gz` on macOS/Linux, `.zip` on
   Windows) plus its SHA256 checksum file.
4. Verifies the checksum.
5. Extracts the archive into:
   - macOS/Linux: `~/.local/share/temper/` (respects `$XDG_DATA_HOME`)
   - Windows: `%LOCALAPPDATA%\Programs\temper\`
6. Creates a `temper` entry on your PATH:
   - macOS/Linux: symlinks `~/.local/bin/temper` → the extracted binary
   - Windows: appends the install directory to your user PATH

The archive contains `temper[.exe]`, a bundled `libonnxruntime` for local
embedding workflows (`temper graph build`, `temper index`), and a copy of the
project LICENSE.

## Pinning to a specific version

```sh
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.sh | sh -s -- --version v0.1.0
```

```powershell
# Windows
$script = irm https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.ps1
& ([scriptblock]::Create($script)) -Version v0.1.0
```

## Don't want to pipe to `sh`?

Download the script, read it, then run it:

```sh
curl -fsSL -o /tmp/install-temper.sh https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.sh
less /tmp/install-temper.sh         # inspect
sh /tmp/install-temper.sh           # run
```

Or grab the release tarball directly from
[github.com/tasker-systems/temper/releases](https://github.com/tasker-systems/temper/releases)
and unpack it wherever you like.

## Upgrading

Run the installer again — it overwrites the previous install in place.

## Uninstalling

### macOS / Linux

```sh
rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/temper"
rm -f "${XDG_BIN_HOME:-$HOME/.local/bin}/temper"
```

### Windows

```powershell
Remove-Item -Recurse -Force "$env:LOCALAPPDATA\Programs\temper"
# Then manually remove the install dir from your user PATH via:
#   rundll32.exe sysdm.cpl,EditEnvironmentVariables
```

## Building from source

If you're on a platform we don't ship binaries for (Linux arm64, Intel Mac,
Windows arm64) or you want a custom build, clone the repo and `cargo install`:

```sh
git clone https://github.com/tasker-systems/temper
cd temper
cargo install --path crates/temper-cli --features embed,extract,hnsw
```

You'll need:
- A Rust toolchain (install via [rustup](https://rustup.rs))
- A C++ compiler (for transitive deps)
- ONNX Runtime installed on your system if you want `temper graph build`
  and `temper index` to work. On macOS, `brew install onnxruntime` suffices.

## Troubleshooting

### "temper: command not found" after install (macOS/Linux)

Your shell's PATH doesn't include `~/.local/bin`. Add it:

```sh
# bash
echo 'export PATH="$PATH:$HOME/.local/bin"' >> ~/.bashrc

# zsh
echo 'export PATH="$PATH:$HOME/.local/bin"' >> ~/.zshrc

# fish
fish_add_path ~/.local/bin
```

Then open a new terminal.

### Windows: "temper : The term 'temper' is not recognized"

Restart your terminal. If the problem persists, log out of Windows and back
in (or reboot) so the updated user PATH propagates.

### Windows: SmartScreen warning

The `temper.exe` binary is currently unsigned. On first run, you may see a
SmartScreen "Windows protected your PC" dialog. Click **More info** →
**Run anyway**. (Code-signing is tracked as a future enhancement.)

### `temper graph build` fails with "ONNX Runtime not found"

The installer bundles `libonnxruntime` next to the `temper` binary, and the
binary looks for it there automatically. If this error appears after a fresh
install, file an issue at https://github.com/tasker-systems/temper/issues
with the output of:

```sh
temper --version
ls -la ~/.local/share/temper/     # macOS / Linux
dir %LOCALAPPDATA%\Programs\temper # Windows
```
```

- [ ] **Step 2: Commit**

```bash
git add docs/guides/install.md
git commit -m "docs(install): add user-facing install guide"
```

---

## Task 12: `.github/scripts/release/` helpers

**Files:**
- Create: `.github/scripts/release/create-github-release.sh`
- Create: `.github/scripts/release/generate-summary.sh`
- Create: `.github/scripts/release/check-failures.sh`

These run inside the `release.yml` workflow. Kept small and single-purpose so the workflow YAML stays readable.

- [ ] **Step 1: Write `create-github-release.sh`**

```bash
#!/usr/bin/env bash
# .github/scripts/release/create-github-release.sh
#
# Create a GitHub Release for the current tag and attach CLI binary artifacts.
#
# Required env:
#   GH_TOKEN     — GitHub token with contents:write
#   VERSION      — e.g. 0.1.0 (without the "v" prefix)
#   ARTIFACT_DIR — directory containing temper-v*.{tar.gz,zip,sha256}

set -euo pipefail

: "${GH_TOKEN:?GH_TOKEN required}"
: "${VERSION:?VERSION required}"
: "${ARTIFACT_DIR:?ARTIFACT_DIR required}"

TAG="v${VERSION}"

# Create the release with auto-generated notes. --generate-notes uses
# commits since the previous tag, so the release-prepare PR body naturally
# shows up as the leading bullet if that was the most recent merge.
if ! gh release view "$TAG" >/dev/null 2>&1; then
    echo "Creating release $TAG..."
    gh release create "$TAG" \
        --title "temper $TAG" \
        --generate-notes
else
    echo "Release $TAG already exists, skipping create step."
fi

# Upload every tarball / zip / sha256 file in the artifact dir
echo "Uploading artifacts from ${ARTIFACT_DIR}..."
shopt -s nullglob
for f in "${ARTIFACT_DIR}"/temper-*.tar.gz \
         "${ARTIFACT_DIR}"/temper-*.zip \
         "${ARTIFACT_DIR}"/temper-*.sha256; do
    echo "  Uploading $(basename "$f")..."
    gh release upload "$TAG" "$f" --clobber
done

echo "Done."
```

- [ ] **Step 2: Write `generate-summary.sh`**

```bash
#!/usr/bin/env bash
# .github/scripts/release/generate-summary.sh
#
# Print a per-platform summary to $GITHUB_STEP_SUMMARY.
#
# Required env:
#   VERSION           — e.g. 0.1.0
#   BUILD_RESULTS_JSON — JSON string from GitHub's `needs` context
#                        for the build-cli-binaries matrix jobs.
#                        Or simpler: one env var per platform.
#   DARWIN_ARM64_RESULT — success|failure|cancelled|skipped
#   LINUX_X64_RESULT
#   WINDOWS_X64_RESULT

set -euo pipefail

: "${VERSION:?VERSION required}"
: "${GITHUB_STEP_SUMMARY:?GITHUB_STEP_SUMMARY required}"

DARWIN_ARM64_RESULT="${DARWIN_ARM64_RESULT:-unknown}"
LINUX_X64_RESULT="${LINUX_X64_RESULT:-unknown}"
WINDOWS_X64_RESULT="${WINDOWS_X64_RESULT:-unknown}"

icon() {
    case "$1" in
        success) echo "✅" ;;
        failure) echo "❌" ;;
        cancelled) echo "⏹️" ;;
        skipped) echo "⏭️" ;;
        *) echo "❓" ;;
    esac
}

{
    echo "## Release v${VERSION}"
    echo ""
    echo "| Platform | Result |"
    echo "|---|---|"
    echo "| darwin-arm64 | $(icon "$DARWIN_ARM64_RESULT") $DARWIN_ARM64_RESULT |"
    echo "| linux-x64    | $(icon "$LINUX_X64_RESULT") $LINUX_X64_RESULT |"
    echo "| windows-x64  | $(icon "$WINDOWS_X64_RESULT") $WINDOWS_X64_RESULT |"
} >> "$GITHUB_STEP_SUMMARY"
```

- [ ] **Step 3: Write `check-failures.sh`**

```bash
#!/usr/bin/env bash
# .github/scripts/release/check-failures.sh
#
# Emit has_failures=true|false to $GITHUB_OUTPUT based on per-platform
# build results.
#
# Required env:
#   DARWIN_ARM64_RESULT
#   LINUX_X64_RESULT
#   WINDOWS_X64_RESULT

set -euo pipefail

: "${GITHUB_OUTPUT:?GITHUB_OUTPUT required}"

HAS_FAILURES=false

for var in DARWIN_ARM64_RESULT LINUX_X64_RESULT WINDOWS_X64_RESULT; do
    value="${!var:-unknown}"
    if [[ "$value" != "success" && "$value" != "skipped" ]]; then
        HAS_FAILURES=true
        echo "::warning::$var = $value"
    fi
done

echo "has_failures=${HAS_FAILURES}" >> "$GITHUB_OUTPUT"
```

- [ ] **Step 4: Make all three executable and smoke-test locally**

```bash
chmod +x .github/scripts/release/create-github-release.sh \
         .github/scripts/release/generate-summary.sh \
         .github/scripts/release/check-failures.sh

# Syntax check
bash -n .github/scripts/release/create-github-release.sh
bash -n .github/scripts/release/generate-summary.sh
bash -n .github/scripts/release/check-failures.sh

# check-failures.sh against fake env vars
(
    export GITHUB_OUTPUT=/tmp/fake-output
    export DARWIN_ARM64_RESULT=success
    export LINUX_X64_RESULT=success
    export WINDOWS_X64_RESULT=failure
    rm -f "$GITHUB_OUTPUT"
    ./.github/scripts/release/check-failures.sh
    cat "$GITHUB_OUTPUT"
)
```

Expected: `has_failures=true` on stdout (via GITHUB_OUTPUT).

- [ ] **Step 5: Commit**

```bash
git add .github/scripts/release/
git commit -m "chore(release): add release.yml helper scripts"
```

---

## Task 13: `.github/workflows/build-cli-binaries.yml`

**Files:**
- Create: `.github/workflows/build-cli-binaries.yml`

Reusable subworkflow. Called by `release.yml` with a `version` input. Emits per-platform tarball + sha256 artifacts.

- [ ] **Step 1: Write the workflow**

```yaml
name: Build CLI Binaries

on:
  workflow_call:
    inputs:
      version:
        description: 'Version string for artifact naming (without leading v)'
        required: true
        type: string
  workflow_dispatch:
    inputs:
      version:
        description: 'Version string for artifact naming'
        required: false
        default: 'dev'

# =============================================================================
# ONNX Runtime version pinned here. Bump when upgrading ort in
# crates/temper-ingest. Sources:
#   https://github.com/microsoft/onnxruntime/releases
# =============================================================================
env:
  ONNX_RUNTIME_VERSION: '1.24.2'

jobs:
  build:
    name: ${{ matrix.target.name }}
    runs-on: ${{ matrix.target.runner }}
    timeout-minutes: 30
    env:
      SQLX_OFFLINE: 'true'
    strategy:
      fail-fast: false
      matrix:
        target:
          - name: darwin-arm64
            runner: macos-14
            triple: aarch64-apple-darwin
            ort_archive: onnxruntime-osx-arm64
            ort_archive_ext: tgz
            lib_name: libonnxruntime.dylib
            lib_dest_dir: lib
            archive_ext: tar.gz
          - name: linux-x64
            runner: ubuntu-22.04
            triple: x86_64-unknown-linux-gnu
            ort_archive: onnxruntime-linux-x64
            ort_archive_ext: tgz
            lib_name: libonnxruntime.so
            lib_dest_dir: lib
            archive_ext: tar.gz
          - name: windows-x64
            runner: windows-2022
            triple: x86_64-pc-windows-msvc
            ort_archive: onnxruntime-win-x64
            ort_archive_ext: zip
            lib_name: onnxruntime.dll
            lib_dest_dir: .
            archive_ext: zip

    steps:
      - name: Checkout code
        uses: actions/checkout@v6

      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target.triple }}

      - name: Setup Rust build cache
        uses: Swatinem/rust-cache@v2
        with:
          shared-key: release-${{ matrix.target.triple }}

      - name: Build temper CLI (${{ matrix.target.triple }})
        shell: bash
        run: |
          cargo build \
            --release \
            --package temper-cli \
            --target ${{ matrix.target.triple }} \
            --features embed,extract,hnsw

      - name: Download ONNX Runtime (${{ matrix.target.name }})
        shell: bash
        run: |
          ORT_VER="${{ env.ONNX_RUNTIME_VERSION }}"
          ORT_NAME="${{ matrix.target.ort_archive }}-${ORT_VER}"
          ORT_EXT="${{ matrix.target.ort_archive_ext }}"
          ORT_URL="https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VER}/${ORT_NAME}.${ORT_EXT}"

          mkdir -p ort-staging
          echo "Downloading ${ORT_URL}..."
          curl -fsSL -o "ort-staging/ort.${ORT_EXT}" "$ORT_URL"

          echo "Extracting..."
          if [[ "$ORT_EXT" == "tgz" ]]; then
            tar -xzf "ort-staging/ort.${ORT_EXT}" -C ort-staging
          else
            # .zip on Windows — use 7z (preinstalled on windows-2022 runners)
            7z x "ort-staging/ort.${ORT_EXT}" -oort-staging -y >/dev/null
          fi

          echo "Contents:"
          find ort-staging -name 'libonnxruntime*' -o -name 'onnxruntime.dll' | head -20

      - name: Assemble archive contents
        shell: bash
        run: |
          VERSION="${{ inputs.version }}"
          TARGET="${{ matrix.target.triple }}"
          STAGING="staging"
          rm -rf "$STAGING"
          mkdir -p "$STAGING/${{ matrix.target.lib_dest_dir }}"

          # Binary
          BIN_SRC="target/${TARGET}/release/temper"
          if [[ "${{ matrix.target.name }}" == "windows-x64" ]]; then
            BIN_SRC="${BIN_SRC}.exe"
            cp "$BIN_SRC" "$STAGING/temper.exe"
          else
            cp "$BIN_SRC" "$STAGING/temper"
            chmod +x "$STAGING/temper"
          fi

          # ONNX Runtime lib — find it anywhere under ort-staging/
          ORT_LIB=$(find ort-staging -name '${{ matrix.target.lib_name }}' -type f | head -n1)
          if [[ -z "$ORT_LIB" ]]; then
            echo "::error::Could not locate ${{ matrix.target.lib_name }} in extracted archive"
            find ort-staging -type f | head -50
            exit 1
          fi

          if [[ "${{ matrix.target.lib_dest_dir }}" == "." ]]; then
            cp "$ORT_LIB" "$STAGING/${{ matrix.target.lib_name }}"
          else
            cp "$ORT_LIB" "$STAGING/${{ matrix.target.lib_dest_dir }}/${{ matrix.target.lib_name }}"
          fi

          cp LICENSE "$STAGING/"
          cat > "$STAGING/README-INSTALL.txt" <<EOF
          temper v${VERSION} — ${TARGET}

          Install this archive via:
            https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.sh
          or
            https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.ps1

          See full install docs at:
            https://github.com/tasker-systems/temper/blob/main/docs/guides/install.md
          EOF

          echo "Staging contents:"
          find "$STAGING" -type f

      - name: Create archive (tar.gz)
        if: matrix.target.archive_ext == 'tar.gz'
        shell: bash
        run: |
          VERSION="${{ inputs.version }}"
          TARGET="${{ matrix.target.triple }}"
          ARCHIVE="temper-v${VERSION}-${TARGET}.tar.gz"
          tar -czf "$ARCHIVE" -C staging .
          ls -lh "$ARCHIVE"
          shasum -a 256 "$ARCHIVE" > "${ARCHIVE}.sha256"
          cat "${ARCHIVE}.sha256"

      - name: Create archive (zip)
        if: matrix.target.archive_ext == 'zip'
        shell: bash
        run: |
          VERSION="${{ inputs.version }}"
          TARGET="${{ matrix.target.triple }}"
          ARCHIVE="temper-v${VERSION}-${TARGET}.zip"
          (cd staging && 7z a -tzip "../${ARCHIVE}" . >/dev/null)
          ls -lh "$ARCHIVE"
          # On windows-2022 runners, certutil produces hex hash; sha256sum is available via git bash
          sha256sum "$ARCHIVE" > "${ARCHIVE}.sha256"
          cat "${ARCHIVE}.sha256"

      - name: Upload artifact
        uses: actions/upload-artifact@v7
        with:
          name: cli-v${{ inputs.version }}-${{ matrix.target.name }}
          path: |
            temper-v${{ inputs.version }}-*.tar.gz
            temper-v${{ inputs.version }}-*.zip
            temper-v${{ inputs.version }}-*.sha256
          retention-days: 7
          if-no-files-found: error
```

- [ ] **Step 2: Syntactic validation (no real run yet)**

```bash
# If `actionlint` is installed locally, use it. Otherwise skip — will be
# validated by GitHub on first push.
command -v actionlint >/dev/null && actionlint .github/workflows/build-cli-binaries.yml || echo "(actionlint not installed, will validate on GitHub)"
```

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/build-cli-binaries.yml
git commit -m "ci(release): add reusable build-cli-binaries workflow"
```

---

## Task 14: `.github/workflows/release-tag.yml`

**Files:**
- Create: `.github/workflows/release-tag.yml`

Fires on pushes to `main` that touch `VERSION`. Reads the new version, creates + pushes `v<N.N.N>` annotated tag.

- [ ] **Step 1: Write the workflow**

```yaml
name: Release Tag

on:
  push:
    branches:
      - main
    paths:
      - 'VERSION'

concurrency:
  group: release-tag
  cancel-in-progress: false

jobs:
  tag:
    name: Create release tag
    runs-on: ubuntu-22.04
    timeout-minutes: 5
    permissions:
      contents: write
    steps:
      - name: Checkout code
        uses: actions/checkout@v6
        with:
          fetch-depth: 0

      - name: Read VERSION
        id: read
        run: ./tools/scripts/release/read-version.sh >> "$GITHUB_OUTPUT"

      - name: Check tag does not already exist
        id: check
        run: |
          TAG="v${{ steps.read.outputs.VERSION }}"
          if git rev-parse "$TAG" >/dev/null 2>&1; then
            echo "Tag $TAG already exists, skipping." >&2
            echo "exists=true" >> "$GITHUB_OUTPUT"
          else
            echo "exists=false" >> "$GITHUB_OUTPUT"
          fi

      - name: Create and push tag
        if: steps.check.outputs.exists != 'true'
        run: |
          TAG="v${{ steps.read.outputs.VERSION }}"
          git config user.name "github-actions[bot]"
          git config user.email "github-actions[bot]@users.noreply.github.com"
          git tag -a "$TAG" -m "Release $TAG"
          git push origin "$TAG"
```

- [ ] **Step 2: Validate locally**

```bash
command -v actionlint >/dev/null && actionlint .github/workflows/release-tag.yml || echo "(actionlint not installed)"
```

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release-tag.yml
git commit -m "ci(release): add release-tag workflow (push tag on VERSION change)"
```

---

## Task 15: `.github/workflows/release.yml`

**Files:**
- Create: `.github/workflows/release.yml`

Triggers on `v*` tag push. Pre-flight → build matrix → summary + GitHub Release.

- [ ] **Step 1: Write the workflow**

```yaml
name: Release

on:
  push:
    tags:
      - 'v*'
  workflow_dispatch:
    inputs:
      tag:
        description: 'Tag to release (e.g. v0.1.0)'
        required: true
        type: string

concurrency:
  group: release
  cancel-in-progress: false

jobs:
  # ============================================================================
  # Stage 1: Determine version from the tag
  # ============================================================================
  determine-version:
    name: Determine Version
    runs-on: ubuntu-22.04
    timeout-minutes: 2
    outputs:
      version: ${{ steps.version.outputs.version }}
    steps:
      - name: Extract version from tag
        id: version
        run: |
          if [[ "${{ github.event_name }}" == "workflow_dispatch" ]]; then
            TAG="${{ inputs.tag }}"
          else
            TAG="${{ github.ref_name }}"
          fi
          VERSION="${TAG#v}"
          echo "version=${VERSION}" >> "$GITHUB_OUTPUT"
          echo "Releasing temper v${VERSION}"

  # ============================================================================
  # Stage 2: Pre-flight validation
  # ============================================================================
  pre-flight-check:
    name: Pre-flight Validation
    runs-on: ubuntu-22.04
    timeout-minutes: 15
    needs: determine-version
    env:
      SQLX_OFFLINE: 'true'
    steps:
      - name: Checkout code
        uses: actions/checkout@v6

      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - name: Setup Rust build cache
        uses: Swatinem/rust-cache@v2
        with:
          shared-key: release-preflight

      - name: Check formatting
        run: cargo fmt --all -- --check

      - name: Run clippy
        run: cargo clippy --package temper-cli --all-features -- -D warnings

      - name: Release build smoke test
        run: cargo build --release --package temper-cli --features embed,extract,hnsw

  # ============================================================================
  # Stage 3: Build CLI binaries (3-platform matrix)
  # ============================================================================
  build-cli-binaries:
    name: Build CLI Binaries
    needs: [determine-version, pre-flight-check]
    permissions:
      contents: read
    uses: ./.github/workflows/build-cli-binaries.yml
    with:
      version: ${{ needs.determine-version.outputs.version }}

  # ============================================================================
  # Stage 4: Create GitHub Release + upload artifacts + summary
  # ============================================================================
  release-summary:
    name: Release Summary
    runs-on: ubuntu-22.04
    timeout-minutes: 5
    needs: [determine-version, build-cli-binaries]
    if: always()
    permissions:
      contents: write
    steps:
      - name: Checkout code
        uses: actions/checkout@v6

      - name: Download CLI binary artifacts
        if: needs.build-cli-binaries.result == 'success' || needs.build-cli-binaries.result == 'failure'
        uses: actions/download-artifact@v8
        with:
          pattern: cli-v${{ needs.determine-version.outputs.version }}-*
          path: cli-artifacts/
          merge-multiple: true

      - name: List artifacts
        run: |
          if [[ -d cli-artifacts ]]; then
            find cli-artifacts -type f | sort
          else
            echo "No artifacts downloaded."
          fi

      - name: Check for build failures
        id: check-failures
        env:
          DARWIN_ARM64_RESULT: ${{ needs.build-cli-binaries.result }}
          LINUX_X64_RESULT: ${{ needs.build-cli-binaries.result }}
          WINDOWS_X64_RESULT: ${{ needs.build-cli-binaries.result }}
        run: ./.github/scripts/release/check-failures.sh

      - name: Generate summary
        env:
          VERSION: ${{ needs.determine-version.outputs.version }}
          DARWIN_ARM64_RESULT: ${{ needs.build-cli-binaries.result }}
          LINUX_X64_RESULT: ${{ needs.build-cli-binaries.result }}
          WINDOWS_X64_RESULT: ${{ needs.build-cli-binaries.result }}
        run: ./.github/scripts/release/generate-summary.sh

      - name: Create GitHub Release
        if: steps.check-failures.outputs.has_failures != 'true'
        env:
          GH_TOKEN: ${{ github.token }}
          VERSION: ${{ needs.determine-version.outputs.version }}
          ARTIFACT_DIR: cli-artifacts
        run: ./.github/scripts/release/create-github-release.sh

      - name: Fail the job if any build failed
        if: steps.check-failures.outputs.has_failures == 'true'
        run: |
          echo "::error::One or more platform builds failed; release was not published."
          exit 1
```

> **Note on matrix-result propagation:** The `needs.build-cli-binaries.result` is a single aggregate value (the result of the whole reusable workflow), not per-matrix. A more precise per-target result would require the reusable workflow to emit outputs per platform — we accept the simpler aggregate for v1, which means a single platform failure marks the whole release as failed. This is the conservative choice; we can add per-platform granularity later if we need to ship a partial release.

- [ ] **Step 2: Validate locally**

```bash
command -v actionlint >/dev/null && actionlint .github/workflows/release.yml || echo "(actionlint not installed)"
```

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci(release): add release.yml (tag-driven build + GH Release)"
```

---

## Task 16: README.md install section rewrite

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Read current README.md**

```bash
cat README.md
```

Identify the current install section (probably a `## Installation` or `## Getting Started` header referencing `cargo install`).

- [ ] **Step 2: Replace the install section**

Replace whatever install section exists with:

```markdown
## Install

Install the latest `temper` CLI binary — no Rust toolchain required.

**macOS (Apple Silicon) and Linux (x86_64):**

```sh
curl -fsSL https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.sh | sh
```

**Windows (x86_64):**

```powershell
irm https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.ps1 | iex
```

> Windows support in v0.1.0 is experimental — please file issues at
> https://github.com/tasker-systems/temper/issues if you hit problems.

For version pinning, uninstall instructions, and build-from-source
alternatives, see [docs/guides/install.md](docs/guides/install.md).
```

- [ ] **Step 3: Update the project elevator pitch if needed**

Read the top of README.md. If the first paragraph references temper as a research project or omits the "knowledge base for AI-assisted development" framing from `CLAUDE.md`, refresh it. Suggested opener (preserve the existing tone if it's working):

```markdown
# Temper

Temper is a knowledge base for AI-assisted development. A local vault of
markdown files with YAML frontmatter gives AI agents session continuity
across conversations — goals, tasks, sessions, and decisions persist
between runs. The `temper` CLI manages the vault locally; the cloud API
(optional) syncs and provides semantic search.

- **CLI**: [install](#install) and run `temper --help`
- **Docs**: [docs/guides/install.md](docs/guides/install.md) · [CLAUDE.md](CLAUDE.md) (contributor guide)
- **Source layout**: Rust workspace (`crates/`) + TypeScript/Bun (`packages/`)
```

Keep whatever sections already exist after this (architecture, build commands, contributing). Only replace the lead + the install section.

- [ ] **Step 4: Verify markdown formatting and commit**

```bash
# Inspect:
head -80 README.md

# Optional: render via a markdown tool if available, otherwise just eyeball.
git add README.md
git commit -m "docs: update README install section and project lead"
```

---

## Task 17: First test release (`v0.0.1-rc1`)

**Not a code change — a verification ritual.** After the implementation PR merges to main, manually exercise the full release pipeline once against a throwaway tag before cutting a real `v0.1.0`.

- [ ] **Step 1: Push a test tag**

```bash
git checkout main
git pull
git tag v0.0.1-rc1 -m "Release pipeline smoke test"
git push origin v0.0.1-rc1
```

- [ ] **Step 2: Watch the `release.yml` run in GitHub Actions**

Expected flow:
1. `determine-version` → `0.0.1-rc1`
2. `pre-flight-check` → PASS
3. `build-cli-binaries` × 3 → PASS
4. `release-summary` → creates GH Release `v0.0.1-rc1` with 3 archives + 3 checksums

- [ ] **Step 3: Install the test release on each platform you can access**

On macOS:
```sh
curl -fsSL https://github.com/tasker-systems/temper/releases/download/v0.0.1-rc1/temper-v0.0.1-rc1-aarch64-apple-darwin.tar.gz \
    -o /tmp/test.tar.gz
tar -xzf /tmp/test.tar.gz -C /tmp/test-install
/tmp/test-install/temper --version
/tmp/test-install/temper --help
```

On Linux / Windows: equivalent.

Run `temper graph build` in a throwaway vault to confirm the bundled `libonnxruntime` loads without `ORT_DYLIB_PATH`.

- [ ] **Step 4: If everything works, delete the test release and tag**

```bash
gh release delete v0.0.1-rc1 --yes
git push --delete origin v0.0.1-rc1
git tag -d v0.0.1-rc1
```

If something didn't work, open a focused follow-up PR for the specific failure — don't batch fixes — then repeat Steps 1-3 with a new `-rc2` tag.

- [ ] **Step 5: Cut `v0.1.0` for real**

```bash
git checkout main
git pull
cargo make release-prepare --bump minor   # 0.0.1 -> 0.1.0
# Reviews, merges the release PR
# release-tag workflow pushes v0.1.0
# release workflow builds and creates GH Release
```

Announce in whatever channel is appropriate. Update any pinned installer-doc links to point at v0.1.0 once it exists.

---

## Self-Review Appendix

Writing-plans skill requires a self-review against the spec. Findings:

**Spec coverage:**

| Spec section | Task |
|---|---|
| Platform matrix (3 targets) | 13 |
| Archive flat layout | 13 |
| `embed.rs` fallback chain | 1 |
| `VERSION` source of truth | 2, 6 |
| `cargo make release-prepare` flow | 7, 8 |
| `release-tag.yml` on VERSION change | 14 |
| `release.yml` DAG | 15 |
| Installer scripts (sh + ps1) | 9, 10 |
| Install docs + README rewrite | 11, 16 |
| Supporting release scripts | 2-7, 12 |
| First test release ritual | 17 |

**Placeholder scan:** no "TBD"/"TODO"/"fill in later" in any task body. All code is actual code.

**Type consistency:** `resolve_dylib_from_candidates`, `binary_adjacent_candidates`, `xdg_data_candidates` are spelled identically across Tasks 1 tests + 1 implementation. Script names are spelled identically across Tasks 2-7 and Task 8 (cargo-make wiring).
