# I1: Workspace Restructure & temper-core Extraction — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Transform the monolithic temper-cli into a Cargo workspace with temper-core extracted as the shared vocabulary crate.

**Architecture:** Create a Cargo workspace at the repo root, move the existing CLI into `crates/temper-cli/`, extract shared types (cloud/types/*, error.rs, ids.rs) into `crates/temper-core/`, and create placeholder crates for api, cloud, client, embed, and mcp. The CLI continues to work identically — the binary is still called `temper`.

**Tech Stack:** Rust, Cargo workspaces, sqlx (derive macros only in core), serde, chrono, uuid

---

## File Structure

### New files

```
Cargo.toml                          # Workspace root (replaces current package Cargo.toml)
crates/
├── temper-core/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── error.rs                # Extracted from src/error.rs
│       ├── ids.rs                  # Extracted from src/ids.rs
│       └── types/                  # Migrated from src/cloud/types/
│           ├── mod.rs
│           ├── access.rs
│           ├── auth.rs
│           ├── config.rs
│           ├── conflict.rs
│           ├── device.rs
│           ├── event.rs
│           ├── invitation.rs
│           ├── manifest.rs
│           ├── ownership.rs
│           ├── profile.rs
│           ├── search.rs
│           ├── sync.rs
│           ├── team.rs
│           ├── transfer.rs
│           ├── upload.rs
│           └── vault.rs
├── temper-cli/
│   ├── Cargo.toml
│   └── src/                        # Current src/ moved here
│       ├── main.rs
│       ├── lib.rs                  # Modified: removes cloud module, imports temper-core
│       └── ... (all existing modules)
├── temper-api/
│   ├── Cargo.toml
│   └── src/lib.rs                  # Placeholder
├── temper-cloud/
│   ├── Cargo.toml
│   └── src/lib.rs                  # Placeholder
├── temper-client/
│   ├── Cargo.toml
│   └── src/lib.rs                  # Placeholder
├── temper-embed/
│   ├── Cargo.toml
│   └── src/lib.rs                  # Placeholder
└── temper-mcp/
    ├── Cargo.toml
    └── src/lib.rs                  # Placeholder
```

### Moved files

- `src/*` → `crates/temper-cli/src/*`
- `tests/*` → `crates/temper-cli/tests/*`
- `migrations/` stays at workspace root (shared across crates)

### Removed files (after extraction)

- `crates/temper-cli/src/cloud/` (types moved to temper-core)

---

### Task 1: Create temper-core Crate

**Files:**
- Create: `crates/temper-core/Cargo.toml`
- Create: `crates/temper-core/src/lib.rs`
- Create: `crates/temper-core/src/error.rs`
- Create: `crates/temper-core/src/ids.rs`
- Copy: `src/cloud/types/*` → `crates/temper-core/src/types/*`

This task creates temper-core as a standalone crate that compiles independently. We copy (not move) files so the existing CLI still compiles.

- [ ] **Step 1: Create directory structure**

```bash
mkdir -p crates/temper-core/src/types
```

- [ ] **Step 2: Create temper-core Cargo.toml**

Create `crates/temper-core/Cargo.toml`:

```toml
[package]
name = "temper-core"
version = "0.1.0"
edition = "2021"
description = "Shared types, traits, and models for the temper knowledge base system"

[dependencies]
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
sqlx = { version = "0.8", features = ["chrono", "json", "macros", "postgres", "runtime-tokio-rustls", "uuid"] }
thiserror = "2"
toml = "0.8"
uuid = { version = "1", features = ["v7", "serde"] }
```

- [ ] **Step 3: Create temper-core error module**

Create `crates/temper-core/src/error.rs`:

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TemperError {
    #[error("Vault not found — run `temper init` or set TEMPER_VAULT")]
    VaultNotFound,

    #[error("Config error: {0}")]
    Config(String),

    #[error("Vault error: {0}")]
    Vault(String),

    #[error("Project error: {0}")]
    Project(String),

    #[error("Embedding error: {0}")]
    Embedding(String),

    #[error("Index error: {0}")]
    Index(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),
}

pub type Result<T> = std::result::Result<T, TemperError>;
```

- [ ] **Step 4: Create temper-core ids module**

Create `crates/temper-core/src/ids.rs`:

```rust
use uuid::{timestamp::Timestamp, Uuid};

/// Generate a new UUIDv7 using the current timestamp.
pub fn generate_id() -> String {
    Uuid::now_v7().to_string()
}

/// Generate a UUIDv7 from a date string (YYYY-MM-DD).
/// Falls back to current timestamp if parsing fails.
pub fn generate_id_from_date(date_str: &str) -> String {
    if let Some(ts) = parse_date_to_timestamp(date_str) {
        Uuid::new_v7(ts).to_string()
    } else {
        generate_id()
    }
}

fn parse_date_to_timestamp(date_str: &str) -> Option<Timestamp> {
    let date = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()?;
    let datetime = date.and_hms_opt(0, 0, 0)?;
    let secs = datetime.and_utc().timestamp() as u64;
    Some(Timestamp::from_unix(uuid::NoContext, secs, 0))
}
```

- [ ] **Step 5: Copy cloud types to temper-core**

```bash
cp src/cloud/types/*.rs crates/temper-core/src/types/
```

- [ ] **Step 6: Create temper-core lib.rs**

Create `crates/temper-core/src/lib.rs`:

```rust
//! temper-core — shared types, traits, and models for the temper knowledge base system.
//!
//! This crate is the vocabulary shared by all temper crates: temper-cli, temper-api,
//! temper-client, temper-cloud, temper-embed, and temper-mcp. It contains domain types,
//! error definitions, and ID generation utilities.

pub mod error;
pub mod ids;
pub mod types;
```

- [ ] **Step 7: Verify temper-core compiles independently**

```bash
cd crates/temper-core && cargo check 2>&1 | tail -10
```

Expected: compilation succeeds. If there are import path issues in the types (they use `super::` references), fix them — the module structure is the same so `super::` should work.

- [ ] **Step 8: Run temper-core tests**

```bash
cd crates/temper-core && cargo test 2>&1 | tail -20
```

Expected: all 32 cloud type tests pass.

- [ ] **Step 9: Commit temper-core crate**

```bash
git add crates/temper-core/
git commit -m "feat: create temper-core crate with shared types, error, and ids"
```

---

### Task 2: Convert to Workspace and Move CLI

**Files:**
- Modify: `Cargo.toml` (root — becomes workspace manifest)
- Move: `src/` → `crates/temper-cli/src/`
- Move: `tests/` → `crates/temper-cli/tests/`
- Create: `crates/temper-cli/Cargo.toml`

This task converts the repo to a Cargo workspace and moves the CLI into its crate directory. The old root `Cargo.toml` becomes the workspace manifest.

- [ ] **Step 1: Create crates/temper-cli directory**

```bash
mkdir -p crates/temper-cli
```

- [ ] **Step 2: Move src/ and tests/ to temper-cli**

```bash
git mv src crates/temper-cli/src
git mv tests crates/temper-cli/tests
```

- [ ] **Step 3: Create temper-cli Cargo.toml**

Create `crates/temper-cli/Cargo.toml` with the same dependencies as the current root Cargo.toml, plus temper-core:

```toml
[package]
name = "temper-cli"
version = "0.1.0"
edition = "2021"
description = "Developer workflow tool for agent-assisted development"

[[bin]]
name = "temper"
path = "src/main.rs"

[dependencies]
temper-core = { path = "../temper-core" }
clap = { version = "4", features = ["derive", "env"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
toml = "0.8"
thiserror = "2"
pulldown-cmark = "0.13"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
anstream = "0.6"
anstyle = "1"
chrono = { version = "0.4", features = ["serde"] }
sha2 = "0.10"
dirs = "5"
candle-core = "0.9"
candle-nn = "0.9"
candle-transformers = "0.9"
tokenizers = "0.21"
hf-hub = "0.5"
instant-distance = "0.6"
bincode = "1"
glob = "0.3"
regex-lite = "0.1"
uuid = { version = "1", features = ["v7", "serde"] }
ratatui = "0.29"
crossterm = "0.28"
sqlx = { version = "0.8", features = ["chrono", "json", "macros", "migrate", "postgres", "runtime-tokio-rustls", "uuid"] }
tokio = { version = "1", features = ["rt", "macros", "sync"] }

[features]
test-embedder = []

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 4: Replace root Cargo.toml with workspace manifest**

Replace the entire contents of the root `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["crates/*"]
```

- [ ] **Step 5: Verify the workspace compiles**

```bash
cargo check --all-features 2>&1 | tail -10
```

Expected: compilation succeeds. The CLI sources are now in `crates/temper-cli/src/` and the workspace finds both crates.

- [ ] **Step 6: Commit workspace restructure**

```bash
git add -A
git commit -m "refactor: convert to Cargo workspace, move CLI to crates/temper-cli"
```

---

### Task 3: Wire temper-cli to Use temper-core

**Files:**
- Modify: `crates/temper-cli/src/lib.rs`
- Modify: `crates/temper-cli/src/error.rs`
- Modify: `crates/temper-cli/src/ids.rs`
- Delete: `crates/temper-cli/src/cloud/` (entire directory)

This task replaces the CLI's local copies of error, ids, and cloud types with re-exports from temper-core.

- [ ] **Step 1: Replace CLI error.rs with re-export**

Replace the entire contents of `crates/temper-cli/src/error.rs`:

```rust
pub use temper_core::error::{Result, TemperError};
```

- [ ] **Step 2: Replace CLI ids.rs with re-export**

Replace the entire contents of `crates/temper-cli/src/ids.rs`:

```rust
pub use temper_core::ids::{generate_id, generate_id_from_date};
```

- [ ] **Step 3: Remove cloud module from CLI lib.rs**

In `crates/temper-cli/src/lib.rs`, remove the `pub mod cloud;` line. The final contents should be:

```rust
pub mod actions;
pub mod chunker;
pub mod commands;
pub mod config;
pub mod discovery;
pub mod embedder;
pub mod error;
pub mod format;
pub mod hnsw;
pub mod ids;
pub mod output;
pub mod project;
pub mod registry;
pub mod tui;
pub mod vault;
```

- [ ] **Step 4: Delete the cloud directory from CLI**

```bash
rm -rf crates/temper-cli/src/cloud
```

- [ ] **Step 5: Verify compilation**

```bash
cargo build --all-features 2>&1 | tail -10
```

Expected: compilation succeeds. All `use crate::error::{Result, TemperError}` imports in the CLI resolve through the re-export to temper-core.

- [ ] **Step 6: Run all tests**

```bash
cargo test --all-features 2>&1 | tail -30
```

Expected: all existing CLI tests pass, plus temper-core's 32 type tests pass.

- [ ] **Step 7: Run clippy**

```bash
cargo clippy --all-features -- -D warnings 2>&1 | tail -10
```

Expected: no warnings.

- [ ] **Step 8: Verify the temper binary works**

```bash
cargo run -p temper-cli -- status 2>&1 | head -10
```

Expected: temper status output (same as before).

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor: wire temper-cli to use temper-core for error, ids, and types"
```

---

### Task 4: Create Placeholder Crates

**Files:**
- Create: `crates/temper-api/Cargo.toml`
- Create: `crates/temper-api/src/lib.rs`
- Create: `crates/temper-cloud/Cargo.toml`
- Create: `crates/temper-cloud/src/lib.rs`
- Create: `crates/temper-client/Cargo.toml`
- Create: `crates/temper-client/src/lib.rs`
- Create: `crates/temper-embed/Cargo.toml`
- Create: `crates/temper-embed/src/lib.rs`
- Create: `crates/temper-mcp/Cargo.toml`
- Create: `crates/temper-mcp/src/lib.rs`

Each placeholder crate depends on temper-core and has a doc comment explaining its future purpose.

- [ ] **Step 1: Create temper-api placeholder**

Create `crates/temper-api/Cargo.toml`:

```toml
[package]
name = "temper-api"
version = "0.1.0"
edition = "2021"
description = "Axum HTTP server implementing the temper cloud API"

[dependencies]
temper-core = { path = "../temper-core" }
```

Create `crates/temper-api/src/lib.rs`:

```rust
//! temper-api — Axum HTTP server implementing the temper cloud API.
//!
//! Platform-agnostic: runs locally via `cargo run` or wrapped by temper-cloud
//! for Vercel deployment. Implements the R5 API contract: resources, sync,
//! teams, profiles, transfer, upload, search, events, and auth.
```

- [ ] **Step 2: Create temper-cloud placeholder**

Create `crates/temper-cloud/Cargo.toml`:

```toml
[package]
name = "temper-cloud"
version = "0.1.0"
edition = "2021"
description = "Vercel deployment adapter for temper-api"

[dependencies]
temper-core = { path = "../temper-core" }
```

Create `crates/temper-cloud/src/lib.rs`:

```rust
//! temper-cloud — Thin Vercel adapter wrapping temper-api for serverless deployment.
//!
//! ~100 lines of composition: imports temper-api routes, bridges to vercel_runtime,
//! configures environment-based database and auth settings.
```

- [ ] **Step 3: Create temper-client placeholder**

Create `crates/temper-client/Cargo.toml`:

```toml
[package]
name = "temper-client"
version = "0.1.0"
edition = "2021"
description = "Auth-aware HTTP client for the temper cloud API"

[dependencies]
temper-core = { path = "../temper-core" }
```

Create `crates/temper-client/src/lib.rs`:

```rust
//! temper-client — Auth-aware HTTP client wrapping the temper cloud API.
//!
//! Shared by temper-cli, temper-mcp, and any future client. Handles JWT
//! lifecycle (login, refresh, logout), device identity, and typed methods
//! for every R5 API endpoint.
```

- [ ] **Step 4: Create temper-embed placeholder**

Create `crates/temper-embed/Cargo.toml`:

```toml
[package]
name = "temper-embed"
version = "0.1.0"
edition = "2021"
description = "Embedding and extraction pipeline for temper knowledge base"

[dependencies]
temper-core = { path = "../temper-core" }
```

Create `crates/temper-embed/src/lib.rs`:

```rust
//! temper-embed — Embedding and extraction pipeline.
//!
//! Separate binary with kreuzberg/ONNX for chunking, embedding, and document
//! extraction. Runs as a background worker processing uploads from Cloudflare R2.
//! Heavy dependencies (kreuzberg, ONNX runtime) are isolated here.
```

- [ ] **Step 5: Create temper-mcp placeholder**

Create `crates/temper-mcp/Cargo.toml`:

```toml
[package]
name = "temper-mcp"
version = "0.1.0"
edition = "2021"
description = "MCP server for agent access to the temper knowledge base"

[dependencies]
temper-core = { path = "../temper-core" }
```

Create `crates/temper-mcp/src/lib.rs`:

```rust
//! temper-mcp — MCP (Model Context Protocol) server for agent workflows.
//!
//! Exposes the temper knowledge base to LLM agents via Claude Desktop,
//! Claude Code, and other MCP-compatible clients. Uses temper-client
//! for API communication.
```

- [ ] **Step 6: Verify all crates compile**

```bash
cargo check --all-features 2>&1 | tail -10
```

Expected: all 7 crates compile (temper-core, temper-cli, and 5 placeholders).

- [ ] **Step 7: Commit**

```bash
git add crates/temper-api crates/temper-cloud crates/temper-client crates/temper-embed crates/temper-mcp
git commit -m "feat: add placeholder crates for api, cloud, client, embed, mcp"
```

---

### Task 5: Final Verification and Cleanup

**Files:**
- Possibly modify: various files for clippy/fmt fixes

- [ ] **Step 1: Run the full test suite**

```bash
cargo test --workspace --all-features 2>&1 | tail -30
```

Expected: all tests pass (existing CLI tests + 32 temper-core type tests).

- [ ] **Step 2: Run clippy across workspace**

```bash
cargo clippy --workspace --all-features -- -D warnings 2>&1 | tail -10
```

Expected: no warnings.

- [ ] **Step 3: Check formatting**

```bash
cargo fmt --all --check 2>&1
```

Expected: no formatting issues. If any, run `cargo fmt --all` and commit.

- [ ] **Step 4: Verify temper binary is installed correctly**

```bash
cargo install --path crates/temper-cli --force 2>&1 | tail -5
temper status 2>&1 | head -10
```

Expected: temper installs and runs identically to before the restructure.

- [ ] **Step 5: Verify workspace member list**

```bash
cargo metadata --no-deps --format-version 1 2>/dev/null | python3 -c "import sys,json; d=json.load(sys.stdin); print('\n'.join(p['name'] for p in d['packages']))"
```

Expected output (7 crates):
```
temper-api
temper-cli
temper-client
temper-cloud
temper-core
temper-embed
temper-mcp
```

- [ ] **Step 6: Commit any cleanup**

If any fixes were needed:

```bash
git add -A
git commit -m "chore: workspace cleanup — fmt, clippy fixes"
```
