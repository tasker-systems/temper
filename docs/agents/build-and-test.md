# Build & Test Commands

> Shared agent guidance — the source of truth for `AGENTS.md` and `CLAUDE.md`.

All commands use **cargo-make** (install: `cargo install cargo-make`). Rust tests use **cargo-nextest** (install: `cargo install cargo-nextest`).

```bash
# Quality checks (Rust fmt + clippy + docs + machete, TS typecheck + biome)
cargo make check

# Auto-fix formatting and lint
cargo make fix

# Unit tests (no database needed)
cargo make test

# Integration tests (requires Docker Postgres running)
cargo make docker-up
cargo make test-db

# E2E tests (CLI ↔ API ↔ DB through real Axum + Postgres; lives at tests/e2e/, not crates/)
cargo make test-e2e

# All tests (Rust + TypeScript + integration)
cargo make test-all

# TypeScript tests only
cargo make ts-test

# Build everything
cargo make build

# Run API server locally
cargo make run

# Generate TypeScript types from Rust structs
cargo make generate-ts-types

# Regenerate openapi.json AND the temper-rb gem AND temper-ts's schema.ts (all products of the router)
cargo make openapi
```

> **`cargo make openapi`, `cargo make generate-ts-types`, and `temper skill emit` each restale
> committed artifacts, and `cargo make check` gates every one of them.** Read the
> `generated-artifacts` skill before changing a response DTO, a route, a ts-rs-derived type, or a
> shared skill template under `crates/temper-cli/templates/shared/`.

## Running a single Rust test
```bash
cargo nextest run --workspace test_name
cargo nextest run --workspace -E 'test(test_name)'        # exact filter
cargo nextest run -p temper-api --features test-db test_name  # specific crate with features
```

> **Gotcha:** a bare `cargo nextest run -p temper-api` (no test filter) **hangs** at test-list enumeration — nextest lists the `temper-api` **bin** target, whose `main()` ignores `--list` and blocks (the slow-timeout doesn't cover the list step). Always scope to the integration test target(s): `cargo nextest run -p temper-api --features test-db --test relationship_handler_test`. Also export `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development` for `#[sqlx::test]` under bare `cargo` (the `cargo make` tasks set it for you).

## Embed-gated e2e tests
`cargo make test-e2e` only enables `--features test-db`, so it **silently compiles out every `test-embed`-gated test**. CI does not: **every CI test job enables `test-embed`**, and ONNX is installed in all of them. When touching push-body, ingest-pipeline, or YAML fixture loading code, run with both features locally to match CI:
```bash
cargo make test-e2e-embed
```

> **Never add a `-E 'binary(...)'` filter to a CI test job.** Selection is `--workspace` so a new crate or test is picked up with no CI edit. A filter that makes CI green is hiding a test, not fixing one. CI jobs are split by **intention** (what they need from the environment), never by feature flag — see [.github/workflows/CLAUDE.md](../../.github/workflows/CLAUDE.md).

## TypeScript & UI checks

> **`cargo make check` does NOT cover temper-ui.** Its TypeScript step runs `tsc` on temper-cloud, not
> `svelte-check` on temper-ui. So a change to a **generated shared type** (`cargo make generate-ts-types`
> → `src/lib/types/generated/*.ts`) that restales a UI fixture — e.g. adding a required field to
> `ResourceRow`, which then breaks a hand-built `makeRow` test helper — passes `cargo make check` and
> fails only in CI's UI job. After any shared-type change, run `cd packages/temper-ui && bun run check`
> yourself. (If it reds on `d3-*` "implicit any" / "cannot find package" in `graph/atlas/layout/*`,
> that is a stale local `node_modules`, not your change — `bun install` first; CI installs fresh. See
> [[project_ci_flake_signatures]].)