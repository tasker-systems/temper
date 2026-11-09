# Code Quality

The engineering baseline (lints, errors, unsafe, auth before writes, query scoping, dependencies) is
the projected [tasker-systems-conventions](../../.claude/skills/tasker-systems-conventions/SKILL.md)
skill; it states each rule's status here. The review lens for this repository is
[internal/development/code-quality-best-practices.md](../development/code-quality-best-practices.md).
What follows is temper's own.

- **Writes come in two families; reads stay service-direct.** Resource writes go through
  `DbBackend`, the server's command layer: a handler builds a backend per request and runs one
  operations command per call. Governance and estate acts (team, context, grant, connection,
  machine, reassign, blob, subscription, erasure) are service-direct writes through
  `temper-services`. In neither family does a handler call `temper-substrate` write persistence
  or inline `sqlx::query!()`; MCP tools and the CLI reach the handlers over HTTP. *Why:* each act
  has one write path, which is what keeps the doors admitting the same things. *(convention)*
- **Repeated multi-table joins become a SQL view**, queried with plain `WHERE`/`ORDER BY`/`LIMIT`
  (incumbents: `kb_resource_workflow_props`, `kb_resource_properties`, `kb_owner_properties`).
  *Why:* copies drift. *(convention)*
- **Cross-runtime wire types live in `temper-core` with `ts-rs` derives**, never a hand-written zod
  mirror. *(gated: `ts-rs-drift`)*
- **Typed structs over `serde_json::json!()` for known shapes.** *Why:* the compiler catches what
  serialization accepts. *(convention)*
- **A params struct past seven parameters**, the line clippy's `too_many_arguments` holds. Below
  it, two same-typed arguments that could transpose take id newtypes (`ProfileId`, `ResourceId`),
  not a params struct. *(gated: `code-quality.yml#rust-quality`, clippy under `-D warnings`; an
  `#[expect]` opts a function out)*
- **TypeScript logs through pino** (`packages/temper-cloud/src/logger.ts`) with field objects, never
  `console.log`. *(convention)*
- **A write command carries `#[act_span]`; never record act ids onto the root span.** Act ids arrive
  in the body, after the root span exists, and recording onto "the current span" silently lands on
  the root until the first nested span appears. *(gated: `act_span_declares_every_act_field`,
  e2e `logging_test.rs`)* See `internal/development/span-field-conventions.md`.
- **Schema-required frontmatter is populated at write time.** Create and update share one
  managed-tier pipeline, `DbBackend`'s `validate_managed_meta_pipeline`: strip echoed system keys,
  apply doc-type defaults (`apply_defaults_value`), validate against the doc-type schema. Identity
  keys come from typed columns (`assemble_frontmatter_document`), never from the payload.
  *(convention: a write that misses a required field is refused at runtime by
  `validate_managed_meta`; nothing checks that a new write path uses the pipeline)*

## SQL

`query!` macros are checked at compile time; after changing SQL or a migration, regenerate the
`.sqlx` caches — read the `sqlx-query-cache` skill first. *(gated: the offline build)*

**`error[E0282]: type annotations needed` on a `query!` you did not touch means the dev database is
behind `migrations/`, not a stale cache.** A bare `cargo` build reads `DATABASE_URL` from `.env`
and checks against the live database; `cargo make` sets `SQLX_OFFLINE=true` and is immune. Fix with
`cargo make db-migrate`.
