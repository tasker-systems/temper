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

`[corrected against the shipped code — 2026-08-12]` The block below was drafted before the split
landed and was false in three ways: there is no `validate_capability`, capability does not take
`&[ActDeclaration]`, and the composition is not a concatenation of two whole passes.

```
query::validate::shape        fn validate_shape_indexed(&Composition)
                                  -> (Vec<PlanRefusal>, Option<Vec<StageNode>>)
                              the module does NOT `use super::registry`
                              the order rides out so `validate` and `shape` cannot sort differently

query::validate::capability   fn validate_stages(&Composition,
                                                 &BTreeMap<&str, &StageNode>,
                                                 &mut Vec<PlanRefusal>)      (capability.rs:60)
                              fn validate_returns(&Composition,
                                                  &mut Vec<PlanRefusal>)     (capability.rs:79)
                              it imports `declaration` directly (capability.rs:34) rather than
                              taking `&[ActDeclaration]` — nothing supplies a family per call

query::validate               fn validate(&Composition)
                                  -> Result<ValidatedComposition, Vec<PlanRefusal>>
query::validate               fn validate_shape(&Composition) -> Vec<PlanRefusal>
                                  the public expressibility-only entry point; PR C's first caller
```

**Capability's two halves are gated differently, and that was a ruling rather than an
implementation detail.** `[decided — 2026-08-12, Pete]` `validate_stages` reads the stage graph, so
`validate` runs it only over a plan that topologically sorts. `validate_returns` compares each
entry's `with` against a constant and never looks a stage up, so it runs **whatever the plan's
shape** — gating it on the topology would take a refusal away from a cyclic plan that used to
receive it, against this module's own rule that `validate` returns every refusal rather than the
first. The refusals accumulate into ONE `Vec` by `&mut`, which is why neither pass's findings can
outrank the other's.

**The import is a necessary guard, not a sufficient one.** `[corrected — 2026-08-12, during plan
grounding]` A pass that cannot reach `declaration()` cannot raise `NotSeparablyReachable`, and that
much matters: version skew is structural here, not hypothetical, since CLAUDE.md's *"Release ≠
deploy"* means a released CLI carries a `search_family()` older than the server's, and
`CALLABLE_FRAGMENTS` — which decides `NotSeparablyReachable` — is precisely what beats D, 10b and 11
keep widening.

But "reads the declaration" and "is a capability refusal" **come apart**, and they come apart on
exactly the refusals that move. Six sites read no declaration at all and are nonetheless pure door
capability. `[corrected — 2026-08-12, Task 2]` An earlier draft of this paragraph said *"their own
detail strings say so"* and then quoted two of them. Only two say it outright; the rest are
capability for a reason that has to be argued rather than read off:

| Site | Detail string | Why it is capability |
|---|---|---|
| `validate.rs:381` | *"this door does not **yet** apply property predicates"* | says so — Task 10b retires it |
| `:389` | *"this door does not **yet** apply edge filters"* | says so — Task 11 retires it |
| `:355` | *"this door does not apply the `{field}` narrowing"* | a compiler slot that does not exist yet, not a permanent property |
| `:370` | *"this door's doc-type narrowing holds exactly one value"* | a fragment's parameter shape, which a later fragment can widen |
| `:777` | *"`{section}` is not a section this door hydrates"* | reads `ReturnSpec::ADMITTED_SECTIONS`, which can widen |
| `:311`, more than one | *"the anchor pair … holds exactly one id"* | `[added — 2026-08-12]` the same class as `:370` and a different parameter: the fragments' `(anchor_table, anchor_id)` pair, retired by an `anchor_ids uuid[]` |

The distinction matters because four of the six read like permanent structural facts and are not.
An import scan would let all six sit in the shape pass, and a stale client would then refuse plans
a newer server runs — the failure this seam exists to prevent.

**So the rule is stated positively, and guarded twice.** The shape pass may raise only refusals that
cannot change without a change to the published wire contract. Guard one: the shape module does not
import `registry` (source scan). Guard two: **how many times the shape pass emits each
`RefusalReason` is pinned by a test** — a table of reason → count, in the family of
`the_cells_tier_one_cannot_discriminate_are_exactly_these` — because the classification of the sites
above is a judgment, and a judgment needs a pin, not an inference.

`[corrected — 2026-08-12, final review]` This said *"the **set** of `RefusalReason`s the shape pass
can emit"*, which is the formulation the implementation proved insufficient and then abandoned; the
plan, the module header and the test itself were all moved to counts and this sentence was not. Two
variants are emitted from BOTH passes (below), so over a set a capability site migrating into shape
changes nothing — the reason is in the set already, and the guard sits green through exactly the
defect it exists to catch. Over counts, shape's tally for that reason goes 1 → 2 and it fails.

The classification below is **per site**, derived by reading each one — not per variant, because two
variants straddle the seam.

**Every bare `validate.rs:NNN` coordinate in this document — both tables in this section, and the
citations in ⟨2⟩ and in Non-goals — is a PRE-SPLIT coordinate, and is retained deliberately.**
`[noted — 2026-08-12]` The table IS the classification's evidence, and it was derived by reading the
2,045-line `validate.rs` that PR A deletes; renumbering
it against the successor files would silently restate the evidence as something it was not. No
reader can open `validate.rs:311` today — the module directory that replaced it is
`crates/temper-core/src/types/query/validate/`, whose `mod.rs`, `shape.rs` and `capability.rs`
carry every site below, each under a header that argues its own side of the seam. Read the row for
the classification and the module for the code.

| Site | Reason | Class |
|---|---|---|
| `validate.rs:657, :669, :684, :696, :705, :729, :754, :765, :796` | the nine topology `Other(_)` strings | shape |
| `:504`, `:512` | `Other("empty-property-key")`, `Other("empty-contains")` | shape |
| `:227` | `Other("unknown-act")` | shape — **rewritten** as `matches!(inv.act, ActName::Other(_))`; `ActName` is open (`act.rs:45-46`), so this is caller-reachable. **Kept in shape with a declared remainder, the costlier of two — see below** |
| `:488` | `MissingIntention` | shape — hardcoded `matches!` on three act names, never consults `search_family()` |
| `:272` | `MissingProvenance` | shape — `kind == Region && provenance.is_none()`, pure plan inspection |
| `:311`, zero ids | `AnchorTakesOneId` | shape — **on direction of failure, not impossibility**: admitting it today silently WIDENS the question, which is a correctness problem; refusing it costs a stale client only an empty answer. See the straddle bullet below |
| `:311`, more than one | `AnchorTakesOneId` | capability — **`[split — 2026-08-12, Pete]`**, see below |
| `:497` | `UnknownFilterValue` | shape — `PropertySubject::Other(_)`, a closed vocabulary in the schema |
| `:408` | `BoundTermNotApplicable`, negative value | shape — a count below zero is malformed whatever the act. Said as *"a row count"* until `[2026-08-12]`, which is false of `BoundTerm::Regions`, a funnel width (`scalars.rs:47`) |
| `:238`, `:246` | `NotImplemented`, `NotSeparablyReachable` | capability |
| `:284`, `:295` | `UnsupportedSeedKind`, `UnsupportedBoundKind` | capability |
| `:421`, `:434` | `BoundTermNotApplicable`, 32-bit slot / not admitted | capability |
| `:447`, `:454` | `FilterNotApplicable`, act does not admit the slot | capability |
| `:355`, `:370`, `:381`, `:389` | `FilterNotApplicable`, **this door does not yet apply** | capability — reads no declaration |
| `:777` | `SectionNotAvailable` | capability — reads `ReturnSpec::ADMITTED_SECTIONS`, which can widen |
| runtime | `EmbeddingUnavailable` | neither, by design |

**Exactly two variants straddle the seam**, and that is why the pin is over sites rather than
variants:

- **`BoundTermNotApplicable`** — its negative-value site is shape; its 32-bit-range and
  not-admitted sites are capability.
- **`AnchorTakesOneId`** — its zero-id site is shape; its more-than-one site is capability.
  `[split — 2026-08-12, Pete]` One site used to refuse `ids.len() != 1` and so conflated two
  claims. *Supplied several* is refused only because today's fragments take an
  `(anchor_table, anchor_id)` PAIR; an `anchor_ids uuid[]` retires it. That is structurally the
  same check as `f.doc_type.len() > 1`, which this table already classifies capability as *"a
  fragment's parameter shape, which a later fragment can widen"* — and the variant's doc already
  called the mismatch *"an open cardinality gap"*, i.e. explicitly not-yet. Both arms keep the same
  `RefusalReason` and both stay refusals of `validate`; what moves is only which pass raises them.

  **The zero arm's reason is DIRECTION OF FAILURE, not impossibility.** `[corrected —
  2026-08-12, re-review]` The two arms fail opposite ways. Admitting a zero anchor *today* would
  drop the scope and answer a **wider** question than the caller asked — a silent widening, which is
  a correctness problem rather than a capability one. Refusing it costs a stale client nothing,
  because the plan it refuses would have returned nothing anyway. The many case is the reverse:
  refusing it is what costs, the moment a fragment can take the set.

  The first draft of this bullet argued impossibility instead — *"an anchor has no `'{}'`/`NULL`
  pair, so no fragment change retires it, and the variant's own doc says so"* — and that is **false
  in two ways**, recorded so it is not re-derived. `disposition.rs:74-77` makes that statement about
  today's `(anchor_table, anchor_id)`, not about all fragment futures, so it does not argue what it
  was cited for. And the widening invoked one paragraph up falsifies it directly: an
  `anchor_ids uuid[]` gives an empty anchor exactly the `'{}'` = bounded-to-nothing meaning
  `IdKind::Resource` already carries — `query_plan.rs` binds a caller resource array unrefused at
  any length, zero included, and `query.openapi.yaml`'s `IdSet.ids` sets no `minItems`. **Under that
  widening both arms retire, not one.**

`[corrected — 2026-08-12, Task 2 re-review]` An earlier draft also said *two*, but pairing
`BoundTermNotApplicable` with `FilterNotApplicable` — which straddles a **different** axis. All six
of that one's sites are capability; what differs between them is whether the limitation is permanent
or not-yet, which is a distinction about retirement, not about which pass may raise it. That
conflation is how the guard nearly got specified as a set: "two variants straddle" sounds like a
statement about the seam and, said of `FilterNotApplicable`, is not.

**Guard two's expected table does not move when a variant is split this way**, and the reason is
worth stating because it looks like an omission. The table pins what SHAPE emits, per reason. Shape
emitted `AnchorTakesOneId` from one site before the split (the `!= 1` arm) and from one site after
it (the zero arm), so its count stays 1 and the entry is unchanged. Run rather than assumed: the
three guards were green on the split without touching the table. What the guard therefore CANNOT see
is which of the two arms shape holds, so
`shape_refuses_an_empty_anchor_and_leaves_the_multi_id_one_to_capability` in `validate/mod.rs` pins
that directly — it asserts `validate_shape` alone raises the empty case and does NOT raise the
many-id one, while `validate` still refuses both.

**`UnknownAct` stays in shape, and carries the costlier of the seam's two declared remainders.**
`[widened from "the one" — 2026-08-12, re-review]` Correcting the zero-anchor arm's reason from
impossibility to cost-asymmetry (above) means that arm can *also*, in principle, fire against a
widened server — so "one remainder" became false the moment the true reason was written, and the
count is restated rather than left to be rediscovered. **The two are not equivalent, and the
difference is why both are tolerable:** `UnknownAct` refuses a plan the newer server would have
**answered**; the zero anchor refuses one that would have returned **nothing**. Blast radius, not
kind, is what separates them.

`[decided — 2026-08-12, Pete]` `ActName` is open and grows with `search_family()`, so the direction that bites is
GROWTH: when an eighth act is declared, a released CLI whose binary predates it deserializes that
name into `ActName::Other` and `validate_shape` refuses `unknown_act` for a plan the current server
would run — the exact failure this seam exists to prevent, inside the pass that is supposed to be
immune to it. It stays anyway, because catching a **misspelled act name** offline is worth more than
the rare stale-binary case, and because the two are textually indistinguishable: nothing at that site
can tell `find-abuot-within` from an act that shipped last week. So the refusal's detail names both
readings instead of asserting the wrong one — *"`{raw}` is not an act this binary knows — check the
spelling, or update if your server is newer than it"* — and `shape.rs` carries the remainder in
prose beside the check. Moving it to capability is the alternative, and was declined rather than
missed.

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

`CALLABLE_FRAGMENTS` (`validate/mod.rs:75-78` since the split; `validate.rs:60-65` when this was
written) maps `search_graph_expand` and `wayfind_region_scores` to
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

### ⟨6⟩ `/api/search` accepts a resource bound, and the door-varying shortfall never happens

`[decided — 2026-08-12, Pete]` Planning PR B surfaced what looked like a structural first: `/api/query`
would be the only door able to supply a resource-id set, so `bounds_unreachable: [IdKind::Resource]`
would go empty at Api and Cli while staying true at Mcp, and `unified_doors` — which passes that axis
once for all three doors — would need a per-door literal.

**The premise was false, and it was an assertion rather than a finding.** This document said
`/api/search` *"cannot take a resource bound and will not"*; the second half described no invariant.
Measured instead:

`[table corrected — 2026-08-12, A1 Task 1]` The first draft omitted `query_find_wide` and concluded
the wide twin *"was never written."* It exists. Read from `pg_proc`, not from a grep:

| Function | args | `p_bound_ids`? | Gated? | `proconfig` | Called by |
|---|---|---|---|---|---|
| `search_exact` | 7 | no | yes | — | `/api/search`, until A1 |
| `search_wide` | 8 | no | yes | `hnsw.ef_search=200` | `/api/search`, until A1 |
| **`query_find_exact`** | 8 | **yes** | **yes** | — | nothing, until A1 |
| **`query_find_wide`** | 9 | **yes** | **yes** | **`hnsw.ef_search=200`** | nothing, until A1 |
| `__temper_ungated_find_exact` | 9 | yes | no | — | `/api/query`'s compiler |
| `__temper_ungated_find_wide` | 10 | yes | no | `hnsw.ef_search=200` | `/api/query`'s compiler |

**Both twins shipped already** — the exact one in `20260810000010`, the wide one in
`20260808000030` — gated, bound-accepting, and uncalled. So **A1 writes no SQL at all**; it repoints
two call sites. And `__temper_ungated_find_wide` already branches
`IF p_anchor_id IS NULL AND p_bound_ids IS NULL THEN <top-k> ELSE <exhaustive>`, so a bound routes to
the exhaustive path on its own — the correctness rule is served and no new semantics are needed.

**Why the error was more than a wrong row.** The plan derived from this table told an implementer to
write `query_find_wide` by mirroring `query_find_exact`, which carries no `SET` clause. A
`CREATE OR REPLACE` in that shape would have **silently dropped `hnsw.ef_search` from 200 to the
default 40**, narrowing every ANN draw on the wide arm — a search-quality regression with no failing
test and no error. It was caught because the implementer checked `pg_proc` before writing, which the
plan instructed and the spec's author had not done: the missing row came from a grep for
`CREATE OR REPLACE FUNCTION`, and the migration says bare `CREATE FUNCTION`.

**A consequence for `served_by`.** Repointing the call sites moves what `/api/search` actually calls,
and `served_by` is documented as naming exactly that. So the find acts' `served_by` becomes
`query_find_exact` / `query_find_wide`, and `CALLABLE_FRAGMENTS`' keys move with them — they stay in
sync because the map is keyed on `served_by` by construction. The reachability gate forces this
rather than merely permitting it: its oracle scans production source for `FROM <name>(`, so a
declaration naming a function no longer called goes red. Nothing downstream reads the literal names;
`/api/query`'s compiler reads the map.

**So the axis goes empty at all three doors at once** (MCP takes the whole `SearchParams`, so it
gains the capability with it), `unified_doors` never grows a third argument, and the
`Door`-is-a-surface-not-a-route ambiguity never has an instance. The question is dissolved rather
than managed.

This does not blur ⟨*Search splits in two*⟩. A resource-id bound is a filter on **one act** —
`accepts_bounds: [Resource]` is the act's own declared affordance — not a composition. `/api/search`
gets the act's full narrowing surface; `/api/query` gets composition. That is a sharper split, not a
muddier one.

**It lands as its own PR before B**, mirroring A0: independent of the door, closing a declared
shortfall, and leaving B's `door_coverage` untouched.

## The cut: six PRs, each with one story

| | Story | Touches | DB |
|---|---|---|---|
| **A0** | `temper search` can page | `temper-cli`, `registry.rs` | no |
| **A1** | `/api/search` accepts a resource bound | `temper-substrate`, `temper-services`, `temper-core`, `temper-cli` | no — both twins already shipped |
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

**`door_coverage` is untouched here — A1 is what moves it.** `[superseded — 2026-08-12 by ⟨6⟩]`
This section used to say the bounds axis moves in B: `POST /api/query` is a door whose params carry
a resource-id list, so `bounds_unreachable: [IdKind::Resource]` would go false at Api and Cli while
staying true at Mcp, and `unified_doors` would need a per-door third argument.

That is no longer what happens, because A1 gives `/api/search` the same bound first. The axis empties
at all three doors before B opens, `unified_doors` keeps its shape, and B declares nothing new — the
find acts are already reachable at every door it touches. The prediction in `unified_doors`' own
comment (*"a door-varying shortfall would need the per-door literal these acts no longer share"*)
stands as written and simply never gets its instance.

**What B must still check:** that `no_door_can_supply_the_resource_bound_the_find_acts_accept` — or
whatever A1 renames it to — is still true after the door lands. A1 empties the axis on the strength
of `/api/search`; B adds a second route that also supplies it. Same declaration, one more reason.

**e2e** drives CLI → API → DB in one test. That is why API and CLI are one PR.

### C — `temper query --check`

Runs `validate_shape` only. No network, no declarations, exit code from the refusals. Its
disclosure is that it reports expressibility and says so — it cannot speak to what the server has
implemented and does not try.

### D — the contract

The recorded lag: the canonical worked example is now a `400` (it uses `edge_filter`); `Extent` and
`IdProvenance` transcribed with the wrong tagging; `located_at` promised and structurally
unfillable; `Composition.bounds` read by nothing; one header block contradicting a later one.

**Sixth, added by PR A `[2026-08-12]`:** two places say `follow-from` *"compiles to the
deliberately-nonexistent placeholder"* — `query.openapi.yaml:154-158` (the IMPLEMENTATION PENDING
header) and `:1897-1903` (the worked example's ADJ-2 note) — and `:1901` adds *"even without the
filter, the stage would fail loudly at Postgres."* ⟨5⟩'s flip makes all of that false: `follow-from`
and `survey` left `CALLABLE_FRAGMENTS`, so they now refuse **statically** as
`not_separably_reachable` and never reach Postgres at all. The yaml is deliberately **not edited by
PR A** — the contract is provisional and D owns it — so the correction is recorded here rather than
applied. Note it changes what a client copying the example should expect: `filter_not_applicable`
**and** `not_separably_reachable` on `neighbours`, all refusals at once.

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
