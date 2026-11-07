# Build and Test

Everything runs through cargo-make; Rust tests use cargo-nextest. Both are pinned in `mise.toml`.
`cargo make --list-all-steps` is the full menu.

```bash
cargo make check            # fmt, clippy, docs, machete, drift gates, TS typecheck + biome
cargo make fix              # auto-fix formatting and lint
cargo make test             # unit tests, no database
cargo make docker-up        # start Postgres on 5437 and apply pending migrations
cargo make test-db          # DB integration tests
cargo make test-e2e         # CLI ↔ API ↔ DB (see the test-embed trap in conventions.md)
cargo make test-all         # Rust + TypeScript + integration
cargo make openapi          # regenerate openapi.json and all three SDKs
cargo make generate-ts-types
```

**Generated artifacts:** `openapi`, `generate-ts-types` and `temper skill emit` each restale
committed files that `cargo make check` gates. Read the `generated-artifacts` skill before changing
a response DTO, a route, a ts-rs type, or a shared skill template.

## Narrow runs

```bash
cargo nextest run -p temper-api --features test-db <test_name>
cargo nextest run -p temper-api --features test-db --test <target>
```

Scope to the crate; `--workspace` compiles every crate's tests to run one. Under bare `cargo`,
export `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`.

## Things that look like your bug and are not

- **Editing an already-applied migration** fails every DB test with "previously applied but has
  been modified" once the test template holds it: `cargo make test-template`. The template is
  opt-in locally (`TEMPER_TEST_TEMPLATE=true` in `.env`); CI always uses it.
- **`cargo make check` does not run `svelte-check`.** After a shared-type change, run
  `cd packages/temper-ui && bun run check`. A red on `d3-*` types there is a stale `node_modules`
  (`bun install`).
- **Never add a `-E 'binary(...)'` filter to a CI test job** — a filter that makes CI green is
  hiding a test. See `.github/workflows/CLAUDE.md`.
