# The query-envelope contract, v0 — design

**Status:** design, approved in session 2026-08-03, **amended the same day** by the §9 census — the
currency became a typed `IdSet` (§3.1) and one declared foreclosure dissolved (§3.3). Ships nothing
by itself; it is the settled shape that task **T2** (`019fbddc-227f-7e41-8167-b9eb0db0a63e`) authors
against and task **T3** (`019fbddc-bf0b-74c3-b48f-c06c799dec04`) inverts into generated artifacts.

**Frame register:** [Every question to Temper is answered by a situated act — acts compose by piping,
no constant decides between them, and no composition
flatters](./019fbdb9-f287-79c0-aab6-efa0b1de12c8).

**Grounded by:** [Asking Temper — chain-shaped askers, situated acts, and the composition
discipline](./019fbd9b-2d28-7530-9da0-4515319d6688) · [What wayfind actually compares — five
incommensurable quantities](./019fbd21-ba77-7b83-b8d3-454d74bb8c7d) · T1's audit, in two parts:
[columns 1–3](./019fbe0f-762a-7ad1-81be-1e346a34ea0c) and [column
4](./019fbe09-d2c9-7c70-981c-d97a62a344cc).

## Provenance discipline

Every load-bearing claim below carries its evidence class, because this area's figures have rotted
before and two of the audit's own findings were left explicitly unresolved.

- **`[verified — 2026-08-03]`** — read first-hand this session against the working tree at
  `49b81668`, or against the dev database. These are re-checks, not citations.
- **`[audit]`** — carried from T1's inventory and not re-opened here. Unrefuted, not confirmed.

Three of the audit's claims were re-checked before being built on, two of which it had filed as
**unresolved**:

| Audit state | Check | Result |
|---|---|---|
| Unresolved: `anchors_selected` vs `anchors_won` | `crates/temper-core/src/types/api.rs:281` | **`anchors_selected`.** `anchors_won` does not exist `[verified — 2026-08-03]` |
| Unresolved: MCP tool count 61 or 62 | `#[tool(…)] async fn` in `crates/temper-mcp/src/service.rs` | **62 distinct.** The ledger family's absence claims rest on a 61-name list — one tool unaccounted for, bounded `[verified — 2026-08-03]` |
| Keystone: `p_scope_ids` exists, no door reaches it | `migrations/20260714000001_ingest_state.sql:186`, `:280`, `:245`; `crates/temper-services/src/backend/substrate_read.rs:542`, `:577-580` | **Confirmed.** Populated only by wayfind or a cogmap set; `SearchParams` carries no `scope_ids` field `[verified — 2026-08-03]` |

`20260714000001` is still the last migration defining `unified_search` — the 2026-08-01 migration
redefines only the candidate functions — so the audit's cited body is current
`[verified — 2026-08-03]`.

---

## 1. What this contract is for

v0 is a contract for the **search family only**. It declares the acts that family serves, the
currency that flows between them, the envelope that carries a composition, and the disclosures that
make a composed answer legible. The other four audited families are declared out of scope with
reasons (§8), and §9 records the standing requirement that they be **safely additive** later.

Four decisions were taken in design and are load-bearing for everything below.

1. **v0 declares the full act vocabulary, with a per-act `build_state`** — not only what ships.
2. **`build_state` is checked against the live router, both directions** — never mirrored beside it.
3. **Bound-presence selects the act; it does not parameterize one.**
4. **Per-stage disclosure lives in an envelope trace in the body**, not in a response header.
5. **The currency is a typed `IdSet`, tagged as data** — added by amendment after the §9 census
   (§3.1). The Rust typed-id vocabulary already existed but is `#[serde(transparent)]`, so it never
   reached the wire where the contract needs it.

---

## 2. The act vocabulary

The vocabulary is **asker-shaped, not mechanism-shaped**. An act names what the asker holds and what
counts as being served; the mechanic that currently serves it is evidence, not identity. Naming acts
after mechanisms is how an act name becomes wrong the moment the mechanism changes.

| act | asker holds | served by | mechanic `[audit + verified]` |
|---|---|---|---|
| `find-exact` | *I can quote the exact words* | the resource containing them | `search_fts_candidates` — `websearch_to_tsquery('english',…)`, `ts_rank(…,33)` |
| `find-about-anywhere` | *a concept, no exact words; search everything I can see* | material about it | `search_vector_candidates` unscoped branch |
| `find-about-within` | *a concept, plus a set to search inside* | material about it, within the set | `search_vector_candidates` scoped branch |
| `follow-from` | *a found thing; I want its neighbours* | the neighbourhood, typed edges | `search_graph_expand` |
| `survey` | *a question about what a scope knows* | the scope's shape, charter-framed | `wayfind_region_scores` Stage-1 |
| `substantiate` | *a claim; I want its defensibility* | standing, not relevance | — |
| `admit` | — | — | **anti-act** |

### 2.1 Why `find-about` is two acts and `find-exact` is one

The split is **earned by a measured asymmetry**, not chosen for symmetry.

`search_vector_candidates` has two branches that differ in three ways
`[verified — 2026-08-03, migrations/20260801000010_stage2_length_subsidy_kind_blind.sql:156-205]`:

| | unscoped | scoped |
|---|---|---|
| candidate pool | global top-`p_k` chunks (`LIMIT p_k`, `vector_k = 100`) | **every** current chunk of every scoped resource, no top-k |
| visibility applied | **after** the top-k, in `admitted` | **before**, in `scoped_res` |
| score aggregated over | the resource's full chunk set, re-derived | `ann` |

The middle row is the one that matters and neither research doc names it: in the unscoped branch,
**chunks belonging to resources the principal cannot see compete for — and consume — the 100 global
top-k slots, and are pruned only afterward.** So an unscoped vector arm's budget is partly spent on
invisible material. Adding a bound does not merely narrow; it removes a competition the caller never
knew they were losing. The migration comment shows this ordering is deliberate — applying the
predicates inside "would force a seq-scan and defeat `idx_kb_chunks_embedding`" — so it is a
documented performance trade, not a defect. It is nonetheless `visibility-is-never-presented-as-relevance`
running in reverse, undisclosed.

A parameter whose presence switches the algorithm is not a parameter. It is an act selector. This
applies the grounding research's own diagnostic — *when a constant's function is to make two
quantities comparable, that is evidence they are not* — one level up.

`find-exact` needs no such split because **the FTS arm has no candidate budget**: `search_fts_candidates`
takes `(p_principal uuid, p_query text)` and nothing else, and `unified_search`'s `fts` CTE applies
no `LIMIT` `[verified — 2026-08-03]`. With no top-k there is nothing to be crowded out of, so
bounding it post-hoc is membership-equivalent to bounding it up front.

**Corollary worth stating, because it constrains `find-about-within`:** the scoped branch **forfeits
the HNSW index by construction** — no top-k, chunks joined to a pre-filtered set. `idx_kb_chunks_embedding`
serves `find-about-anywhere` alone. That is the same fact as the visibility-ordering trade, seen from
the index side, and it is the second reason these are two acts rather than one act with a flag.

### 2.2 `build_state`

Three-valued, and every value is mechanically checkable. This is the whole point: a hand-maintained
`build_state` would be the `ADMIN_EVENT_TYPES` failure — a const listing four event types where the
registry carries six, with a test holding *its own second copy* so it can never red `[audit]`.

- **`served`** — exactly one door invokes this act alone.
- **`fused(host)`** — the mechanic runs only inside a named composite; the act has no door, the host
  has one.
- **`unbuilt`** — no mechanic exists.

Current state of the search family:

| act | `build_state` |
|---|---|
| `find-exact` | `fused(unified_search)` |
| `find-about-anywhere` | `fused(unified_search)` |
| `find-about-within` | `fused(unified_search)` |
| `follow-from` | `fused(unified_search)` |
| `survey` | `fused(unified_search)`, **output projected away** |
| `substantiate` | `unbuilt` |

**Nothing in the search family is `served` today. That is the finding, not an omission.**

`admit` carries no `build_state` because it is not an act (§2.3). A refusal has nothing to be built.

### 2.3 `admit` is an anti-act

Cold-start wholesale admission is visibility-shaped admission presented as relevance-shaped
selection — ~98% of wayfind scope arriving with no relevance signal `[audit]`. The contract declares
it as a **refusal**, naming the thing `visibility-is-never-presented-as-relevance` forbids.

Consequence, and the reason to spend a declaration on it: a future change promoting cold-start to a
real act must **delete an explicit refusal**, not quietly add a row.

---

## 3. The bounds currency, and dual-mode consumption

### 3.1 The currency is a TYPED id set

> **Amended 2026-08-03**, after the §9 census. The currency was originally specced as a bare
> resource-id set; that is insufficient, and the reason is not "some acts return regions."

```
IdSet { kind, provenance?, ids[] }
```

**Membership, never rank.** Only an `IdSet` crosses a stage boundary. Per-act `meta` is terminal —
produced, disclosed, never consumed as a later stage's input. That is what makes
`no-cross-act-ranking` structural rather than policed: a stage receives a *set*, so no ordering is
available to blend.

**Why the tag must be carried as data.** The Rust vocabulary already exists — `crates/temper-core/src/types/ids.rs`
defines **17 typed ids** via a `define_id!` macro, heavily used (`ProfileId` 1116, `ResourceId` 580,
`ContextId` 249, `CogmapId` 208, `EdgeId` 123, `BlockId` 100, `RegionId` 33, …), each carrying
`utoipa::ToSchema`, `ts_rs::TS`, `schemars(inline)`, and full sqlx `Type`/`Encode`/`Decode` **plus
`PgHasArrayType`**, so `Vec<RegionId>` already binds to `uuid[]` `[verified — 2026-08-03]`.

But the macro applies `#[serde(transparent)]`: **every typed id serializes as a bare uuid string.**
The module's own test states the intent — *"CogmapId and ResourceId cannot be compared with `==` —
different types — which is the point"* — and enforces it **in Rust only**. This contract is a *wire*
contract and jaq operates on *JSON*, so a Rust newtype buys the contract nothing at the seam that
matters: §4.3's chaining check would have nothing to check, and a jaq projection could move region
ids into a `bounds` field with no signal at authoring time or at runtime.

**Why a type alone is still insufficient — `provenance`.** Context regions and cogmap regions are
both `RegionId` and are **not interchangeable**: `graph_region_composition` gates on
`cogmap_readable_by_profile($2, reg.cogmap_id)`, and a context region's `cogmap_id` is NULL by
construction, so every id `temper context shape` produces 404s at the sole consumer of region ids
`[audit]`. A correct `kind: region` tag would still admit that chain and still 404. So for `region`
the tag must also carry **which anchor produced it** — the `HomeAnchor` shape
(`crates/temper-core/src/types/home.rs`). `provenance` is optional because `region` is the only kind
that needs it today, not because it is decorative.

### 3.1.1 The `kind` vocabulary

Closed against evidence by the §9 census — these are the kinds that travel as a **set** on the wire:

| kind | produced by | consumed as |
|---|---|---|
| `resource` | search results · list rows · lineage · `edges.peer_resource_id` · auditor sweep `finding_id` · slice and composition nodes · panorama orphans | **seed** (`search.seed_ids`, `slice.seeds`); **bound** (indirectly, via `p_scope_ids`) |
| `region` | `cogmap_shape` · `context_shape` · region metrics · panorama territories | **bound** (`GET /api/graph/regions/composition?ids=`) — cogmap-only |
| `cogmap` | `cogmap list` · `GET /api/steward/candidates` · invocations | **bound** (`search.cogmap_ids`, `resources.cogmap_ids`) |
| `context` | `contexts list` | **bound** (as an `@owner/slug` ref) |

**Produced but terminal** — no consumer takes them as a domain limit, so they belong in `meta` and
result rows, never in `bounds`: `event`, `block`, `profile`, `edge`, `correlation`, `invocation`,
`property`, `chunk`, `entity`, `team`, `lens`, `remote-source`. Some are consumed as a *single* id
(`edge` at `/api/relationships/{edge_handle}`, `lens` as a query param) — that is **addressing, not
bounding**, and the distinction is why they are excluded.

**`kind` is OPEN**, per §6.1 — a new kind is additive, which is what makes the four out-of-scope
families admissible without a breaking change. Each act declares which kinds it accepts for `bound`
and for `seed`; an unaccepted or unknown kind renders a typed **`refused`** (§5), never a silent
drop.

**Kinds are domain-named, not table-named.** `resource`, not `kb_resources`. This deliberately
diverges from `LedgerRefTarget`, whose `LedgerRefKind` renames every variant to its SQL table
`[verified — 2026-08-03, crates/temper-core/src/types/admin.rs:197-216]` — persistence leaking onto
the wire.

### 3.1.2 Six incumbent patterns, and why this establishes a seventh

The tree already says "a typed id" six ways, disagreeing on tag placement, vocabulary, typing, and
grain `[verified — 2026-08-03]`:

| pattern | placement | vocabulary | typed | grain |
|---|---|---|---|---|
| `HomeAnchor` | external enum | domain (2) | yes | per-id, **no wire derives — internal only** |
| `LedgerRefTarget {kind,id}` | field | SQL table names (9) | yes | per-id, on wire |
| `ElementKind` | **URL path** | `node`\|`edge` | yes, 400 on miss | per-id |
| `BlockProvenanceRow {source_kind, source_id}` | field | `resource`\|`event`\|`remote` | **no — `String`** | per-id, on wire |
| provenance **write** side | field | same three | yes, `{kind,value}` sum | per-id |
| SQL convention | sibling column | `anchor_table` text | no | per-row |

The last two are **the same concept in the same module, stringly-typed on the read side and a typed
sum on the write side** — the drift shape, already realized. `IdSet` is per-**set** rather than
per-id because every act produces a homogeneous set; per-id tagging would pay for a generality
nothing uses and would repeat the tag N times.

*Not undertaken here:* converging the six. This contract establishes one shape for the query
surface; retrofitting the incumbents is separate work with its own blast radius.

### 3.2 Two consumption modes, declared at the consuming stage

The shipped system already has both mechanisms and no way for a caller to choose between them
deliberately: `p_scope_ids` narrows, `seed_ids` widens `[audit — Tier-1 #3]`. v0 keeps one currency
and makes the **mode** explicit:

- **`bound`** — narrow to within this set.
- **`seed`** — grow from this set.

The **producer emits membership; the consumer declares the mode.** This fixes the Tier-1 finding
(`seed_ids` reads as "search within these" and does the opposite) not by renaming it but by making
the mode explicit where it is used — and it lands in the trace, so a reader sees *"stage 2 consumed
stage 1's 40 ids as seeds (expanding)"* rather than inferring it.

### 3.3 The chainability matrix

Read off the deployed signatures `[verified — 2026-08-03]`. Each cell carries its own `build_state`
and is gated (§6, gate 2). **Every ✅ below is `fused(unified_search)`, not `served`** — the mechanism
exists but is reachable only through the composite, which is §2.2's finding restated per-cell. ❌
marks a cell as `unbuilt` unless the row says *by definition*, which marks it excluded by the
vocabulary rather than missing from it.

| act | consumes **bounds** | consumes **seeds** | produces ids |
|---|---|---|---|
| `find-exact` | ✅ post-filter, membership-equivalent (no top-k on the FTS arm) | ❌ no mechanism | ✅ |
| `find-about-anywhere` | ❌ *by definition* — a bound makes it `-within` | ❌ | ✅ |
| `find-about-within` | ✅ **required**, genuine pre-filter (`scoped_res`) | ❌ | ✅ |
| `follow-from` | ❌ **unbuilt** — `search_graph_expand` takes no scope parameter | ✅ **required** (`kind: resource`) | ✅ `resource` |
| `survey` | ✅ **`kind: cogmap` / `kind: context`** — see below | ❌ | ✅ `region` *(today projected away)* |
| `substantiate` | `unbuilt` | `unbuilt` | `unbuilt` |

Signatures this rests on `[verified — 2026-08-03]`:

```
search_fts_candidates(p_principal uuid, p_query text)
search_graph_expand(p_principal uuid, p_seed_ids uuid[], p_depth int,
                    p_edge_types text[], p_gamma double precision)
wayfind_region_scores(p_principal uuid, p_lens uuid, p_emb vector, p_regions_n int,
                      p_anchor_table varchar DEFAULT NULL, p_anchor_id uuid DEFAULT NULL)
```

**Both motivating chains work today.** Exact-first-then-expand is `find-exact` → `follow-from` as
**seeds** (literally `p_seed_ids`). Wide-then-narrow is `find-about-anywhere` → `find-exact` as
**bounds**.

**One genuine foreclosure**, declared rather than discovered later:

**`follow-from` cannot be bounded.** *"Walk from these seeds but stay inside this set"* is
unstatable — which is exactly what "graph-walk my exact hits, but only within this context" needs.
`search_graph_expand` has no scope parameter, so this is a real SQL-level hole.

**`survey` chained-into was a foreclosure of the untyped currency, and the typed currency dissolves
it** `[amended 2026-08-03]`. `wayfind_region_scores` takes `(…, p_anchor_table varchar, p_anchor_id
uuid)` `[verified — 2026-08-03]` — an anchor, which is exactly `HomeAnchor`'s shape. Under a bare
resource-id currency that is unstatable; under `IdSet`, `cogmap_list` produces
`{kind: cogmap, ids: […]}` and `survey` consumes it directly. **The chain
`cogmap_list → survey` is expressible with no SQL change at all.** It was never a missing mechanism —
it was a currency that could not name what the mechanism already accepted.

---

## 4. The envelope

### 4.1 Base ⊕ per-act extension, on both sides

OpenAPI 3.1 `allOf` + `discriminator` on `act`:

```
IdSet:          { kind, provenance?, ids[] }                              (§3.1)

ActInvocation:  { act, bounds: IdSet?, bounds_mode, limit, offset }  ⊕  params<act>
ActResult:      { act, produced: IdSet, total, limit_effective, offset,
                  narrowed_by[], bounds_in, bounds_honored, bounds_withheld }
                                                                    ⊕  meta<act>
```

`produced` is an `IdSet`, so an act's output kind is declared and machine-checkable rather than
inferred from which act ran. `bounds` is an `IdSet` for the same reason on the way in, and
`bounds_mode` (`bound` | `seed`, §3.2) says how it is consumed.

Four rules make it chain predictably:

1. **Only an `IdSet` crosses a boundary** (§3.1).
2. **Every act-specific quantity that could narrow is an act *input*, not a post-filter.**
   `find-exact` accepts `min_lexical_rank`; `survey` accepts `min_region_salience`; `find-about-*`
   accept `min_affinity`. The narrowing happens inside the act, where the quantity is commensurable
   **with itself and nothing else**. The result discloses
   `narrowed_by: [{key, value, admitted, excluded}]`. This removes the *reason* to post-filter rather
   than pretending jaq can be prevented from it.
3. **When jaq post-filters anyway, the envelope says so.** Each stage's `bounds_in` carries
   provenance: `upstream(stage_k)` | `expression` | `caller`. If a jq step subselects between
   stages, the next stage's bounds no longer equal the upstream act's `produced` set, and the trace
   records that an expression produced them. Not forbidden — **disclosed**. This is
   `no-silent-question-substitution` holding at the one seam where it can actually be violated.
4. **Field names carry their act.** No bare `score`: `lexical_rank`, `vector_affinity`,
   `region_salience`, `graph_adjacency`. jaq can still mash them; the schema makes the category
   error legible instead of inviting it.

Two base details that fall straight out of the audit:

- **`total` is a typed sum, not a nullable** — `Known(n)` | `Unavailable{reason}`. Search has no
  total today and `matched` echoes the *post-clamp* count, so 50 rows is indistinguishable from a
  corpus holding exactly 50 `[audit]`. A nullable would reproduce the
  `is_stale`-on-a-never-materialized-map ambiguity one family over.
- **`limit_effective` sits beside `limit`** — the `regions_effective` pattern, which the audit
  singles out as *"a model of an honest knob"* sitting in the same response object as two silent
  clamps. The disclosure pattern already exists here; it was never applied to `limit` or `depth`.

### 4.2 Why the body, not a header

The incumbent `x-temper-search-diagnostics` header is a **deliberate** backward-compatibility choice
under issue #360, stated in near-identical terms on both surfaces
`[verified — 2026-08-03]`:

- `crates/temper-api/src/handlers/search.rs:11-13` — *"Kept out of the body so the `200` contract
  stays a bare `Vec<UnifiedSearchResultRow>` — older clients ignore the header, newer ones read it."*
- `crates/temper-mcp/src/tools/search.rs:32` — *"On the happy path this block is absent and the
  output is byte-identical to before."*

Three facts settle the v0 choice:

1. **A composition has N stages; a single header cannot carry N stages' disclosure.**
2. **The constraint that produced the header is legacy body compatibility, and a new versioned
   surface has no legacy body to protect.**
3. The body-envelope precedent already ships next door: `ResourceListResponse { rows, total, facets }`
   (`crates/temper-workflow/src/types/resource.rs:311-315`) `[verified — 2026-08-03]` — the one MCP
   discards.

The ASCII constraint reinforces it rather than causing it: non-ASCII header bytes are
percent-encoded at the serverless adapter, downstream of both handler and client, which is why the
e2e can never observe it (it drives a bare Axum server) `[verified — 2026-08-03, source comments]`.

### 4.3 Composition-level envelope

Five things ride it; four are the deltas the grounding research says Temper cannot inherit from
tasker-grammar unchanged.

**The principal is at invocation, never in the envelope.** Visibility applies inside each act's
execution — one known application point per stage. jaq reshapes what visibility admitted and never
sees the credential. An inert principal identifier may ride for trace legibility only.

**The intention is computed once and threaded.** The query embedding is computed at composition
start, carried, and **inspectable in the trace**, so every `find-about-*` stage provably interrogates
the same intention rather than re-embedding a mutated string. This closes a live ambiguity the audit
could not settle: its methods disagreed on which hop `--text-only` breaks at, and its stated
resolution was *"what would settle it: a distinct wire field."* The envelope settles it structurally
— if no intention is present, a `find-about-*` stage **cannot run, and that is a refusal**, not the
server quietly embedding on the caller's behalf (`crates/temper-cli/src/main.rs:1200-1204` declining;
`substrate_read.rs:641-655` doing it anyway) `[audit]`.

**No implicit fallbacks.** Stages reference their inputs explicitly. tasker-grammar's `resolve_target`
prev-else-context is ergonomic for handler authors and a flattering-degradation vector here: a stage
silently answering the raw query because upstream came back empty is *a different question answered*.
Empty upstream is a visible disposition, never a fallback.

**Contract chaining between stages.** Stage N+1's declared input schema is checked against stage N's
declared output. Since only an `IdSet` crosses, the check is small but no longer trivial: it compares
**kinds**, so a plan piping `kind: region` into an act that accepts only `kind: resource` fails at
authoring rather than at execution. That is the check a bare uuid array could not have supported.

**An `OutcomeDeclaration` per composition** — description plus output schema. The pocket outcome
register: a saved plan states its served-by in the act schemas' own terms, so `every-act-is-situated`
reaches named compositions and not only single acts.

### 4.4 Meta accumulation, and its budget

Two tiers, split by cost curve.

**Tier 1 — per-stage summary. Mandatory, never truncated, no knob turns it off.** `act`, what was
asked, `bounds_in`/`bounds_honored`/`bounds_withheld` as counts, `bounds_mode`,
`narrowed_by: [{key, value, admitted, excluded}]`, `total`, `limit_effective`, `disposition`. This is
O(stages) — a handful of scalars per stage — and it is what `composition-is-legible` actually
requires.

**Tier 2 — per-resource meta. Accumulated, budgeted, disclosed when capped.** This is the
O(results × stages) term. The invocation declares a level:

| level | retains | cost |
|---|---|---|
| `surviving` *(default)* | per-resource meta only for ids in the **final** result set, at each stage that touched them | bounded by the caller's own limit |
| `full` | every id at every stage, including ids dropped mid-composition — answers *"why did this resource drop at stage 2"* | O(results × stages); opt-in |
| `none` | Tier 1 only | — |

**An accumulation cap is disclosed, always.** `meta_truncated: {stage, retained, dropped}` rides the
envelope whenever a budget bites. This exists because of a measured counter-example: `ORPHAN_LIMIT = 50`
truncates with no response flag and — alone in that service — not even a server-side `warn!`, so the
caller receives a shorter list that reads as complete `[audit]`. The contract may decline to carry
detail; it may never do so silently.

---

## 5. Refusal and the four dispositions

Every stage resolves to exactly one:

| disposition | means |
|---|---|
| `answered` | rows returned |
| `empty` | honest zero — the question was asked and nothing matched |
| `withheld` | material exists; the asker's standing does not admit disclosure at this depth |
| `refused` | the act declines a well-formed question |

**`refused` is a typed variant in the contract**, so every door renders the same value. How a door
*transports* it — HTTP status, MCP error code — stays a door concern; the variant does not. This is
the answer to the audit's refusal-dialect divergence, where one condition renders as `404` /
`200 []` / `invalid_params` / `internal_error` / success-with-body-`null` depending on the door, and
where `context_shape`'s documented anti-oracle property turns out to be **a property of one door
rather than of the affordance** `[audit]`.

Two constraints carried in unchanged:

- **Disclosure depth is governed by the refused actor's standing relative to the refused scope** —
  inherited from the register's refusal-surface doctrine.
- **The composition declares its disposition toward a stage refusal before execution** — `halt` |
  `degrade-and-disclose`. The executor never improvises it. A degrade is disclosed by construction,
  since the trace entry exists whether or not a result does.

Preserved from the predecessor register: naming an anchor the principal cannot read yields zero rows,
never a leak; an empty scope is an honest empty-scope signal, not an error.

---

## 6. Versioning, and what T3 gates

### 6.1 Open and closed types

Stated rather than assumed, because it decides the growth path:

- The **`act` discriminator is open** — clients must tolerate an unknown act.
- The **`disposition` enum is closed** — clients must handle all four exhaustively.

So adding an act is *additive*; adding a fifth disposition is *breaking*. Widening a closed type
silently weakens every exhaustive match on it, and the growth this contract wants is new acts, not
new dispositions.

### 6.2 The semver table

| additive | breaking |
|---|---|
| a new act declaration | removing an act |
| a new optional param | `build_state` moving `served` → `fused`/`unbuilt` |
| a new `meta` field on an act extension | renaming or narrowing any field |
| a new trace field | a new `disposition` variant |
| a chainability cell moving `unbuilt` → `served` | a cell moving `served` → `unbuilt` |
| a new `IdSet` `kind` (the vocabulary is open, §3.1.1) | an act **dropping** a `kind` it accepted |

### 6.3 Five gates

Each checks a declaration **against reality**, never against a second copy of itself.

1. **`build_state` vs the live router**, both directions, plus the `fused(host)` clause — no door for
   the act, a door for the host. The third clause is what makes `fused` a fact rather than a
   euphemism, and it is the one that reds when an act quietly acquires a door.
2. **The chainability matrix vs the SQL signatures.** `follow-from` declaring it accepts bounds reds
   until `search_graph_expand` grows the parameter.
3. **Contract diff vs the semver table.** Unclassifiable changes are surfaced for deliberate
   classification, never auto-passed.
4. **Mechanic-body fingerprint vs `scoring_revision`.** Each act names the SQL function serving it;
   CI hashes the deployed definition; a changed hash without a revision bump reds. This exists
   because of a case the audit caught: `ts_rank` flag 32 → 33 dropped absolute `fts_score` by ~5×
   **with an unchanged shape and no wire version marker** `[audit]`. A shape differ is structurally
   blind to it; a body hash is not.
5. **Round-trip regression** — real responses deserialized against the generated schemas, in the e2e
   suite.

### 6.4 What no gate can see — named, not assumed covered

- **Gate 4 fingerprints an act's own body, not its callees.** A change inside `resources_visible_to`
  alters every act's semantics without moving a single hash.
- **Corpus-dependent drift with no code change at all.** `survey`'s `sal_norm` is a `percent_rank`
  whose window frame is *the asker's visible set* — measured, 382 of 385 regions score differently
  across two visible-anchor sets. Its output moves as the corpus moves and as *who is asking*
  changes. The candidate repair is measured (`PARTITION BY …, home_anchor_id` → **exactly zero**
  divergence) and **makes small anchors worse** by widening their salience ladder, so it is not
  obviously worth taking. Cited so the next reader does not re-run it.

---

## 7. `unified_search` is already a composition

Reading the CTEs in order, the shipped function is a hardcoded plan
`[verified — 2026-08-03, migrations/20260714000001_ingest_state.sql:239-285]`:

```
fts         → find-exact (unbounded)
vec         → find-about-* (bounded by p_context_id / p_scope_ids)
blend0      → 1.0·fts_norm + 1.0·vec_norm          ← THE SUM THE FRAME FORBIDS
seeds       → top-20 of blend0 ∪ caller seeds       ← auto_seed_n = 20, undisclosed
graph       → follow-from, seeded by the above
cand/corpus → union, then post-filter by scope/context/doc_type
```

Two consequences.

**The caller's graph expansion is auto-seeded from a blend they cannot see**, by a resident constant
no door reports. `follow-from`'s origins are decided by the exact violation `no-cross-act-ranking`
names.

**The migration question has a better answer than "keep it or break it."** The incumbent surface is
**expressible as the first named plan** in the new grammar, and writing it down is what makes
`blend0` legible as a defect rather than a design. That gives the plan library a non-hypothetical
first entry and gives T3 a regression target that already carries traffic.

*Named fork, not settled here:* when the incumbent `POST /api/search` and its diagnostics header
retire. The expression of that surface as a plan is settled; the timing is not.

---

## 8. The inexpressibility section

Machine-readable in the artifact, so stated silence becomes schema.

**Out of scope by decision** — the four unmodelled families (list/filter; shape/region/analytics;
graph/trail/lineage/evidence; ledger/invocation/introspection), roughly sixty affordances, pointing
at T1's inventory. Write paths: the acts are reads, and persisting a composition is ordinary resource
creation. The think-with tier: it requires a resident entity.

**Inexpressible by construction, and that is the point** — cross-act ranking. There is no well-typed
way to state it, which is `no-cross-act-ranking` having teeth rather than being a rule.

**Declared holes with no mechanism** — `substantiate` (no representation in search: not input, not
ranking term, not output field; the nearest thing, `GET /api/resources/{id}/evidence`, is
one-resource-at-a-time, has no batch form, and is MCP-absent — so `claims-carry-standing` stays
openly uncovered); `follow-from` **bounded**. *(`survey` chained-into was listed here before the §9
census and is no longer a hole — see §3.3.)*

**Not undertaken, with blast radius named** — converging the six incumbent tagged-id patterns
(§3.1.2) onto `IdSet`. This contract establishes one shape for the query surface; retrofitting
`LedgerRefTarget`, `BlockProvenanceRow`'s stringly-typed `source_kind`, and the `anchor_table`
convention is separate work.

**Declared as a refusal, not a gap** — `admit`.

**Open axes, inherited** — every rate-shaped axis: query volume, composition depth, agent cadence at
scale. Nothing here closes over rate, and no test explores it.

**Named forks** — the incumbent header's retirement timing (§7); the two gate blind spots (§6.4).

---

## 9. Standing requirement: the out-of-scope families must be safely additive

**Recorded as a requirement on this design, to be worked immediately after this spec** `[decided —
2026-08-03, Pete]`. The four families in §8 were scoped out to concentrate attention, **not** because
they are unimportant. Every one of them matters, and the envelope and act declarations designed here
must be able to admit them **additively** — without a breaking change to what v0 ships.

The mechanisms v0 already provides for this, which the follow-up work should test rather than assume:

- The **open `act` discriminator** (§6.1) makes a new act additive by construction.
- **Base ⊕ per-act extension** (§4.1) means a new family's acts bring their own `params` and `meta`
  without touching the base.
- The **chainability matrix** (§3.3) has room for new rows and columns, each gated independently.
- **`build_state`** lets a family be declared before it is served.

### 9.1 The base-currency question — ANSWERED `[2026-08-03]`

The open question was whether `resource_ids` is the right universal currency for families whose
natural products are region, edge, block, or event ids. **It is not.** The census was run and the
currency is now a typed `IdSet` (§3.1), with the `kind` vocabulary closed against evidence (§3.1.1)
and left **open** for additive growth.

Three things the census settled that were not visible when §9 was written:

1. **The Rust type vocabulary already existed** — 17 typed ids, heavily used — so this was lifting
   an existing vocabulary to the wire, not inventing one.
2. **`#[serde(transparent)]` meant it did not reach the wire**, so a Rust newtype could never have
   given the contract's chaining check anything to check. The tag had to become data.
3. **Typing dissolved one of the two declared foreclosures** — `cogmap_list → survey` needs no SQL
   change (§3.3). A currency that cannot name what a mechanism accepts manufactures holes that are
   not there.

### 9.2 The admissibility pass — RUN `[2026-08-03]`

Five risks were named as the things that could force a v0 change. **All five are closed, and none
forces one.** The pass was scoped to *"what would admitting the four families break?"* — deliberately
not to modelling them.

| # | Risk | Result |
|---|---|---|
| A | A family produces a **heterogeneous** id set, breaking per-set tagging | **Closed.** No producer emits one |
| B | A family needs a **bounds kind** beyond the four | **Closed.** None does |
| C | `provenance` needed for a kind other than `region` | **Closed.** No |
| D | One act carrying **two build-states** across families, breaking `ActDeclaration` | **Dissolved.** They are two acts |
| F | A currency that **isn't a uuid set** | **Closed.** None is |

**A — per-set tagging survives.** `AtlasSubgraph { nodes, edges }` is two homogeneous collections.
`TerritoryOverview.territories: Vec<Territory>` *admits* a mixed set — `Territory` carries its own
`kind` — but **both producers emit homogeneous sets**: `context_graph_service.rs:67` always
`TerritoryKind::Context`, `graph_service.rs:256` always `TerritoryKind::Region`
`[verified — 2026-08-03]`.

> **TRAP, and it must be named before the graph family is admitted: `TerritoryKind` is not an id
> kind. It is a rendering tint, and on one producer it is wrong about its own id.** The code says so
> in its own comment — *"Tint encodes the AXIS, not container-ness (spec D6). A goal container sits
> on the builder axis, so it is Context-tinted even though it is rooted at a goal"* — so that
> producer emits `Territory { id: <a goal's RESOURCE id>, kind: Context }`. Anyone later mapping
> `Territory → IdSet` by reading `kind` produces `{kind: "context", ids: [<resource id>]}`: a wrong
> tag that the chaining check would then **trust**. The mapping must read the *producer*, never the
> field.

**B — no new bounds kind.** The `group=<key>:<value>` drill token looked like a second currency and
is not: `CompositionTarget::Bucket { key, value }` resolves through `residual_member_ids` into
`Vec<Uuid>` seeds **before** the composition read `[verified — 2026-08-03, handlers/graph.rs:328-345]`.
So it decomposes into *an act that produces an `IdSet`*, not a new kind. A batch `substantiate` would
consume `IdSet { kind: resource }` — already in the vocabulary.

**C — no.** Block and edge ids are globally unique and need no anchor to address. `Territory.anchor_id`
is per-id in the type but **constant per response** in both producers (the path's cogmap or context),
so per-set provenance holds.

**D — dissolved; they are two acts, not one act twice.** `ShapeQuery { lens: Option<Uuid> }` — the
shape read takes **no query and no embedding**, and dispatches on `HomeAnchor`
`[verified — 2026-08-03, handlers/cognitive_maps.rs:38-40, :186-196]`. So *"show me this scope's
shape"* (query-free) and `survey`'s *"what does this scope know, near my question"* (query-relative)
are different asker-holds. **`ActDeclaration` needs no family or instance dimension.** The shape
family contributes a *new* act, which is additive under the open discriminator.

**F — closed.** Invitation tokens are not bounds-shaped and sit outside the currency; a context ref
is addressing sugar over `ContextId`; the element trail's `(kind, id)` is already a tagged id.

**Incidental finding, recorded because it is the audit's own class:** `TerritoryKind::Cogmap` is
**inert** — declared and constructed by no producer `[verified — 2026-08-03]`. T1 missed it because
it inventoried parameters and response *fields*, not enum *variants*.

### 9.3 Still genuinely open

- The four families' **act vocabularies** — this pass established they can be admitted, not what they
  are. The shape family's query-free scope act is the one whose existence is now certain.
- **Every non-read affordance.** The pass, like T1's, was read-only.
- Whether `Territory` and `NodeHome` should be *retired* in favour of `IdSet` — that is the six-pattern
  convergence (§3.1.2), still not undertaken.

---

## 10. Summary of the shape

Seven act declarations · a **typed, dual-mode** `IdSet` currency over an open `kind` vocabulary · a
two-tier envelope with a mandatory per-stage trace · four dispositions · five gates · the declared
silences of §8.

Three of these are genuinely new builds — the envelope and trace, the typed refusal variant, and the
gates. Everything else is **exposure and honest naming of mechanics that already exist**, and the §9
census sharpened that claim rather than weakening it: the typed-id vocabulary was already there and
already used, and typing the currency **removed** a declared hole (`cogmap_list → survey`) instead of
adding work.

### Gate 2 grows a clause

The chainability gate (§6.3) now checks kinds, not just presence: an act declaring it accepts
`kind: region` must be served by SQL that can actually take one. This is what would have caught the
region-composition dead end at declaration time rather than at the caller's 404.
