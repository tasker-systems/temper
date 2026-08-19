# PR A2 — a composition asks one question **per find act**, not one per composition

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move `Intention` from the composition envelope onto `ActInvocation`, and give it the query
vector. After this, `find A, find B, intersect them` — two questions in one DAG — is expressible,
and the vector travels with the text it was computed from.

**Architecture:** `Composition.intention: Option<Intention>` is deleted;
`ActInvocation.intention: Option<Intention>` is added; `Intention` gains
`embedding: Option<Vec<f32>>`. Three consumers follow the field: shape validation, the compiler, and
the server-side embed. `compile`'s third parameter disappears, because the vector is no longer a
side-channel.

**Tech Stack:** Rust (serde, schemars, ts-rs, utoipa, sqlx), cargo-nextest. No database change, no
migration, no `.sqlx` regeneration.

**Spec:** [`docs/superpowers/specs/2026-08-12-api-query-door-design.md`](../specs/2026-08-12-api-query-door-design.md) ⟨7⟩ — read it first. It carries the ruling, the argument, and the provenance finding this plan rests on.

**Why now, and not later:** ⟨4⟩'s timing argument applies unchanged. Nothing consumes this contract,
so moving `query` from the envelope onto the stage is **free now and a breaking change to every
stored plan the moment `/api/query` publishes**. This PR must land before B.

## Decisions this PR takes

**This block is the contract between the plan and whoever ratified it. A decision that is not listed
here was not taken — if implementation needs one, it stops and asks rather than recording it in a
doc comment, a commit message or a test name.** That failure is not hypothetical: ⟨7⟩ exists because
the incumbent placement of `Intention` entered as a first-person paragraph in commit `3d73a70b`,
hardened into the test name `the_intention_is_a_composition_level_field_not_a_per_stage_one`, and
steered three sessions of planning before anyone asked who took it.

**The closing summary of this PR must reproduce this table verbatim**, including the OPEN rows.

| # | Decision | Rests on | Status |
|---|---|---|---|
| 1 | `Intention` moves from `Composition` onto `ActInvocation` | Pete, from a three-option prompt carrying the wire JSON for each | **decided** `[2026-08-12, Pete]` |
| 2 | `Intention` gains `embedding: Option<Vec<f32>>`; the server still embeds when none arrives | Pete, in his own words: *"the cli already has the ability to generate the vector embedding… but we cannot assume that many api callers will have this ability"* | **decided** `[2026-08-12, Pete]` |
| 3 | This lands as its own PR, before B | Pete, after the sequencing was flagged as an unratified agent decision | **decided** `[2026-08-12, Pete]` |
| 4 | `compile` loses its `embedding` parameter | Derived from #1 + #2 — two sources for one fact is the prev-else-context shape this contract refuses | **derived**, argued at Task 3 |
| 5 | The envelope-level intention is **deleted, not demoted to a default** | Derived from #1 — a stage inheriting the envelope's intention is prev-else-context under another name | **derived**, argued in Declared risk |
| 6 | The server embeds **before** `validate()`, behind a `validate_shape` gate — deserialize → shape → embed → validate → compile | Pete, in prose: *"if a composition is structurally invalid then we don't want to pay the onnx cost"* | **decided** `[2026-08-13, Pete]` |
| 7 | `Intention.embedded` is **deleted**, not moved to `StageTrace` | Pete, in prose, after the `StageTrace` proposal was put and declined — the boolean's only live distinction is already covered by `EmbeddingUnavailable` | **decided** `[2026-08-13, Pete]` |
| 8 | `validate_shape` is **not** reserved for PR C | The reservation was an unsigned line in a ⟨3⟩ code block that hardened into a hand-off constraint | **decided** `[2026-08-13, Pete]` |
| 9 | The shape→embed→validate order lives in ONE function, `query_read::prepare` — not spelled at each call site | Pete, from a three-option prompt. Row 6 ruled the *order*; it did not say where the order lives, and Task 4 found there is no caller of `run_composition` to build it in — B's route does not exist | **decided** `[2026-08-13, Pete]` |

**Nothing in this plan is OPEN.** `[2026-08-13]` Rows 6 and 7 were open for a day and are now ruled;
Tasks 4 and 5 are implementations rather than questions. If execution turns up a decision this table
does not carry, that is the signal to **stop and ask** — not to record one and continue.

**Efficiency is measured, not assumed** `[2026-08-13, Pete]`: row 6 is taken on the expectation that
invalid compositions rarely reach the embed step — the CLI catches them offline, MCP carries the
schema, and serde rejects malformed bodies before anything runs. If that proves false the shape gate
is where the counter goes. Do not pre-optimise it.

## Global Constraints

- **Never scope a test with `--workspace`** — it hangs on bin-target enumeration. Always `-p <crate>`, prefer `--test <target>`.
- **Do not pipe test output through `tee`** — it reports tee's exit code, so a red gate looks green.
- **`cargo make check` must pass** before any task is claimed complete. **It is not sufficient on its own** — see the next two bullets.
- **`cargo make check` does not run the schema-snapshot tests.** `crates/temper-core/tests/query_schema.rs` is `#![cfg(feature = "mcp")]` and feature-pinned on purpose. **A doc comment on a schemars-derived type is a serialized `description`** — this plan moves and rewrites several. Regenerate with `UPDATE_SCHEMA=1 cargo nextest run -p temper-core --features mcp --test query_schema` and commit the fixtures **in the same commit**. This exact gap reddened CI three times on the sibling PRs.
- **Run it package-scoped** — `-p temper-core`, never `--workspace`. Feature unification changes the emitted schema; `-p` is what the regen emits and what the gate compares.
- **`readback/` tests are gated `artifact-tests`.** A run scoped `--features test-db` compiles them to nothing and reads green. Task 3's verification needs `cargo make test-artifacts`.
- **ts-rs types regenerate too**: `cargo make generate-ts-types` — `Intention`, `ActInvocation` and `Composition` all carry `ts(export, export_to = "query.ts")`.
- **`docs/api/query.openapi.yaml` is NOT edited here.** The contract is provisional and **D owns it** — this is the same call PR A took for ⟨5⟩'s flip. Record the lag in the spec's D section instead; see Task 6.

---

### Task 1: `Intention` moves onto the stage and gains the vector

**Files:**
- Modify: `crates/temper-core/src/types/query/composition.rs` (delete `Composition.intention`; `Intention` itself lives here and stays)
- Modify: `crates/temper-core/src/types/query/envelope.rs:21-50` (`ActInvocation` gains the field)
- Modify: `crates/temper-core/tests/fixtures/query/*.schema.json` (regenerated)
- Modify: `packages/temper-ui/src/lib/types/generated/query.ts` (regenerated)

**Interfaces:**
- Produces: `ActInvocation.intention: Option<Intention>`, consumed by Tasks 2, 3 and 4.
- Produces: `Intention { query: String, embedding: Option<Vec<f32>> }` — final shape, `embedded` gone.

> **`[deviation — 2026-08-13, during execution]` Task 5's deletion of `embedded` was FOLDED into
> Task 1.** This line previously said *"`embedded` is deleted by Task 5, not here"*, on the grounds
> that separating them lets a bisect tell "the field moved" from "the field went away". In execution
> that reasoning did not survive contact: keeping the field through Task 1 means writing
> `embedded: false` into ~25 construction sites and deleting all 25 again in Task 5, and both
> commits land in one PR, so the bisect distinction buys nothing a `git show` would not give.
> **The DECISION is unchanged** — row 7 ruled the field deleted, and it is. Only the sequencing
> moved. Recorded rather than silently done, per this plan's own opening rule.

**No `Eq` on the affected types** `[found during execution — 2026-08-13]`. `Vec<f32>` is not `Eq`,
so the derive must come off `Intention` and every type transitively holding it — `ActInvocation`,
`StageNode`, `Composition`. `PartialEq` stays, which is enough for every `assert_eq!` in the
corpus. The precedent is stated in-tree: `StageResult` derives *neither* "while the quantities are
floats" (`envelope.rs:71-75`); this is that argument one derive milder.
- Removes: `Composition.intention`.

**Grounding you can rely on** (verified at `2d3c94d5`):

`ActInvocation` today — note there is no text slot, which is the whole finding (`envelope.rs:21-50`):

```rust
pub struct ActInvocation {
    pub name: StageName,
    pub act: ActName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<StageInput>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub terms: BTreeMap<BoundTerm, i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_filter: Option<ResourceFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_filter: Option<EdgeFilter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<PropertyPredicate>,
}
```

`Intention` today, with the ruling the new field must **not** contradict (`composition.rs:20-36`):

```rust
/// An absent EMBEDDING is a different absence and does **not** refuse. The CLI can embed; the ruby
/// gem, the TypeScript package and MCP structurally cannot, so refusing a vector search for want
/// of a precomputed vector would deny this surface to every non-CLI client. The server embeds when
/// none arrives, exactly as `/api/search` already does, and only a FAILED embed refuses — as
/// [`super::disposition::RefusalReason::EmbeddingUnavailable`], the one runtime refusal in the
/// contract. `[decided — 2026-08-08, Pete]`
pub struct Intention {
    pub query: String,
    /// Whether an embedding was computed for it. Inspectable in the trace, which is what makes
    /// paraphrase-stability measurable from outside.
    pub embedded: bool,
}
```

**Tag: AMEND.** Authorized by spec ⟨7⟩ *"A find act's question — its text, and now its vector — is a
**parameter of that act**, carried on `ActInvocation` beside `terms`, `resource_filter` and
`properties`. `Composition.intention` goes away."* The disk thing being amended is
`composition.rs:181` (`pub intention: Option<Intention>`) and the test that hardened it,
`the_intention_is_a_composition_level_field_not_a_per_stage_one` (`composition.rs:290-303`).

**Steps:**

- [ ] Add `intention: Option<Intention>` to `ActInvocation`, with `#[serde(default, skip_serializing_if = "Option::is_none")]` matching every other optional field on that struct.
- [ ] Add `embedding: Option<Vec<f32>>` to `Intention`, likewise skipped when absent. **A 768-float array must never serialize into a response** — it does not today, because `CompositionTrace` is `{ stages }` (`trace.rs:93-98`) and never echoes an intention. If Task 5 later puts an intention in the trace, that constraint becomes load-bearing rather than incidental; note it in the field's doc now.
- [ ] Delete `Composition.intention`. Follow the **remove-and-tolerate precedent** the same struct already sets twice: `on_stage_refusal` and `bounds` were removed and an unknown key is ignored like any other, *"pinned by the legacy-payload test below"* (`composition.rs:194-203`). Extend that legacy-payload test so an envelope-level `intention` is ignored rather than erroring.
- [ ] **Invert `the_intention_is_a_composition_level_field_not_a_per_stage_one`** rather than deleting it. A test asserting the opposite, under a name that says so and a comment citing ⟨7⟩, is what stops this being re-derived. Its current rationale — *"Computed ONCE at composition start and threaded, so every find-about-\* stage provably interrogates the same intention rather than re-embedding a mutated string"* — is the property ⟨7⟩ **deliberately gives up**; say that in the comment, do not merely drop it.
- [ ] Rewrite `an_intention_carries_the_fact_of_embedding_and_never_the_vector` (`composition.rs:317-330`). Its assertion is a byte-exact serialization of `Intention`, so it goes red by construction. Its *reason* — *"putting it in the envelope would be a wire contract nobody asked for"* — is what ⟨7⟩ overturns, and ⟨7⟩'s counter-argument (`CompositionTrace` never echoes an intention, so this is request-only) belongs in the replacement.
- [ ] Regenerate schema fixtures **and** ts-rs types, in-commit. `intention.schema.json`, `act_invocation.schema.json` and `composition.schema.json` all move.
- [ ] Verify: `cargo nextest run -p temper-core --features mcp --test query_schema` and `cargo nextest run -p temper-core --lib types::query`.

---

### Task 2: shape validation follows the field

**Files:**
- Modify: `crates/temper-core/src/types/query/validate/shape.rs:400-420`
- Modify: `crates/temper-core/tests/query_validate_seam.rs:110` (the `MissingIntention` count pin)

**Interfaces:**
- Consumes: `ActInvocation.intention` from Task 1.
- Preserves: `RefusalReason::MissingIntention` — same variant, same pass, same count.

**Grounding you can rely on** — the check is **already per-stage**, which is why this task is small
(`shape.rs:400-420`):

```rust
    if matches!(
        inv.act,
        ActName::FindExact | ActName::FindAboutAnywhere | ActName::FindAboutWithin
    ) {
        let missing = match c.intention.as_ref() {
            None => Some("a find act requires a threaded intention"),
            Some(i) if i.query.trim().is_empty() => Some(
                "a find act requires a threaded intention with a question in it; this one is empty",
            ),
            Some(_) => None,
        };
        if let Some(detail) = missing {
            errs.push(refusal(Some(name), RefusalReason::MissingIntention, detail));
        }
    }
```

It already loops per invocation and already attaches the refusal to `name`. The change is
`c.intention` → `inv.intention` and two detail strings that stop saying *"threaded"*.

**Tag: CONFORM.** The seam rule this must not break is spec ⟨3⟩: *"The shape pass may raise only
refusals that cannot change without a change to the published wire contract."* `MissingIntention` is
classified **shape** in ⟨3⟩'s table on the grounds that it is *"hardcoded `matches!` on three act
names, never consults `search_family()`"* — moving the field it reads does not touch that reasoning,
and the module must still not import `registry` (guard one, a source scan).

**Steps:**

- [ ] Read `inv.intention` instead of `c.intention`. Keep the empty/whitespace arm — `[widened — 2026-08-09]` records why it exists, and the reason is unchanged by the move.
- [ ] Update both detail strings. *"threaded"* was the envelope's vocabulary; a stage carries its own.
- [ ] **Guard two must stay green without editing its table.** `query_validate_seam.rs:110` pins `("MissingIntention", 1)` — how many SITES in the shape pass emit that reason, not how many stages do. One site before, one site after. If that entry needs changing, the change is wrong: re-read ⟨3⟩'s *"Guard two's expected table does not move when a variant is split this way"* before touching it.
- [ ] Add a test that two find stages with **different** intentions both validate — the case that was inexpressible, asserted at the layer that used to make it impossible.
- [ ] Add a test that one find stage missing its intention refuses **while its sibling validates**, and that the refusal names the right stage. Under the envelope this distinction could not exist.
- [ ] Verify: `cargo nextest run -p temper-core --test query_validate_seam` and `cargo nextest run -p temper-core --lib types::query::validate`.

---

### Task 3: the compiler reads the stage's intention, and `compile` loses its embedding parameter

**Files:**
- Modify: `crates/temper-substrate/src/readback/query_plan.rs:167-192` (`compile`), `:216`, `:222`, `:285-292` and `:322-368` (`emit_act_body`)
- Modify: `crates/temper-substrate/tests/query_plan_compile.rs`, `crates/temper-substrate/tests/query_plan_execute.rs`

**Interfaces:**
- Changes: `compile(v: &ValidatedComposition, principal: ProfileId) -> Result<CompiledQuery, PlanRefusal>` — the `embedding: Option<&[f32]>` parameter is **removed**.
- Consumes: `ActInvocation.intention` from Task 1.

**Grounding you can rely on** — `compile`'s own doc is the justification ⟨7⟩ overturns, and it must
be rewritten rather than left standing (`query_plan.rs:170-173`):

```rust
/// `embedding` is the query vector. It is a parameter rather than a field of
/// `Composition.intention` because `Intention` is a WIRE type carrying `query: String` and
/// `embedded: bool` — the *fact* that an embedding was computed, never the vector. Putting a
/// 768-float array in the envelope would be a contract change nobody asked for.
```

And the two per-stage read sites (`:216`, `:322-326`):

```rust
    let intention = v.composition().intention.as_ref();
    …
                let emitted = emit_act_body(inv, intention, embedding, &mut binds, &mut refusals)?;
```

```rust
            let q = intention.map(|i| i.query.as_str()).ok_or_else(|| {
                missing_question(
                    inv,
                    "find-exact needs the intention's query text — it becomes `p_query`, and there \
                     is nowhere else to source it. The composition threaded no intention",
                )
            })?;
```

**Tag: AMEND.** Authorized by ⟨7⟩. Removing the parameter is the point, not a tidy-up: with the
vector on the stage, a `compile` that also accepts one has **two sources for one fact**, and the
prev-else-context fallback this whole contract refuses is exactly what two sources for one fact
becomes. The task body names that vector by name — *"`resolve_target()` is 'use `.prev` when
present, otherwise `.context`' — the prev-else-context fallback the contract names as the
flattering-degradation vector Temper must not inherit."*

**Steps:**

- [ ] Delete `compile`'s third parameter. Drop `let intention = …` at `:216`; `emit_act_body` reads `inv.intention` for both the text and the vector.
- [ ] Rewrite the doc block at `:170-186`. **Keep** the `[amended — 2026-08-08, Pete]` paragraph — *"Embedding on the caller's behalf is this surface's job"* — that ruling is untouched by ⟨7⟩ and is what Task 4 implements. **Replace** the `:170-173` paragraph with ⟨7⟩'s reasoning.
- [ ] Preserve the `None`-means-could-not-obtain semantics exactly (`:175-186`). A stage whose intention carries no vector, after the server has tried, still refuses `EmbeddingUnavailable` — *"the stage holds a well-formed question it cannot answer, and searching on nothing returns a list that reads like an answer."* This is the refusal that must not become a silent NULL bind.
- [ ] Update `missing_question`'s detail strings: *"The composition threaded no intention"* is now false — the stage carries none.
- [ ] Add a compile test that two find-about stages with **different** vectors emit **different** binds. This is the property the whole PR exists to create, and it is invisible at the type layer alone.
- [ ] Verify: `cargo make test-artifacts` — `readback/` tests are `artifact-tests`-gated and a `--features test-db` run compiles them to nothing.

---

### Task 4: the server embeds per intention, not per composition

**Files:**
- Modify: `crates/temper-services/src/backend/query_read.rs:45-101` (`run_composition`, `resolve_embedding`) and the `wants_a_vector` region at `:1030-1060`
- Modify: `crates/temper-services/tests/query_run_composition_test.rs`

**Interfaces:**
- Changes: `run_composition`'s `caller_embedding: Option<Vec<f32>>` parameter — see the open decision below.
- Consumes: Task 3's `compile` signature.

**Grounding you can rely on** — the current resolution is composition-wide and singular
(`query_read.rs:84-101`):

```rust
async fn resolve_embedding(
    v: &ValidatedComposition,
    caller_embedding: Option<Vec<f32>>,
) -> Option<Vec<f32>> {
    if caller_embedding.is_some() {
        return caller_embedding;
    }
    if !v.ordered().iter().any(wants_a_vector) {
        return None;
    }
    let query = v.composition().intention.as_ref().map(|i| i.query.trim())?;
    …
}
```

`wants_a_vector` is **already a per-node predicate** (`v.ordered().iter().any(wants_a_vector)`), so
the per-stage shape is a `filter` where there is an `any` — the smaller half of this task.

**DECIDED `[2026-08-13, Pete]` — the pipeline is shape-gated, and the vector is written into the
plan before it is validated:**

```
deserialize          serde — minimal shape, free, rejects a malformed body before anything runs
  → validate_shape   cheap, pure, no DB and no declarations (⟨3⟩'s expressibility pass)
  → embed            only the intentions that need one and did not carry one
  → validate         the full pass, capability included
  → compile
```

The alternative — **`compile` takes a side-channel** `BTreeMap<StageName, Vec<f32>>` — was declined:
it reintroduces the two-sources-for-one-fact shape Task 3 exists to remove, one layer down.

**Two corrections to how this was first framed, both worth carrying.** `[2026-08-13]` The plan
originally presented parse-don't-validate as a constraint handed to us — *"the server cannot fill in
vectors after validation"* (`validate/mod.rs:118-126`). **We choose where that seal falls; it is our
own line, not an external law**, and the shape above puts it after the embed rather than before.
And the claim that `validate_shape` was reserved for PR C was an unsigned line in a ⟨3⟩ code block
that hardened into a hand-off constraint — see the spec's correction. A server-side caller makes C
stronger, not weaker.

**`validate` runs shape internally, so shape is evaluated twice.** A pure function over a small
struct; noise. Named so it is not later "discovered" as a defect.

**Whichever is chosen, embed per DISTINCT query text, not per stage.** Two stages naming the same
string must not pay ONNX twice, and must not be able to receive two different vectors for one
question — which would make paraphrase-stability unmeasurable in precisely the way the envelope
placement was trying to protect.

**Tag: AMEND**, authorized by ⟨7⟩; the *"server embeds when none arrives"* rule it implements is
CONFORM to `[decided — 2026-08-08, Pete]` (`composition.rs:20-25`).

**Steps:**

- [x] Build the shape-gated pipeline above in the caller of `run_composition`, so the vector is on the plan before `validate` seals it.
- [x] Resolve embeddings per **distinct intention text**, not per stage — two stages naming the same string must not pay ONNX twice, and must not be able to receive two different vectors for one question.
- [x] Keep the two absences distinct. `MissingIntention` (no question) is a shape refusal; `EmbeddingUnavailable` (the server tried and failed) is the contract's one runtime refusal. `[widened — 2026-08-09]` at `shape.rs:405-409` records what happens when they blur: *"the caller was told `embedding_unavailable` — a server fault, for a question they never asked."*
- [x] Update `wants_a_vector`'s doc block. Its `[fixed — 2026-08-12]` narrative and the `emitted_fragment_for` two-hop derivation are **unchanged and must stay** — that is the repair that keeps `served_by` free to move. Only the composition-wide framing around it changes.
- [x] Add the **find-about** case to `query_run_composition_test.rs`. `query_read.rs:1041-1048` names its absence as the reason a hardcoded `"search_wide"` survived the `served_by` repoint with nothing going red: *"there is no find-about case in `query_run_composition_test.rs`, and `/api/query` has no route yet, so the door would have opened already broken."* This is that line being taken. It is `test-embed`-gated.
- [x] Add a test that two stages with different questions receive **different** vectors, end to end.
- [x] Verify: `cargo nextest run -p temper-services --features test-db,test-embed --test query_run_composition_test`. A `test-db`-only run compiles the embed cases to nothing. — **8/8**, both new cases included.

> **`[deviation — 2026-08-13, during execution]` "the caller of `run_composition`" does not exist,
> so Task 4 built the pipeline's home rather than wiring into one.** `rg run_composition` finds
> only `query_read.rs` itself and seven test call sites; B's route is what would have been the
> caller. Decision 9 rules where the order lives. The step's *intent* — the vector is on the plan
> before `validate` seals it — is unchanged and is now structural: `run_composition` still takes a
> `ValidatedComposition`, and `prepare` is the only constructor, so there is no way to reach the
> compiler having skipped the embed.

> **`[found during execution — 2026-08-13]` The predicate this task needed already had a name, and
> the plan did not carry it.** `temper-services`' own
> `substrate_read::embed_query_text(&str) -> QueryEmbed` was extracted for exactly this caller —
> *"so `/api/query` reaches the SAME attempt rather than a second one … A second implementation
> here would be a second answer to 'which space is this vector in'"*. `embed_missing_intentions`
> calls it; it re-rolls neither the `spawn_blocking` hand-off nor the wall-clock budget. Recorded
> because a plan that names no incumbent is exactly the shape `plan-verification.md` warns is
> invisible to a name-check.

> **`[found during execution — 2026-08-13]` The branch was RED when this task picked it up, and the
> hand-off's verified baseline could not have seen it.** `cargo nextest run -p temper-services
> --lib backend::query_read` — **11 of 12 failing**. Task 1's per-stage move left this module's
> `act_node` helper at `intention: None`, and `plan()` calls `validate`, which now refuses
> `MissingIntention`; two further tests hand-built the same literal inline. Workspace
> `cargo check --all-features --all-targets` was genuinely 0 errors and clippy genuinely clean —
> these are runtime failures — and the baseline's `temper-services` line named the **`--test`
> target**, not `--lib`. Fixed here (the two inline sites now build through `act_node`, so the
> helper is the one definition), and `act_node`'s doc says why its query text is allowed to be
> arbitrary, so the next reader does not mistake the placeholder for a question. **The lesson is
> the one this plan already records one paragraph down, arriving a second time by a different
> route: a suite that was never run is not a suite that passed.**

---

### Task 5: delete `Intention.embedded`

**Files:**
- Modify: `crates/temper-core/src/types/query/composition.rs` (`Intention`)
- Modify: `crates/temper-core/tests/fixtures/query/intention.schema.json` and any fixture carrying it (regenerated)
- Modify: every construction site — counted in Declared risk

**Interfaces:**
- Produces: `Intention { query: String, embedding: Option<Vec<f32>> }`. Nothing else.
- Removes: `Intention.embedded`.

**Tag: AMEND**, authorized by spec ⟨7⟩ *"`Intention.embedded` is DELETED, not relocated"*
`[decided — 2026-08-13, Pete]`. **`StageTrace` is NOT touched** — the proposal to move the field
there was put and declined.

**The argument, carried so the field is not re-added by someone who spots its absence.** The
boolean's only live distinction is *your vector was used* versus *the server embedded for you*, on
the success path — the failure case is already `EmbeddingUnavailable`, the contract's one runtime
refusal (`disposition.rs`). Its stated purpose, *"makes paraphrase-stability measurable from
outside"*, is unfulfilled by construction: `CompositionTrace` carries only `stages`
(`trace.rs:93-98`). Nothing measures paraphrase stability, and every clause of the frame register is
`declared-uncovered`.

The one hazard where embedding provenance genuinely bites — a released CLI embedding with a
different model than the server's corpus, which happened, *"the index filled with vectors from two
different models with nothing recording which"* — is already guarded by `temper-ingest/build.rs`'s
model-sha256 pin, and **a `bool` cannot express model identity anyway.** The field is also
caller-asserted and validated by nothing.

**If real provenance is ever wanted it returns as a model identity on `StageTrace`** — additive, and
answering a question the boolean could not. That is a future feature, not a remainder this PR owes.

**Steps:**

- [ ] Delete the field and its doc comment.
- [ ] Update every construction site. `serde` has no `#[serde(default)]` on it today, so a stored payload carrying `embedded` would now fail to deserialize — apply the **remove-and-tolerate precedent** the struct's neighbours already set (`composition.rs:194-203`) so an unknown `embedded` key is ignored, and extend the legacy-payload test to cover it. Nothing ships against this contract, but the precedent is cheap and the test is the record.
- [ ] Rewrite `an_intention_carries_the_fact_of_embedding_and_never_the_vector` — already flagged for rewrite in Task 1, and this is what it becomes: an assertion that the intention carries the query and the vector, and no claim *about* the vector.
- [ ] Regenerate schema fixtures and ts-rs types, in-commit.
- [ ] Verify: `cargo nextest run -p temper-core --features mcp --test query_schema` and `cargo nextest run -p temper-core --lib types::query`.

---

### Task 6: record the contract lag; do not edit the yaml

**Files:** Modify `docs/superpowers/specs/2026-08-12-api-query-door-design.md` (the D section only).

**Tag: CONFORM** to the standing ruling that `docs/api/query.openapi.yaml` is **provisional** and D
owns it. PR A set the precedent for exactly this: ⟨5⟩'s flip falsified two passages and *"the yaml
is deliberately **not edited by PR A** — the contract is provisional and D owns it — so the
correction is recorded here rather than applied."*

**Steps:**

- [x] Add A2's items to the D list. At minimum: every worked example carrying an envelope-level `intention` is now wrong, and `Composition.intention` is gone from the schema. — landed as D's **seventh** entry, which also records that those examples do not merely mis-document: they come back `missing_intention` on every find stage.
- [x] Do **not** touch `query.openapi.yaml`.

> **`[widened — 2026-08-13, during execution]` Two passages in the spec's **B** section were also
> falsified, by this PR rather than by the yaml's age, and were corrected in place.** B's handler
> sketch still read `validate()` and `run_composition(…, caller_embedding)` — both moved — and the
> *"`wants_a_vector` integration hole is B's to close"* paragraph named work Task 4 has now done.
> This is outside the task's stated *"D section only"* scope, and is done anyway on the narrow
> ground that these are not lag: **D records what the provisional contract has not caught up to;
> these two were sentences A2 itself made untrue**, and leaving them would hand B a grounding
> passage that fails its own verification pass. `query.openapi.yaml` is still untouched.

---

## `[execution note — 2026-08-13]` A scripted refactor silently changed what four tests asked

Moving a field across ~25 construction sites invites a scripted pass, and one was used. It
introduced a defect **no compiler could catch and three of the four affected tests did not catch
either**: where a composition-level `intention` was deleted and a stage-level one inserted, the
inserted literal carried a **hardcoded placeholder query** (`"salience"`) instead of the string the
test had been using. Four sites lost their real question — two `"kestrel"` in
`query_plan_execute.rs`, two `"composable"` in `query_run_composition_test.rs`.

**Why it nearly shipped green.** The compile-only tests assert on emitted SQL *shape*, so the query
text is invisible to them and they passed. Only the **database-backed** assertions — "this stage
produced 2 rows" against a corpus seeded with the word *kestrel* — could see it, and those are
`artifact-tests` / `test-db` gated, which a default `cargo nextest` run compiles to nothing (trap 2).
A run scoped to the fast suites would have reported a clean refactor.

**Two habits this argues for, neither of which is "don't script it":**

- **A mechanical pass may move a value; it must never author one.** Where the script could not
  recover the original string it should have failed loudly, not substituted a plausible constant.
- **When a refactor touches test fixtures, the DB-backed suites are the gate**, not the fast ones —
  the fast suites are exactly the ones structurally blind to fixture semantics.

Recovered by `git diff`-ing the deleted literals rather than re-deriving them, and the whole
substitution set was then audited: three further strings (`"composable fragments"` ×2, `"x"`)
appeared deleted-without-replacement but survive as `set_intentions(&mut c, "…")` arguments. No
other test changed what it asks.

## What this plan does NOT do

- **It does not open the door.** `POST /api/query` and `temper query` are PR B. A2 changes the contract B will publish; it publishes nothing.
- **It does not add per-stage `properties` or `edge_filter` capability.** Those refusals stay (`validate.rs:378`/`:386`, Tasks 10b and 11).
- **It does not touch `door_coverage`.** A1 emptied the bounds axis; nothing here moves a declaration.
- **It does not add a second question to any act's SQL.** Both find fragments already take one query text and one vector per call — this changes where the caller writes them, not what the fragment receives.
- **It does not commit a visibility-hoist strategy.** Still behind its seam, still awaiting the portable visibility-cost probe (`019fddc6-aace-7db0-a14d-5c610bc6506b`).

## Declared risk

**Two open decisions block two of six tasks** (Task 4's write-point, Task 5's `embedded`). Both are
named rather than resolved, per GD-5: a plan that fabricates a resolution to look complete is the
failure this repo has now caught three times on this arc.

**The fallout is measured, not estimated:** 25 construction sites of a composition-level intention
across six files — `validate/mod.rs` (10), `query_plan_execute.rs` (7), `query_plan_compile.rs` (3),
`query_run_composition_test.rs` (3), `query_read.rs` (1), `composition.rs` (1). Every one is a test
fixture or a single production read; there is no third-party consumer, which is the whole reason
this is free today.

**The property being given up is real.** Envelope placement made "every find stage asks the same
question" structural. After A2 it is merely *declared* — two stages may differ, and a caller who
meant them to agree can now write plans where they do not. That is the cost of expressiveness and it
is accepted, not mitigated. What must not follow is a *default* that quietly makes them agree again:
a stage inheriting the envelope's intention when it declares none is the prev-else-context fallback
under another name, and ⟨7⟩ ruled it out by deleting the envelope field rather than demoting it.
