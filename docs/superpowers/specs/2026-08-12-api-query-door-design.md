# The `/api/query` door — design

`[designed — 2026-08-12, Pete + session]` Under frame register *"Every question to Temper is
answered by a situated act — acts compose by piping, no constant decides between them, and no
composition flatters"* (goal `019fbdb9-f287-79c0-aab6-efa0b1de12c8`), task
`019fd25f-c0ea-70d2-a442-c46d697c4598`.

Successor to the coverage map (session `019fed80-6d1b-7830-85c0-28cc2bfb7daf`), whose `NEXT` item 2
this document answers: *"the door — still the next beat, still opens with a design conversation: how
a DAG is expressed through a flag surface and a tool schema."*

The wire contract [`docs/api/query.openapi.yaml`](../../api/query.openapi.yaml) remains
**provisional** `[decided — 2026-08-09, Pete]`; alignment with it is bidirectional and adjudicated
case by case. Nothing here re-opens its RATIFICATION block or the ADJ rulings.

---

## What was decided, and what it cost to decide

Five rulings, in the order they were taken. Each names the argument that decided it, because the
argument is the part that has to survive into implementation.

### ⟨1⟩ The CLI is a transport, not a composer

`temper query` takes a plan and returns a response. It does **not** assemble a DAG from flags.

The decisive argument is not ergonomics. A repeatable `--stage 'near = find-about-within --bound-on
seed --limit 10'` surface is a second grammar for a document that already has a published schema,
and it would have to grow combinator syntax, then quoting rules, then precedence. That is the
expression language this arc already refused —
`crates/temper-core/src/types/query/filter.rs:6` records that such a language *"would be more
expressive and would immediately re-open every conflation this contract exists to close"*, and the
task body's summary of the same ruling is *"a schema, not a syntax."*

**A cost considered and found weaker than first stated.** It was raised in-session that a transport
CLI leaves `door_coverage`'s CLI term axis with nothing to check. That is false as stated:
`DoorReach::Serves { terms_unreachable }` describes what a door *can supply*, a transport door
supplies everything the API does, and tier 2 finding no term flags on `temper query` is honest —
there are no term flags to lie about. See ⟨5⟩ for where the axis genuinely moves.

### ⟨2⟩ The MCP tool is deferred, and not for schema-size reasons

`[decided — 2026-08-12, Pete]` The MCP tool is taken up separately, in the context of MCP tooling
consolidation — the current tool set should be slimmed and generalized, and some tools removed.
The reason is a real asymmetry between the surfaces: *a CLI can carry many top-level commands and
flags at near-zero cost to a caller, where proliferating MCP tools eats the context window every
session, used or not.*

Measurement taken while the question was open, recorded so it is not re-measured: the derived
schemars schema for `Composition` — which *is* the MCP tool's input schema — is **27,353 bytes
across 20 `$defs`** (`crates/temper-core/tests/fixtures/query/composition.schema.json`). It is
dominated by prose, not structure: `ResourceSection` alone is 2,241 bytes for a three-variant enum.
**~12% of it (3,316 bytes) describes fields that are unconditional refusals today** — `properties`
(`validate.rs:378`) and `edge_filter` (`validate.rs:386`) are declined on every act. Whatever the
tool ends up being, it must not offer those.

Size was **not** the deciding factor and should not be cited as one.

### ⟨3⟩ Validation splits into two passes, separated by what each can see

The distinction is Pete's, and it is not a refactor of one thing into two files. It is two different
questions that were being answered by one function:

- **Expressibility** — is this a well-formed composition? True of the plan and the published schema
  alone. Cannot change under the caller's feet.
- **Capability** — the shape is fine, and this server has not built it yet. Moves with every beat.

```
query::validate::shape        fn validate_shape(&Composition) -> Vec<PlanRefusal>
                              the module does NOT `use super::registry`
query::validate::capability   fn validate_capability(&Composition, &[ActDeclaration]) -> Vec<PlanRefusal>
query::validate               fn validate(&Composition)  =  shape ++ capability(search_family())
```

**The import is a necessary guard, not a sufficient one.** `[corrected — 2026-08-12, during plan
grounding]` A pass that cannot reach `declaration()` cannot raise `NotSeparablyReachable`, and that
much matters: version skew is structural here, not hypothetical, since CLAUDE.md's *"Release ≠
deploy"* means a released CLI carries a `search_family()` older than the server's, and
`CALLABLE_FRAGMENTS` — which decides `NotSeparablyReachable` — is precisely what beats D, 10b and 11
keep widening.

But "reads the declaration" and "is a capability refusal" **come apart**, and they come apart on
exactly the refusals that move. Five sites read no declaration at all and are nonetheless pure door
capability — their own detail strings say so: `validate.rs:381` (*"this door does not **yet** apply
property predicates"*), `:389` (*"the only act that admits one still compiles to the absent
placeholder"*), `:355`, `:370`, and `:777`'s `SectionNotAvailable`, which reads
`ReturnSpec::ADMITTED_SECTIONS`. Task 10b makes the first work; a widened `ADMITTED_SECTIONS` makes
the last work. An import scan would let all five sit in the shape pass, and a stale client would
then refuse plans a newer server runs — the failure this seam exists to prevent.

**So the rule is stated positively, and guarded twice.** The shape pass may raise only refusals that
cannot change without a change to the published wire contract. Guard one: the shape module does not
import `registry` (source scan). Guard two: the set of `RefusalReason`s the shape pass can emit is
**pinned by a test**, in the family of `the_cells_tier_one_cannot_discriminate_are_exactly_these` —
because the classification of the five sites above is a judgment, and a judgment needs a pin, not an
inference.

The classification, derived by reading every refusal site rather than by shape:

The classification below is **per site**, derived by reading each one — not per variant, because two
variants straddle the seam.

| Site | Reason | Class |
|---|---|---|
| `validate.rs:657, :669, :684, :696, :705, :729, :754, :765, :796` | the nine topology `Other(_)` strings | shape |
| `:504`, `:512` | `Other("empty-property-key")`, `Other("empty-contains")` | shape |
| `:227` | `Other("unknown-act")` | shape — **rewritten** as `matches!(inv.act, ActName::Other(_))`; `ActName` is open (`act.rs:45-46`), so this is caller-reachable and answerable from the type alone |
| `:488` | `MissingIntention` | shape — hardcoded `matches!` on three act names, never consults `search_family()` |
| `:272` | `MissingProvenance` | shape — `kind == Region && provenance.is_none()`, pure plan inspection |
| `:311` | `AnchorTakesOneId` | shape — `ids.ids.len() != 1`, pure plan inspection |
| `:497` | `UnknownFilterValue` | shape — `PropertySubject::Other(_)`, a closed vocabulary in the schema |
| `:408` | `BoundTermNotApplicable`, negative value | shape — a row count below zero is malformed whatever the act |
| `:238`, `:246` | `NotImplemented`, `NotSeparablyReachable` | capability |
| `:284`, `:295` | `UnsupportedSeedKind`, `UnsupportedBoundKind` | capability |
| `:421`, `:434` | `BoundTermNotApplicable`, 32-bit slot / not admitted | capability |
| `:447`, `:454` | `FilterNotApplicable`, act does not admit the slot | capability |
| `:355`, `:370`, `:381`, `:389` | `FilterNotApplicable`, **this door does not yet apply** | capability — reads no declaration |
| `:777` | `SectionNotAvailable` | capability — reads `ReturnSpec::ADMITTED_SECTIONS`, which can widen |
| runtime | `EmbeddingUnavailable` | neither, by design |

**Two variants straddle the seam**, which is why the pinned set is over sites rather than variants:
`BoundTermNotApplicable` (negative is shape; range and admission are capability) and
`FilterNotApplicable` (four "not yet" sites and two "act does not admit" sites, all capability, but
for different reasons).

**A correction recorded so it is not re-derived.** It was claimed in-session that
`FilterNotApplicable` is split *within one variant* and so cannot be classified. That confused two
axes: on the permanent-versus-not-yet axis it is genuinely split, but on the shape/capability axis
all six of its sites are capability. Splitting the variant is **not required by this seam** and stays
out of scope — it can earn its way in when a caller is confused by it.

### ⟨4⟩ The twelve string refusals are promoted, and the timing is forced

`RefusalReason` carries `#[serde(rename_all = "snake_case")]` with `#[serde(untagged)] Other(String)`.
So `Other("dangling-reference")` reaches the wire as `"dangling-reference"`, while a promoted
`DanglingReference` reaches it as `"dangling_reference"`. **Nine of the twelve change spelling.**

Nothing consumes them today, because there is no door. So the promotion is free now and a breaking
change to a published `400` body the moment `/api/query` ships. It lands **before** the door.

This is also a repair the codebase already filed against itself. `disposition.rs:158-172`:
*"the server emits reasons its own `RefusalReason::is_known` answers `false` to, and those twelve
are kebab-case while every declared variant is snake_case — a client's vocabulary is two
conventions. Found in review; recorded rather than repaired, because promoting twelve strings to
variants is a wire change and **belongs with the door, not ahead of it.**"*

`Other(String)` stays, for its real purpose: a producer newer than the consumer.

### ⟨5⟩ The placeholder flip is what keeps `door_coverage` honest

`CALLABLE_FRAGMENTS` (`validate.rs:60-65`) maps `search_graph_expand` and `wayfind_region_scores` to
`__temper_unbound_act`, a function that deliberately does not exist. So `follow-from` and `survey`
**validate clean and then fail at execution** — today that is invisible, because nothing executes a
composition outside its own tests.

The flip — dropping both rows, so the two acts refuse statically as `NotSeparablyReachable` — is
usually framed as avoiding a `500`. It is more than that. `registry.rs:42-46` declares both acts
`DoorReach::Absent` at all three doors and promises they *"restore to `Serves` when that door
lands."* If the placeholder survives the door, **both `Absent` and `Serves` are false**: the acts
are reachable through the door and cannot answer. The flip is what makes `Absent` remain true.

Its own doc comment records the reason the rows were kept — *"preserves the beat-C behaviour their
tests pin"* — and that reason is now spent.

---

## The cut: five PRs, each with one story

| | Story | Touches | DB |
|---|---|---|---|
| **A0** | `temper search` can page | `temper-cli`, `registry.rs` | no |
| **A** | The refusal vocabulary becomes two vocabularies, and the placeholder stops lying | `temper-core` | no |
| **B** | The door opens, end to end | `temper-api`, `temper-cli`, `temper-client`, e2e | yes |
| **C** | The CLI can check a plan offline | `temper-cli` | no |
| **D** | The contract catches up to the code | `docs/api/query.openapi.yaml` | no |

B carries API and CLI together deliberately: the CLI is what drives the e2e test, so interop is
demonstrated in the PR that creates it rather than in a later one.

### A0 — `temper search --offset`

Independent of the door and pre-existing. `/api/search` the **route** has always accepted offset
(`SearchParams.offset`, `api.rs:63-64`), which is why `Door::Api` and `Door::Mcp` declare
`terms_unreachable: []`. The gap is exactly one missing clap flag.

Files: `cli.rs` (the flag), `main.rs` and `commands/search_cmd.rs` (threading),
`actions/search.rs` (`CliSearchArgs` gains the field; `..SearchParams::default()` supplies `None`
today). **No `temper-client` change** — it takes `SearchParams` whole.

Then `registry.rs`: three `unified_doors(vec![BoundTerm::Offset], …)` become
`unified_doors(vec![], …)`.

The tests are already waiting for this. `temper_search_still_cannot_page` in
`crates/temper-cli/tests/act_door_coverage_cli_terms.rs` asserts `!flags.contains("offset")` and
fails with a message naming both edits — including that
`the_cli_cannot_page_the_find_acts_and_that_is_declared` in `registry.rs` compares the declaration
to a literal *"and will not notice on its own."* Landing A0 means letting that tripwire fire and
following it.

**The axis gains content rather than losing it.** Tier 2 derives the shortfall from clap
(`the_cli_term_shortfall_is_what_clap_actually_lacks`), so with `terms_unreachable: []` it now
requires the parser to actually carry `--limit` *and* `--offset`.

### A — the vocabulary and the flip

1. Split `validate.rs` (2,045 lines) into `shape` and `capability` per ⟨3⟩. `validate()` composes
   them and keeps its signature and behaviour: **every** refusal, never the first.
2. Hold the seam with a source scan in the family of ADJ-9d tier 1 — assert the shape module's
   source contains no `registry` / `declaration` / `search_family` reference. The module boundary is
   the design; the scan is what stops a later edit reaching across it.
3. Promote the twelve `Other(_)` strings to variants per ⟨4⟩. Regenerate the query JSON-Schema
   fixtures and `query.ts` in-commit.
4. Flip the placeholder per ⟨5⟩.

**Test fallout, measured rather than estimated:** 32 references to `ActName::FollowFrom` /
`ActName::Survey` in `validate.rs`, 2 in `crates/temper-substrate/tests/query_plan_compile.rs`. The
15 in `registry.rs` are declaration tests (`accepts_bounds`, quantity ranges) that reachability does
not touch.

The fallout has a known shape because it reverses a decision already on record. Beat B grounded its
legal-plan examples on `follow-from`/`survey` **specifically because the find acts were then
unreachable**; beat D made the find acts reachable and nobody moved the examples back. A does that
move. Each test's actual subject is unchanged — only the act it is expressed over.

### B — the door

**API.** `POST /api/query` in `gated_routes()` (`crates/temper-api/src/routes.rs:43`) —
`require_auth` + `require_system_access`, like every other content-touching route. There is no open
auth question: every route is authenticated, with two exceptions in the whole project
(`/api/health`, the Slack OAuth callback).

Handler: deserialize `Composition` → `validate()` → on refusals, `400` carrying **all** of them in
`ErrorBody.error.details.refusals` → otherwise `run_composition(pool, principal, &validated,
caller_embedding)`, which already answers in `QueryResponse`'s shape
(`temper-services/src/backend/query_read.rs`). `openapi.json` regenerates in-commit — the
`generated-artifacts` gate covers it.

**CLI.** `temper query`, transport only. Plan source mirrors `temper resource update`'s body-source
precedence rather than inventing one: `--plan @<path>` wins, `--plan -` always blocks-reads stdin,
implicit non-TTY stdin is auto-detected. A missing plan is an error — unlike `update`, there is no
case where absent input means "no change requested".

Output is `QueryResponse` through the existing `--format json|toon` machinery. **The trace always
rides**, never behind a flag: `composition-is-legible` is the property the door exists to deliver,
and a trace you have to ask for is one most callers will not have.

**`door_coverage`, the axis that genuinely moves.** `find-exact` and `find-about-within` declare
`bounds_unreachable: [IdKind::Resource]`, commented *"unreachable from every caller … no door's
params carry a resource-id list."* `POST /api/query` is a door whose params carry a resource-id list
— that is what `StageInput` and `p_bound_ids uuid[]` are. So the entry becomes false at Api and Cli
and stays true at Mcp, which ⟨2⟩ defers.

That breaks `unified_doors()`'s shape exactly as its own comment predicted: *"`bounds_unreachable`
is the same at all three doors and so is passed once… A door-varying shortfall would need the
per-door literal these acts no longer share."* B gives the helper a third argument; three call
sites, and it still earns its keep on the terms axis.

**e2e** drives CLI → API → DB in one test. That is why API and CLI are one PR.

### C — `temper query --check`

Runs `validate_shape` only. No network, no declarations, exit code from the refusals. Its
disclosure is that it reports expressibility and says so — it cannot speak to what the server has
implemented and does not try.

### D — the contract

The recorded lag: the canonical worked example is now a `400` (it uses `edge_filter`); `Extent` and
`IdProvenance` transcribed with the wrong tagging; `located_at` promised and structurally
unfillable; `Composition.bounds` read by nothing; one header block contradicting a later one.

Per the standing ruling the contract is **provisional** and alignment is bidirectional, so D
adjudicates case by case rather than conforming code to the yaml.

---

## Non-goals, named so they are not rediscovered as good ideas

- **The MCP tool** — ⟨2⟩, deferred with the consolidation view.
- **Property predicates** (Task 10b) and **the edge-provenance spike** (Task 11, which is what
  actually unblocks `follow-from`). The refusals at `validate.rs:378`/`:386` stay.
- **The generative `EXPLAIN` harness** (Task 14) and **the survey redesign**.
- **The visibility-hoist strategy** — still behind its seam, still awaiting the portable
  visibility-cost probe (`019fddc6-aace-7db0-a14d-5c610bc6506b`). B must not commit one.
- **A caching layer for the visibility gate** — deferred with its precondition named; no beat here
  may assume one.
- **Splitting `FilterNotApplicable`** on the permanent/not-yet axis — real, not required, ⟨3⟩.
- **Rate-shaped axes** — inherited open. One compiled statement concentrates risk in the query plan.

`promote_admin`'s gate (`access_service.rs:592`) is ruled and rides in whichever PR touches
`access_service.rs` first, or its own one-line commit. It is consistency, not exploitability; the
inertness argument is already answered and is not re-opened.

## What this does not claim

**No answer-quality witness is taken, and this design does not create one.** The standing caution
holds. Every clause of the frame register remains `declared-uncovered`. This ships a door: it is the
first thing in the arc that *could* be judged by whether a question gets answered, which is worth
noting without claiming.
