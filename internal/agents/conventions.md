# Branch, Commit & Feature Flag Conventions

> Shared agent guidance — the source of truth for `AGENTS.md` and `CLAUDE.md`.

## Branch and Commit Conventions

These patterns are observed in recent history rather than rigidly enforced. Match the existing style when in doubt.

### Branch naming

`<initials>/<scope>` — current author uses `jct/<scope>` with kebab-case scope. Examples: `jct/wave1-phase3a-dbbackend-foundation`, `jct/post-cloud-only-qol-trivial-trio`. Keep scopes terse but specific enough to disambiguate parallel branches.

### Commit and PR title prefixes

| Prefix | Use for |
|--------|---------|
| `wave N phase X[a]:` or `Wave N Phase X:` | Numbered phases inside a multi-PR feature plan |
| `cloud-only(<scope>):` | Commits in a multi-chunk migration; `<scope>` is the chunk or PR-letter |
| `QoL:` | Polish, ergonomics, dead-code drops, small cleanups |
| `post-PR-<n>:` | Follow-up to review feedback on PR #n that didn't land inline |
| `audit:` | Output of an audit sweep — rationalization comments, threading fixes |
| `fix(<scope>):` / `refactor(<scope>):` / `docs(<scope>):` / `test:` / `chore:` / `mcp:` | Conventional-Commits style for narrow scoped changes |

Self-contained features sometimes use a plain narrative title with no prefix (e.g. "Limb 1 — relationship events + edge projection", "Add offline_access scope and refresh_token grant support"). That's fine when the PR is its own story; reach for a prefix when the change is one beat of a longer arc.

### Bundling fixes into the PR that surfaced them

If a fix's story is "this PR's tests / new code path surfaced a pre-existing bug," bundle it into the same PR rather than extracting. The narrative stays cohesive: one PR, one explanation. Examples in history: PR #69 bundled the empty-body dedup fix into Phase 3a's PR because workspace feature unification first exposed it under that test suite.

Conversely, if the fix is unrelated to the PR's narrative — even if you noticed it while working — extract it. Mixed-narrative PRs are harder to review and harder to revert.

## Feature Flags

Rust crates use feature flags to gate heavy dependencies:
- `test-db` — enables database integration tests (temper-api, tests/e2e)
- `test-embed` — enables embedding tests (temper-ingest)
- `embed` / `extract` — gates ONNX and kreuzberg dependencies (temper-ingest)
- `web-api` — enables utoipa OpenAPI derives (temper-core)
- `typescript` — enables ts-rs type generation (temper-core)
- `mcp` — enables schemars JsonSchema derives for MCP tool parameters (temper-core)
- `artifact-tests` — enables temper-substrate's **scenario write-path** integration tests (bootseed, seed/scenario load + roundtrip + equivalence, charter, content, ledger, replay) plus ONNX. Tests run on ephemeral `public`-schema databases via `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]` — each test gets its own isolated database. CI runs it in its own **Substrate Artifact Tests** job (a distinct feature set, so it cannot fold into the `--workspace` integration run); run locally with **`cargo make test-artifacts`**. temper-substrate's pure core tests (affinity, cluster) are ungated and run in CI.
- `scenario-schema` — enables `schemars::JsonSchema` derives for temper-substrate's **two** JSON-Schema snapshot suites: `tests/scenario_schema.rs` (the scenario YAML model) and `tests/payload_schema.rs` (the **event payload wire contract** — the boot-seed stamps those fixtures into `kb_event_types.payload_schema`, so repo == registry == Rust types). Runs in the **Unit** CI job and via **`cargo make test-schema`** (which `cargo make test` depends on). Regenerate with `UPDATE_SCHEMA=1 cargo make test-schema`.

  > **Run it package-scoped — `-p temper-substrate`, never `--workspace`.** Feature unification changes the emitted schema; `-p` is what the regen emits and what the boot-seed stamps. See [crates/temper-substrate/CLAUDE.md](../../crates/temper-substrate/CLAUDE.md).