# PR B — the door opens: `POST /api/query`, then `temper query`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`)
> syntax for tracking.

**Goal:** Publish the composition contract as a route, then as a command. Everything before this
beat built a door that nothing could knock on.

**Spec:** [`docs/superpowers/specs/2026-08-12-api-query-door-design.md`](../specs/2026-08-12-api-query-door-design.md) §"B — the door" — read it first. **Two of its passages were corrected on 2026-08-13 after A2 shipped**; the corrections are in place, but read the `[corrected — …]` notes rather than the sentences they replaced.

**Tech Stack:** Rust (axum, utoipa, serde, reqwest, clap), cargo-nextest. No migration, no `.sqlx`
regeneration — B adds no SQL. It regenerates **three** committed artifact trees; see Global
Constraints.

**Baseline:** grounded against `0a812703` (main, with A2 merged).

## The cut: B is TWO PRs

`[decided — 2026-08-13, Pete]`, from a three-option prompt carrying the touch list for each. B was
ratified as one PR; the grounding pass below found two scope items its spec section does not
mention, and the second is load-bearing rather than cosmetic.

- **B1 — the API door.** Server-complete and independently shippable: the error contract, the route,
  the handler, the ~51-type contract landing, the route e2e.
- **B2 — the CLI door.** `temper-client`'s query module and its 400 arm, `temper query`, the CLI e2e.

**The rejected alternative was a three-way split** hoisting the error-contract widening into its own
PR. Declined because the new `ApiError` arm would then have **no emitter until the next PR** — a seam
with nothing on either side of it, which this repo treats as a smell rather than as caution.

## Decisions this plan takes

**This block is the contract between the plan and whoever ratified it. A decision that is not listed
here was not taken — if implementation needs one, it stops and asks rather than recording it in a
doc comment, a commit message or a test name.** That rule is not decoration on this arc: ⟨7⟩ exists
because a placement nobody ruled hardened into a test name and steered three sessions, and
`.github/scripts/audit-unattributed-decisions.sh` is the guard built from that specimen.

**The closing summary of each PR must reproduce this table verbatim.**

| # | Decision | Rests on | Status |
|---|---|---|---|
| 1 | B is cut into **B1 (API) + B2 (CLI)** | Pete, three-option prompt carrying each touch list | **decided** `[2026-08-13, Pete]` |
| 2 | `ErrorDetail.details` becomes a `oneOf`; a new `ApiError` arm carries `Vec<PlanRefusal>` | Pete, re-confirmed in prose after being flagged — spec §B | **decided** `[2026-08-13, Pete]` |
| 3 | `POST /api/query` sits in `gated_routes()` — `require_auth` + `require_system_access` | Spec §B; every content-touching route does, with two whole-project exceptions | **decided**, spec |
| 4 | `docs/api/query.openapi.yaml` is still **not** edited; **D** owns it | Standing ruling, held by A and A2 | **decided**, standing |
| 5 | The handler calls `prepare`, and does **not** assemble the pipeline itself | A2 decision 9 — the order has one home | **derived** from A2 |
| 6 | The client gains a 400 arm that **preserves the refusal list** | Follows from ⟨3⟩'s *"every refusal, not the first"*, which is otherwise API-only | **derived**, argued at Task B2.1 |

**Nothing here is OPEN.** If execution turns up a decision this table does not carry, that is the
signal to **stop and ask**.

## Global Constraints

- **Never scope a test with `--workspace`** — it hangs on bin-target enumeration. Always
  `-p <crate>`, prefer `--test <target>`.
- **Do not pipe test output through `tee`** — it reports tee's exit code, so a red gate looks green.
- **`cargo make check` must pass**, and on this PR it gates **three** generated trees at once:
  `openapi.json`, the temper-rb gem, and temper-ts's `schema.ts`. Read the `generated-artifacts`
  skill before touching a response DTO or a route. Regenerate with `cargo make openapi`.
- **`cargo make check` does NOT cover temper-ui.** Run `cd packages/temper-ui && bun run check`
  after any shared-type change. (The pre-commit hook does run `svelte-check`; `cargo make check`
  does not.)
- **The find-about e2e is `test-embed`-gated.** A run scoped `--features test-db` alone compiles it
  to nothing and reads green — trap 2. Use `cargo make test-e2e-embed`.
- **A `--test <target>` run says nothing about the same crate's `--lib`.** A2 was handed a branch
  with 11 of 12 `temper-services --lib` tests failing under a baseline that was true about
  everything it had actually run. Run both.

---

# B1 — the API door

### Task B1.1: `ErrorDetail.details` becomes a `oneOf`

**Files:** Modify `crates/temper-services/src/error.rs`.

**Tag: AMEND.** The disk thing being amended states its own amendment condition, which is why this
is AMEND and not EXTEND (`error.rs:74-87`):

```rust
#[derive(Serialize, ToSchema)]
pub struct ErrorDetail {
    code: &'static str,
    message: String,
    /// Present only on `SYSTEM_ACCESS_REQUIRED`, where it carries the typed refusal; absent on
    /// every other error.
    // Held as a `Value` because `IntoResponse` erases the variant before serializing, but declared
    // to the generators as what it actually is: `SystemAccessRequired` is the ONLY arm that ever
    // populates this (every other arm, and `ErrorBody::new`, sends `None`), so an untyped `details`
    // described nothing while costing the SDKs their typed refusal. Should a second variant ever
    // carry details, this becomes a `oneOf` — widen it then, deliberately.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<temper_core::types::access_gate::SystemAccessDetails>)]
    details: Option<serde_json::Value>,
}
```

The condition — *"should a second variant ever carry details"* — is now met. Authorized by spec §B:
*"B adds an `ApiError` arm carrying `Vec<PlanRefusal>` under its own code, and `details` becomes a
`oneOf`."*

**The invariant this must not break, quoted rather than paraphrased** (spec §B): *"The alternative —
a route-local 400 body — was declined for forking the error contract."* So the refusals ride the
**shared** `ErrorBody`, not a `/api/query`-shaped body.

**Grounding: `details` is populated in exactly one place** (`error.rs:143-148`), which is what makes
the widening a two-arm change rather than an audit of every call site:

```rust
        let details_json = match &self {
            ApiError::SystemAccessRequired { details } => {
                Some(serde_json::to_value(details).unwrap_or_default())
            }
            _ => None,
        };
```

**And `BadRequest` genuinely cannot carry them today** — `ApiError::BadRequest(String)`
(`error.rs:39`, rendered at `:103`), so this is a new arm rather than a widening of that one. A new
arm also keeps the **code** distinct, which Task B2.1 keys on; reusing `BAD_REQUEST` would force the
client to sniff the body shape.

**Steps:**

- [ ] Add the `ApiError` arm carrying `Vec<PlanRefusal>`, with its own error code. Mirror
      `SystemAccessRequired`'s shape — it is the one existing arm that carries structured detail.
- [ ] Extend the `details_json` match. **Do not** widen the `_ => None` arm into something clever;
      two named arms and a catch-all is what makes a third one visible later.
- [ ] Widen the `#[schema(value_type = …)]` declaration to the `oneOf`. **This is the line the
      generators read** — the runtime type stays `Option<serde_json::Value>` because
      `IntoResponse` erases the variant, exactly as the incumbent comment records.
- [ ] Confirm the rendered status is **400** and that the existing 403 `SystemAccessRequired`
      rendering is byte-identical to before. A shared type widened is a change to every route's
      error contract; the regression boundary is that no other route's body moves.
- [ ] Verify: `cargo nextest run -p temper-services --lib error` and
      `cargo nextest run -p temper-client --lib http` (the client's `map_status_to_error` tests
      assert on real 403 bodies and must not move).

---

### Task B1.2: the route and the handler

**Files:** Add `crates/temper-api/src/handlers/query.rs`; modify
`crates/temper-api/src/handlers/mod.rs`, `crates/temper-api/src/routes.rs`.

**Tag: CONFORM.** Both the placement and the handler shape have an incumbent, and neither should be
invented.

**Grounding — `gated_routes()` is at `routes.rs:43`** (the spec's line number is still correct), and
the registration idiom is one `.routes(routes!(…))` per path:

```rust
fn gated_routes() -> OpenApiRouter<AppState> {
    use axum::routing::{get, patch, post};

    OpenApiRouter::new()
        .routes(routes!(
            handlers::resources::list,
            handlers::resources::create
        ))
```

**Grounding — the sibling handler is 48 lines and is the shape to copy**
(`handlers/search.rs:36-48`). `/api/search` is the closest analogue: same auth, same
`Json<Params>` in, same `Json<Response>` out, delegating to temper-services:

```rust
pub async fn search(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(params): Json<SearchParams>,
) -> ApiResult<Json<SearchResponse>> {
    let response = temper_services::backend::substrate_read::search_select(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        params,
    )
    .await?;
    Ok(Json(response))
}
```

**What differs, and it is the only thing that differs:** `search_select` takes params and answers.
The query door has a refusal branch in the middle, because `prepare` returns
`Result<ValidatedComposition, Vec<PlanRefusal>>`.

**The two functions the handler composes, with their real signatures** — read them at
`crates/temper-services/src/backend/query_read.rs` rather than from here:

```rust
pub async fn prepare(mut c: Composition) -> Result<ValidatedComposition, Vec<PlanRefusal>>
pub async fn run_composition(
    pool: &PgPool,
    principal: ProfileId,
    v: &ValidatedComposition,
) -> ApiResult<QueryResponse>
```

**`prepare` is called, never re-assembled** (decision 5 / A2 decision 9). Its doc block carries the
pipeline and the reason the seal falls where it does; the handler adds no ordering of its own.

**Steps:**

- [ ] Write the handler: deserialize → `prepare` → on `Err`, the 400 from Task B1.1 carrying **all**
      refusals → on `Ok`, `run_composition`.
- [ ] Register it in `gated_routes()`. **Not** `authed_routes()` — spec §B: *"`require_auth` +
      `require_system_access`, like every other content-touching route."*
- [ ] Write the `#[utoipa::path]` block. Include the 400 with the refusal body — that response is
      the door's most-read documentation, since a caller meets it before they meet a 200.
- [ ] Verify: `cargo nextest run -p temper-api --features test-db --test <the new target>`. **Never
      a bare `-p temper-api`** — it hangs at bin-target enumeration (CLAUDE.md).

---

### Task B1.3: the contract landing — ~51 schemas at once

**Files:** Modify `openapi.json`, `clients/temper-rb/**` (generated), `clients/temper-ts/src/generated/schema.ts` (generated).

**Tag: EXTEND**, authorized by spec §B: *"`openapi.json` regenerates in-commit — the
`generated-artifacts` gate covers it."*

**This task exists because the grounding pass found it, and the spec's B section does not mention
it.** `QueryResponse` is absent from `openapi.json` today — nothing referenced it:

```
$ python3 -c "import json;d=json.load(open('openapi.json'));print('schemas:',len(d['components']['schemas']))"
schemas: 203
$ rg -c "QueryResponse" openapi.json
QueryResponse ABSENT from openapi.json
```

and the tree that is about to arrive is **51 types** carrying `ToSchema` under `web-api`:

```
$ rg -c 'cfg_attr\(feature = "web-api", derive\(utoipa::ToSchema\)\)' crates/temper-core/src/types/query/**/*.rs
act.rs:9  filter.rs:7  composition.rs:7  stage.rs:5  hits.rs:5  envelope.rs:4
validate/mod.rs:3  trace.rs:3  id_set.rs:3  disposition.rs:3  scalars.rs:2      = 51
```

So one route grows the published contract by roughly a quarter, and **every new enum becomes Ruby
gem surface at once** — CLAUDE.md records that the gem `raise`s on an enum value it does not know
(`search_scope.rb:39` is the worked case), which is what makes a new variant a hard-fail break for
an older client. That is a property of the *generated* gem, so the risk here is not that this PR
breaks it but that nobody has looked at 51 types' worth of it in one diff.

**Steps:**

- [ ] `cargo make openapi` — regenerates `openapi.json`, the gem, and `schema.ts` together. Commit
      all three **in the same commit**; the drift gates compare against HEAD, not the index.
- [ ] **Read the generated enum surface**, do not merely regenerate it. List the new enums and their
      variants and check each spelling against the Rust `rename_all`. A wrong wire spelling here is
      not caught by any Rust test — `openapi.json` is downstream of the derives, so it agrees with
      whatever they say.
- [ ] Confirm the schema count moved by roughly the expected amount and that **no unrelated schema
      changed**. An unrelated diff means a shared type moved, which is Task B1.1's blast radius
      showing up somewhere it was not expected.
- [ ] Verify: `cargo make check` (which runs `openapi-check`, `openapi-rb-drift`, `openapi-ts-drift`)
      and `cd packages/temper-ui && bun run check`.

---

### Task B1.4: the route e2e

**Files:** Add a test under `tests/e2e/tests/`.

**Tag: CONFORM** to the standing requirement, quoted from spec §B: B's e2e *"must include a
**find-about** stage, which makes it `test-embed`-gated — and per trap 2, a run scoped
`--features test-db` alone compiles it to nothing and reads green."*

**What A2 already took, so this does not duplicate it.**
`crates/temper-services/tests/query_run_composition_test.rs::server_side_embedding` drives a
find-about stage through `prepare → compile → execute` against a real corpus with a caller that
supplies no vector. **What is still uncovered is the ROUTE**: auth, the gate, deserialization of a
composition from real JSON, and the 400 body's shape.

**Siblings to read first:** `tests/e2e/tests/search_test.rs` and
`tests/e2e/tests/server_query_embed_test.rs` — the latter is the closest, being both `test-db` and
`test-embed` gated, and it already contains the corpus-seeding helper shape this needs.

**Steps:**

- [ ] A happy-path case: a composition over a seeded corpus, through the real route, hydrated back.
- [ ] **A refusal case asserting the 400 carries MORE THAN ONE refusal.** This is the property Task
      B1.1 exists to make expressible, and a single-refusal assertion would pass against a body that
      truncates. It is also the only end-to-end witness that `details` survived the `oneOf`.
- [ ] Verify: `cargo make test-e2e-embed`. A `test-e2e` run compiles the find-about case to nothing.

---

# B2 — the CLI door

### Task B2.1: the client learns that a 400 can carry refusals

**Files:** Modify `crates/temper-client/src/error.rs`, `crates/temper-client/src/http.rs`; add
`crates/temper-client/src/query.rs`; modify `crates/temper-client/src/lib.rs`.

**Tag: AMEND**, and this task is the one the spec does not have. **Decision 6.**

**Grounding — there is no 400 arm, and the fall-through misclassifies it.**
`map_status_to_error` (`http.rs:449`) handles 401, 403, 404, 409, 422, 429, `>= 500`, and then:

```rust
        s => {
            let message =
                parse_error_message(body).unwrap_or_else(|| format!("unexpected status {s}"));
            ClientError::Server { status: s, message }
        }
```

So a `/api/query` 400 arrives as `ClientError::Server { status: 400, … }` — **the refusal list
discarded, and a caller fault reported to the user as a server error.**

**Why that is load-bearing rather than cosmetic**, quoted from `validate/mod.rs`'s module header:
*"`validate` returns **every** refusal, not the first — a caller repairing a plan should see all of
it in one round trip."* The whole reason the API answers with all of them is an experience that only
exists if the client surfaces them. Dropped here, that property is real for raw HTTP and absent for
the CLI, which is the door's headline consumer.

**Grounding — the incumbent to mirror.** `parse_system_access_details` (`http.rs:528`) is the only
details-parsing site in the client, and the 403 arm keys on the **code**, with a comment saying why:

```rust
            } else if parse_error_field(body, "code").as_deref()
                == Some(temper_core::error::FORBIDDEN_DETAIL_CODE)
            {
                // A refusal that named the capability it withheld. Keyed on the CODE, mirroring the
                // 422 arm below — a message-text heuristic would silently reclassify the message-less
                // 403 the moment either side reworded it.
```

**CONFORM to that rule: key the new arm on the CODE**, never on the presence of a `details` object
or on message text.

**Name hazard, found by grepping:** `SearchClient` already has a method called `query`
(`search.rs:29`), alongside `text_query`, `search` and `search_with_params`. A `QueryClient::query`
would be a second thing called `query` on a sibling client. Choose deliberately and say why in the
doc; do not discover the collision at review.

**Steps:**

- [ ] Add the `ClientError` variant carrying the refusals. It is a **caller** error — do not route
      it through `Server`.
- [ ] Add the 400 arm to `map_status_to_error`, keyed on the code. Note the existing arms are
      ordered by status; keep that order.
- [ ] Add `query.rs` and expose it from `lib.rs` beside `search()` (`lib.rs:145`).
- [ ] Verify: `cargo nextest run -p temper-client --lib http`. `map_status_to_error` is *"extracted
      as a pure function so it can be unit-tested without network calls"* — so a 400 body fixture is
      the cheap, exact witness here, exactly as the 403 fixtures at `http.rs:725` and `:741` are.

---

### Task B2.2: `temper query`

**Files:** Add `crates/temper-cli/src/commands/query_cmd.rs` and
`crates/temper-cli/src/actions/query.rs`; modify the CLI's command enum and dispatch.

**Tag: CONFORM.** Spec §B: *"`temper query`, transport only. Plan source mirrors `temper resource
update`'s body-source precedence rather than inventing one: `--plan @<path>` wins, `--plan -` always
blocks-reads stdin, implicit non-TTY stdin is auto-detected. A missing plan is an error — unlike
`update`, there is no frontmatter-only case."*

**Grounding — the precedence has a name, and it is generic and already tested.** The spec describes
the behaviour without naming the function; it is
`crates/temper-cli/src/actions/body_source.rs:39`:

```rust
pub fn resolve_body_source<R: Read>(
```

called from `commands/resource.rs:680` and `:2095`, each passing
`crate::actions::body_source::stdin_has_input_within` as the readiness probe (`body_source.rs:104`
documents it as *"the production `stdin_ready`"*). Its own test module exercises it at `:167`,
`:179`, `:193`, `:205`, `:216`. **Call it. Do not restate the precedence** — an inlined copy is
exactly the drift site `plan-verification.md` names, because it would be a true statement about real
behaviour that nothing links to the original.

**Grounding — the command/action split to copy.** `commands/search_cmd.rs` is **85 lines** and
`actions/search.rs` is **370**: the command parses and dispatches, the action holds the logic
(CLAUDE.md: *"Commands live in `src/commands/`, business logic in `src/actions/`"*).

**Output goes through `output/`** — never raw ANSI, and format/color resolve once in `main` via
`OutputFormat::resolve_with` and `color::apply_color_choice`. With a non-TTY stdout the default is
JSON and ANSI-free, which is how an agent will invoke this.

**Steps:**

- [ ] Add the command and the action, split at the incumbent ratio.
- [ ] Source the plan through `resolve_body_source`. **A missing plan is an error** — this is where
      B diverges from `update`, and the divergence is in the spec, so state it in the code.
- [ ] Render the refusal list from Task B2.1's error. The 400 is the response a plan author will see
      most often; showing them one refusal at a time is the experience ⟨3⟩'s "every refusal" rule
      was written to prevent.
- [ ] Verify: `cargo nextest run -p temper-cli --lib` **and** the new `--test` target. Both, per
      Global Constraints.

---

### Task B2.3: the CLI e2e

**Files:** Add a test under `tests/e2e/tests/`.

**Tag: CONFORM** to the e2e tier's purpose (CLAUDE.md): tests *"that span CLI ↔ API ↔ DB"* and drive
*"the actual `temper-cli` and `temper-client` code paths."*

**Read before writing:** the e2e harness spawns the real binary, and there are two standing traps —
a stale `temper` bin (nextest rebuilds the lib, not the bin) and the bootstrap env-var contract that
local `$HOME` masks. Both are recorded in this machine's memory; check `MANAGED_MEMORY.md` under the
e2e entries rather than rediscovering them.

**Steps:**

- [ ] A plan piped on stdin, through the real CLI, against the real server.
- [ ] A refusal case asserting the CLI **prints more than one refusal**. This is the end-to-end
      witness for decision 6, and it is the only test in the whole plan that can fail if the client
      silently drops `details`.
- [ ] Verify: `cargo make test-e2e-embed`.

---

## What this plan does NOT do

- **It does not ship `temper query --check`.** That is PR C, over `validate_shape` — which A2 already
  put into production use, so C ships a flag over an exercised function.
- **It does not add an MCP tool.** Spec ⟨2⟩ defers it, and not for schema-size reasons.
- **It does not edit `docs/api/query.openapi.yaml`.** Provisional; D owns it. D's list now runs to
  seven entries, the seventh being A2's.
- **It does not make `follow-from` or `survey` reachable.** They refuse statically as
  `NotSeparablyReachable`, and their fragments take arguments no slot supplies.
- **It does not add per-stage `properties` or `edge_filter` capability.** Those refusals stay.

## Declared risk

**The blast radius is a shared type, and it is B1.1 rather than the route.** `ErrorDetail` is on
**every route in the project**. The regression boundary is that no other route's error body moves —
assert it, do not assume it. This is why the widening ships in the same PR as its first emitter:
a widened contract with nothing emitting it is untestable at the level that matters.

**The contract landing is under-reviewed by construction.** 51 schemas arrive in one diff, and no
Rust test can see a wrong wire spelling in the generated output — `openapi.json` is downstream of
the derives and agrees with whatever they say. Task B1.3's "read the enum surface" step is the only
control, and it is a human one. Naming it here rather than pretending the gate covers it.

**Two known-uncovered things this plan inherits rather than creates.** The frame register's
answer-quality clauses remain `declared-uncovered` — nothing in B measures whether an answer is
good, only that the door opens. And a stage's `properties`/`edge_filter` refusals are still capability
refusals, so the first caller to write a natural-looking plan will meet one; B's job is to render it
well, not to remove it.
