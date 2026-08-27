# Contributing to Temper

Thanks for your interest in Temper. This guide covers the development setup, the testing tiers, and
what we look for in a pull request.

## Before you start

- **Security issues do not go here.** If you have found a vulnerability, read
  [SECURITY.md](SECURITY.md) and use one of the private channels. Do not open an issue, a discussion,
  or a PR.
- **Open a discussion first for anything large.** Temper is pre-1.0 and several subsystems are
  mid-arc. A quick conversation before you build saves you from landing on top of work already in
  flight.

## Getting started

### Prerequisites

- **Rust** stable, with `cargo-make` (`cargo install cargo-make`) and `cargo-nextest`
  (`cargo install cargo-nextest`)
- **Docker** — for the local PostgreSQL instance
- **Node** and **Bun** — for the TypeScript packages and the web UI

### Setup

```bash
git clone https://github.com/tasker-systems/temper.git
cd temper

# PostgreSQL on port 5437 — deliberately not 5432, to avoid colliding with other projects
cargo make docker-up

export DATABASE_URL="postgresql://temper:temper@localhost:5437/temper_development"
cargo make db-migrate

cargo make check    # fmt + clippy + docs + machete, TS typecheck + biome
cargo make test     # unit tests, no database needed
```

Install the pre-commit hook with `bash scripts/install-hooks.sh`. It formats staged Rust files and
runs a subset of the CI gates. It is **not** a substitute for `cargo make check` — see *Gates the
hook does not run*, below.

## Development workflow

### Branches and commits

Branches are `<initials>/<scope>` in kebab-case — `jct/refresh-chain-owner`,
`jct/query-door-stage-cap`. Keep the scope specific enough to tell two parallel branches apart.

Commit and PR titles use a prefix where the change is one beat of a longer arc, and a plain narrative
title where the PR is its own story. Common prefixes: `fix(scope):`, `refactor(scope):`,
`docs(scope):`, `test:`, `chore:`, `audit:`, `QoL:`.

**If a change is security-relevant, read [SECURITY.md](SECURITY.md) § *How this project handles
security work internally* before writing the title or body.** Titles land in auto-generated public
release notes, stripped of their context. Say what the change establishes going forward; leave the
account of the prior state out of the public record.

### Testing tiers

Each tier skips things the one below it covers, so a green run at one tier is not a green run at all
of them.

| Command | Covers | Needs |
|---|---|---|
| `cargo make test` | unit tests | nothing |
| `cargo make test-db` | integration | Docker Postgres up |
| `cargo make test-e2e` | CLI ↔ API ↔ DB through real Axum + Postgres | Docker Postgres up |
| `cargo make test-all` | Rust + TypeScript + integration | Docker Postgres up |
| `cargo make ts-test` | TypeScript only | — |

Two things that will bite you:

- **`cargo make test-e2e` compiles out every `test-embed`-gated test.** CI does not. If you touch
  push-body, ingest-pipeline, or YAML fixture loading, run `cargo make test-e2e-embed` to match CI.
- **A bare `cargo nextest run -p temper-api` hangs** at test-list enumeration. Scope to the test
  target: `cargo nextest run -p temper-api --features test-db --test <target>`.

### Generated artifacts

Several committed files are generated, and `cargo make check` gates every one of them. If you change
a response DTO, a route, a ts-rs-derived type, or a shared skill template, regenerate and commit the
result:

```bash
cargo make openapi             # openapi.json + all three SDKs (temper-rb, temper-ts, temper-py)
cargo make generate-ts-types   # ts-rs TypeScript type trees
```

Commit **all** changed codegen output, not just the file you were aiming at.

### SQL

Queries use `sqlx` macros checked against the real schema at compile time. After changing SQL or a
migration:

```bash
cargo sqlx prepare --workspace -- --all-features
git add .sqlx/
```

**Migrations are immutable once applied.** Editing one — even a comment — changes its checksum and
breaks `db-migrate` for everyone who has already run it. Add a new migration instead.

### Gates the hook does not run

The pre-commit hook is narrower than CI, and two gaps catch people out: it reads the **working tree**
rather than the index, and it does not run every drift check. Before pushing, run `cargo make check`
against what you actually staged.

### Code standards

- `#[expect(lint, reason = "...")]` rather than `#[allow]`
- All public types implement `Debug`
- All MPSC channels are bounded
- `--all-features` for builds and clippy
- Authorization checks go at the **top** of any function that mutates, before any write. A gate after
  the mutation leaves orphaned data on refusal.
- Any query touching user data is scoped through the profile-aware predicates. This holds in
  background paths too — the profile flows through the whole call chain.

## Where documentation goes

This one matters, and it is easy to get wrong.

- **`docs/` is public and synced to the documentation site.** Everything in it is published. Nothing
  else lives there.
- **`internal/` is engineering material** — specs, plans, reviews, audits, decision records.

`internal/` is **not documentation, but it is still visible**: this is a public repository, and
`internal/` is world-readable. So the test for a document is not "is this internal?" but *does this
describe a settled choice or an open one?* A decision we hold deliberately can be written down. A
weakness we intend to fix belongs with the work, written once the work has landed and describes
something closed.

Specs and plans are not kept in this repository — they live in the private
`tasker-systems/temper-artifacts` repo, under `specs/` and `plans/`. Decision records
do stay here, in `internal/decisions/` — see that directory's README for the
conventions.

## Submitting changes

1. Branch from `main`
2. Keep it focused — one logical change per PR
3. `cargo make check` and the relevant test tier pass
4. Update docs if you changed public behaviour
5. Open a PR against `main`

A PR is required for every merge, and `CI Success` must be green. Note that `CI Success` is
**scope-aware**: it validates each job against a computed scope, so jobs outside the scope of your
diff will show as skipped and the gate will still pass. That is intended.

### What we look for

- **Correctness, with a test that would have failed before the change.** For a bug fix, a regression
  test; for a feature, a test that fails against the prior state.
- **Follows the patterns already in the file.** Read the file you are changing and a sibling in the
  same module before you write.
- **No unnecessary abstraction.** Extract when two implementations would drift apart. Otherwise
  inline it.
- **Honest completion.** If part of the change did not land, say so in the PR rather than leaving it
  to review to discover.

## Questions

- [Discussions](https://github.com/tasker-systems/temper/discussions) for questions
- [Issues](https://github.com/tasker-systems/temper/issues) for bugs and feature requests — **not for
  security reports**, see [SECURITY.md](SECURITY.md)
