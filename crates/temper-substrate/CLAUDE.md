# temper-substrate

Persistence write/readback core (`writes`/`readback`) plus the cognitive-map / telos-lens
region producer and the YAML scenario DSL. Pulls `temper-ingest(embed)` unconditionally,
so every crate depending on it links ort.

## `scenario-schema` — run it package-scoped, `-p temper-substrate`, never `--workspace`

The `scenario-schema` feature enables `schemars::JsonSchema` derives for the **two**
JSON-Schema snapshot suites: `tests/scenario_schema.rs` (the scenario YAML model) and
`tests/payload_schema.rs` (the **event payload wire contract** — the boot-seed stamps
those fixtures into `kb_event_types.payload_schema`, so repo == registry == Rust types).
Runs in the **Unit** CI job and via **`cargo make test-schema`** (which `cargo make test`
depends on). Regenerate with `UPDATE_SCHEMA=1 cargo make test-schema`.

The emitted schema depends on **feature unification**: under `--workspace`, temper-core's
`mcp` feature (schemars derives) unifies in and the id newtypes emit **inline**; under
`-p temper-substrate` they emit as `$ref`s into `$defs`. Same structs, two different
schemas, decided by the cargo invocation. `-p` is authoritative because it is what the
regen emits and what the boot-seed stamps — gating the workspace shape would gate a schema
nothing ever writes. This is the one place the "selection is `--workspace`" rule does
**not** apply, and it is not an exception to *"CI runs everything by intention"*: the
intention (no DB, no ONNX) is why it lives in the Unit job; the scoping is about matching
the producer.

This feature was wired into **no job and no task** until 2026-07-16, so all four snapshot
tests ran **nowhere** and sat **red on `main`** — the same rot as `streaming_ingest_test`,
via a feature flag instead of an `-E` filter. It was not cosmetic: `segmented`,
`telos_centroid`, and `TelosConstants` shipped with no schema, and prod's `kb_event_types`
still describes an older payload than the code writes (re-stamping prod is its own task).
**A test no job enables is a test that runs nowhere.**

## `artifact-tests`

Enables the **scenario write-path** integration tests (bootseed, seed/scenario load +
roundtrip + equivalence, charter, content, ledger, replay) plus ONNX. Tests run on
ephemeral `public`-schema databases via
`#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]` — each test gets its own isolated
database. CI runs it in its own **Substrate Artifact Tests** job (a distinct feature set,
so it cannot fold into the `--workspace` integration run); run locally with
**`cargo make test-artifacts`**. The pure core tests (affinity, cluster) are ungated and
run in CI.
