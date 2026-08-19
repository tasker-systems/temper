# P3 — Thread a caller-supplied `correlation_id` into the event ledger

Task `019f4912-2f31-7740-a075-1720845005bd` · Goal `019f4910` (temper-rb) · build/medium
Branch `jct/p3-correlation-over-the-wire`

## Plan/reality corrections (verified before writing this plan)

The task description names things that do not exist. Corrected here so no step inherits the error.

| Task text | Reality |
|---|---|
| "`emit_event` … `migrations/20260624000002_canonical_functions.sql:769`" | The function is **`_event_append`**. There is no `emit_event` anywhere in the repo. Line 769 is right. |
| "The SQL is ready." | Only the *sink* is ready. `_event_append` accepts `p_correlation`, but **no call site anywhere passes it**, and none of the 14 mutation functions the Rust layer actually calls accept a correlation parameter at all. The migration below is the bulk of this task. |
| "Out to `emit_event(p_correlation)`" — one hop | Two hops: `fire_with` → `<mutation_fn>(…, p_correlation)` → `_event_append(p_correlation => …)`. |

Two further facts, verified:

- **Nothing reads `kb_events.correlation_id`.** No query in `temper-services/src/services/` or
  `temper-substrate/src/readback.rs` touches it. So there is no existing grouping semantics to
  collide with, and the "self-roots when unsupplied" acceptance criterion is already true today
  (`COALESCE(p_correlation, v_ev)` with `p_correlation` always NULL).
- **`relationship_events.rs:5-6` is false.** It claims "the projection builder keys on
  `correlation_id`, not on ledger `references`." `_project_relationship_asserted` keys on
  `edge_id` read out of the payload. Fixed in this PR (it rides the PR that surfaced it).

## Decisions

- **`CorrelationId` newtype, not bare `Uuid`.** The task says `Option<Uuid>`. Deviating: `ids.rs`
  has a `define_id!` macro giving transparent serde (wire shape is an unchanged plain UUID string),
  `utoipa`/`ts-rs`/`schemars(inline)` derives, and a `sqlx::Type` impl — one block, all of it. The
  point is that `invocation` and `correlation` are both `Option<Uuid>` otherwise and trivially
  swappable at a call site, which is precisely the confusion the task's "Not `invocation_id`"
  paragraph warns about. A newtype makes that swap a compile error.
- **Append `p_correlation` last in every signature.** Existing positional call sites
  (`facet_set($1,$2)`, the `fire()` default path) stay byte-identical.
- **Segmented ingest is out of scope.** `SegmentedBeginResponse.correlation_id` stays a
  server-minted, unthreaded value. Threading it forces a precedence rule against a caller-supplied
  correlation; that is its own decision. Filed as a follow-up task.
- **All three surfaces.** API (via `ActInput`), MCP (free — `ActInput` is `#[serde(flatten)]`ed onto
  the tool inputs), CLI (`--correlation` on the shared `ActArgs`, covering all five call sites).

## Steps

### 1. SQL migration — `migrations/20260709000050_act_correlation_passthrough.sql`

Same shape as `20260629000003_nonauthored_act_correlation.sql`: `DROP FUNCTION` + `CREATE FUNCTION`
per function (adding a parameter changes identity, so `CREATE OR REPLACE` would leave a second,
ambiguous overload). Each body copied verbatim from its **current** definition, with exactly two
changes: the signature gains `p_correlation uuid DEFAULT NULL` **last**, and the single
`_event_append(…)` call forwards `p_correlation => p_correlation`.

Current definition of each of the 14 (grep-verified — copy from *these*, not from birth):

| Function | Current definition |
|---|---|
| `resource_create` | `20260624000002:747` |
| `relationship_assert` | `20260624000002:823` |
| `relationship_fold` | `20260624000002:852` |
| `facet_set` | `20260624000002:889` |
| `resource_delete` | `20260629000003:25` |
| `resource_update` | `20260629000003:42` |
| `resource_rehome` | `20260629000003:59` |
| `property_set` | `20260629000003:73` |
| `relationship_retype` | `20260629000003:92` |
| `relationship_reweight` | `20260629000003:108` |
| `block_mutate` | `20260629000003:124` |
| `cogmap_charter_set` | `20260629000003:153` |
| `resource_reassign` | `20260703140000:26` |
| `block_append` | `20260708000012:31` |

`DROP` safety: no view, trigger, or SQL function calls any of the 14 at runtime. The only
in-SQL callers (`relationship_assert`, `relationship_fold`, from
`20260709000005_backfill_goal_parent_of_to_advances.sql`) are inside a one-shot backfill that has
already run and is ordered before this migration. Plain `DROP` (no `CASCADE`) succeeds.

Portability: no new UUID minting, so the `uuid_generate_v7()`-not-`uuidv7()` trap does not apply
here — but do not introduce one.

### 2. `temper-core` — the wire type

- `ids.rs`: `define_id!(CorrelationId)`.
- `authorship.rs`: `ActContext.correlation: Option<CorrelationId>`;
  `ActInput.correlation_id: Option<CorrelationId>`; thread through `into_act_context`,
  `From<ActContext> for ActInput`, and `is_empty()`.
- Unit tests: correlation-only assembles a non-empty context with no authorship; the
  `ActContext ↔ ActInput` roundtrip preserves it; `is_empty()` is false with correlation alone.

### 3. `temper-substrate` — the sink

- `ids.rs`: re-export `CorrelationId`.
- `events.rs`: `EventContext.correlation: Option<CorrelationId>` + `correlation_uuid()`; `fire_with`
  binds `ctx_corr` into all **14** ctx-threading arms (each `query_scalar!` gains one `$n`).
  The four non-ctx arms (`facet_set($1,$2)` legacy, `lens_create`, `region_materialize`,
  `invocation_open`/`invocation_close`) are untouched.
- `writes.rs`: `EventContext::default()` call sites need no change.

### 4. `temper-services` — the mapper

Nine `EventContext { … }` construction sites in `backend/db_backend.rs` map `act.correlation`
through. `reassign_service.rs` uses `EventContext::default()` — unchanged.

### 5. Surfaces

- **API** — `ActInput` flows through existing `params(ActInput)` / body DTOs. No handler change.
- **MCP** — `act: ActInput` is `#[serde(flatten)]`ed; the field appears automatically.
- **CLI** — `ActArgs` (`cli.rs:51`) gains `--correlation <uuid>`, parsed trailing-UUID-only via
  `parse_ref` exactly as `--invocation` is.
- **temper-client** — carries the field on the request shapes it already builds from `ActInput`.

### 6. Tests (TDD — red first)

- **`temper-services/tests/act_correlation_test.rs`** (new, `#[cfg(feature = "test-db")]`, runs
  locally): the load-bearing proof, at the real `DbBackend` caller. Modeled on
  `segmented_backend_test.rs` (ONNX-free, bring-your-own chunks).
  1. Two writes issued with the same `correlation` share one `kb_events.correlation_id`.
  2. A write with no `correlation` self-roots (`correlation_id = id`).
  3. Correlation and invocation are independent — a correlation with no invocation writes
     `correlation_id` and leaves `invocation_id` NULL.
- **`temper-substrate/tests/nonauthored_act_correlation.rs`** — extend the existing `stamped()`
  context to carry a correlation and assert the column. `artifact-tests`-gated (ONNX): runs in the
  Embed CI job, not locally.

### 7. Regeneration + gates

In order:
1. `cargo make generate-ts-types`
2. `bash .github/scripts/check-openapi-spec.sh` (regenerate, then confirm clean) and
   `check-openapi-routes.sh` — both are `code-quality.yml` gates.
3. `cargo sqlx prepare --workspace -- --all-features` → `cargo make prepare-services` →
   `cargo make prepare-api` → `cargo make prepare-e2e`. Every `query_scalar!` in `events.rs`
   changed its arg count, so the workspace cache *must* be regenerated or the offline build fails.
4. `cargo make check`, `test`, `test-db`, `test-e2e`.

### 8. Bundled fixes (ride the PR that surfaced them)

- `relationship_events.rs:5-6` — delete the false claim about the projection keying on
  `correlation_id`.
- `SegmentedBeginResponse.correlation_id` doc — point at the follow-up task rather than "Beat 1".

## Acceptance criteria (from the task)

- [ ] Two writes issued with the same `correlation` share one `kb_events.correlation_id`.
- [ ] A write with no `correlation` self-roots, unchanged from today.
- [ ] Correlation is a correlation aid, never authorization — no gate keys off it.
- [ ] `cargo sqlx prepare` ritual run.
