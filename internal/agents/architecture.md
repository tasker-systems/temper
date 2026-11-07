# Architecture

Temper is a knowledge base for agent-assisted development: goals, tasks, sessions, research and
decisions persist across conversations in a cloud store with semantic search. The `temper` CLI,
the MCP server and the web UI are its doors.

## Layout

A Rust workspace (`crates/`, members in the root `Cargo.toml`) and a Bun workspace (`packages/`),
deployed to Vercel through the adapters in `api/`. `tests/e2e/` is a standalone crate that drives
CLI ↔ API ↔ Postgres for real. Each crate's role is in its `Cargo.toml` description and crate docs;
the facts below are the ones a reader would otherwise get wrong.

## What is not obvious

- **Layering.** `temper-api` and `temper-mcp-server` are transport only. Business logic and SQL live
  in `temper-services`, the persistence core in `temper-substrate`, domain operations and
  `parse_ref` in `temper-workflow`, shared and wire types in `temper-core`. Neither surface depends
  on the other. *Why:* the two doors must admit the same callers. *(convention)*
- **temper-mcp holds no database.** Every tool relays to the deployed API through `relay_client`,
  so MCP cannot drift from the API's authorization. *(gated: `temper-mcp-server/tests/no_database_test.rs`)*
- **The CLI embeds; the server embeds only when chunks are absent.** Client-supplied chunks ride
  through verbatim. Both sides must use the same model — see `crates/temper-ingest/CLAUDE.md`.
  `temper-substrate` links ort unconditionally, so every crate above it does.
- **temper-migrate's dependency list is the point of the crate.** The deploy applies schema before
  building the API, so anything added to this crate lands on the deploy's critical path. After
  adding or editing a migration, `cargo clean -p temper-migrate`: `sqlx::migrate!` embeds
  `migrations/` at compile time and cargo's tracking of it is unreliable.
- **Release ≠ deploy.** A `v*` tag builds CLI binaries and a GitHub Release; it deploys nothing.
  Each site is its own Vercel project on its own Neon database. `main` auto-deploys safely because
  the build runs `temper-migrate --additive-only`, which fails the deploy at the first migration
  not declared additive. *(gated: the deploy build itself)* See `DEPLOYING.md`.
- **`api/internal.rs` is a separate function only for its longer `maxDuration`** (the embed crons).
  `create_app` mounts the same routes, so single-process deploys serve everything.
- **Postgres is 18 locally and in CI, 17 on Neon.** Migrations must be version-portable.

## Telemetry

- **The OTLP exporter must use reqwest's blocking client.** The batch processor exports from a
  thread with no Tokio reactor, so the async client drops every span silently.
  *(gated: `temper-telemetry/tests/live_export_client.rs`)*
- **The CLI exports only with `TEMPER_CLI_TRACE=true` and an OTLP endpoint**, because the endpoint
  variable is often set for another project. Filtering is per layer.
  *(gated: `temper-telemetry/tests/cli_export_filter.rs`)*
- Inbound trace context is never a parent; a caller that passed authentication is joined by span
  link. See `docs/playbooks/send-traces-to-an-otlp-backend.md`.
