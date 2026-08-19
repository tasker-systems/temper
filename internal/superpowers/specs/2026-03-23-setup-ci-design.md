# Setup CI — Design Spec

**Ticket:** `2026-03-23-setup-ci`
**Project:** temper
**Date:** 2026-03-23

## Goal

Add continuous integration to the temper CLI project with code quality checks, testing, and local git hooks. Match the reusable workflow patterns used by sibling projects (storyteller, tasker-core) for consistency.

## Constraints

- No HF model downloads in CI — tests that call `Embedder::ensure_model()` / `embed()` / `embed_batch()` are local-only
- Lean workflow suitable for a single-binary Rust CLI (v0.1.0)
- Off-the-shelf caching (`Swatinem/rust-cache`) — a deliberate simplification over sibling projects' custom `setup-rust-cache` actions, appropriate for temper's lighter compile surface
- Release/binary-build workflows are out of scope (future work)

## Design

### 1. CI Workflow Structure

Four reusable workflow files matching storyteller's caller/callee pattern:

```
.github/workflows/
├── ci.yml              # Orchestrator
├── code-quality.yml    # fmt, clippy, doc, audit
├── test-rust.yml       # cargo test (default features)
└── ci-success.yml      # Branch protection gate
```

**`ci.yml`** — Orchestrator

- Triggers: `push: [main]`, `pull_request: [main]`
- Calls `code-quality.yml` and `test-rust.yml` in parallel via `workflow_call`
- Calls `ci-success.yml` with `needs: [code-quality, test-rust]` (the `needs` clause lives here in the caller, not inside the callee workflow)

**`code-quality.yml`** — Reusable workflow (`on: workflow_call`)

- Runner: `ubuntu-latest`
- `timeout-minutes: 15`
- Steps:
  1. `actions/checkout@v4`
  2. `dtolnay/rust-toolchain@stable` with `components: rustfmt, clippy`
  3. `Swatinem/rust-cache` with `prefix-key: code-quality`
  4. `cargo install cargo-audit` (or use `rustsec/audit-check` action)
  5. `cargo fmt --all -- --check`
  6. `cargo clippy --all-targets -- -D warnings`
  7. `cargo doc --no-deps --document-private-items` with `RUSTDOCFLAGS: -D warnings`
  8. `cargo audit` with `continue-on-error: true` (advisory DB issues make this unreliable as a gate)
- Note: clippy does NOT use `--all-features` because `test-embedder` is a test-only feature

**`test-rust.yml`** — Reusable workflow (`on: workflow_call`)

- Runner: `ubuntu-latest`
- `timeout-minutes: 15`
- Steps:
  1. `actions/checkout@v4`
  2. `dtolnay/rust-toolchain@stable`
  3. `Swatinem/rust-cache` with `prefix-key: test`
  4. `cargo test --locked` (default features only — embedder tier excluded; `--locked` ensures `Cargo.lock` is respected exactly)

**`ci-success.yml`** — Reusable workflow (`on: workflow_call`)

- Single job with a trivial pass step (e.g., `echo "CI passed"`)
- Provides a stable check name for branch protection rules
- The `needs` dependency on code-quality and test-rust is declared in `ci.yml` (the caller), not here

### 2. Local Git Hooks

**`githooks/pre-commit`** — Executable shell script

- Runs the same two fast checks as CI code-quality:
  1. `cargo fmt --all -- --check`
  2. `cargo clippy --all-targets -- -D warnings`
- Fast-fail: exits on first failure for immediate feedback
- Skips `cargo doc` and `cargo audit` (too slow for pre-commit)

**`scripts/install-hooks.sh`** — Hook installer

- Sets `git config core.hooksPath githooks` for this repo
- Opt-in: contributors must run the script; documented in README and CLAUDE.md

### 3. Test Tier Feature Flag

**`Cargo.toml` change:**

```toml
[features]
test-embedder = []
```

**Test migration** — Replace `#[ignore]` with `#[cfg(feature = "test-embedder")]` on 4 tests that download/use the HF model:

| File | Test | Current | After |
|------|------|---------|-------|
| `tests/embedder_test.rs` | `test_embedder_creates_and_loads_model` | `#[ignore]` | `#[cfg(feature = "test-embedder")]` |
| `tests/embedder_test.rs` | `test_embed_single_text` | `#[ignore]` | `#[cfg(feature = "test-embedder")]` |
| `tests/embedder_test.rs` | `test_embed_batch` | `#[ignore]` | `#[cfg(feature = "test-embedder")]` |
| `tests/embedder_test.rs` | `test_similar_texts_have_higher_cosine` | `#[ignore]` | `#[cfg(feature = "test-embedder")]` |

**Tests that remain in default tier (safe for CI):**

- All HNSW tests (`tests/hnsw_test.rs`) — pure data structure tests with synthetic vectors
- `test_index_empty_vault` (`tests/index_test.rs`) — `Embedder::new()` is lazy (sets `model: None`), and an empty vault has zero files to embed, so no model download is triggered
- All preprocessing tests in `tests/embedder_test.rs` (5 tests) — text manipulation only, no model

**Usage:**
- CI: `cargo test --locked` (skips embedder tier)
- Local full suite: `cargo test --features test-embedder`

## Files Changed

| File | Change |
|------|--------|
| `.github/workflows/ci.yml` | New — orchestrator |
| `.github/workflows/code-quality.yml` | New — fmt/clippy/doc/audit |
| `.github/workflows/test-rust.yml` | New — test runner |
| `.github/workflows/ci-success.yml` | New — gate job |
| `githooks/pre-commit` | New — local pre-commit hook |
| `scripts/install-hooks.sh` | New — hook installer |
| `Cargo.toml` | Add `[features] test-embedder = []` |
| `tests/embedder_test.rs` | Replace `#[ignore]` with `#[cfg(feature = "test-embedder")]` on 4 model tests |
