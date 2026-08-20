# The graph successor surface — one route, one kind, three declared bounds

Design for the successor to the Atlas, under goal
[The graph surface shows the reader's own material — derived structure organizes it and is never
mistaken for it](./019fbaac-96e2-7620-ace2-667a0f8ff000), whose grounding and design direction come
from decision
[Atlas pivot — derived structure organizes the view; it is never the subject](./019fbaa8-9eec-7aa0-a4e1-c6b933a60bf5).

The register states outcomes and names no mechanism. **This document is all mechanism**, which is
what a spec is for. Where it makes a claim about what exists today, the claim carries a `file:line`
citation or quoted output; where it goes beyond what exists, it says so.

**Working mockup: [`mockups/2026-08-20-graph-successor-surface.html`](./mockups/2026-08-20-graph-successor-surface.html)**
— all three entries of §2 rendered live, with the §3 mark vocabulary, the *why-these* readout and the
bound line. `/dev/atlas`'s fixtures could not serve it: they are shaped for territories and tiers,
and this surface has neither. Its own fixture uses real titles, doc types and edge kinds from
`@me/temper`; the cogmap on the second tab is illustrative in name only. Its palette encodes §3's
central rule — **magenta means derived, and it never appears as a mark** — so
`no-derived-thing-poses-as-authored` can be checked by looking.

---

## 0. The finding that makes this cheap

The successor needs **almost no backend change.** Every mechanic it requires shipped between
2026-08-14 and 2026-08-17, and the UI has never called any of it.

> **`[corrected — 2026-08-20]` One backend change turned out to be required, and it is a
> pre-existing defect rather than a cost of this surface.** This section originally claimed *"no
> backend change at all."* Grounding found that `survey` declares `discloses: vec![Disclosure::Region]`
> and **nothing delivers it** — no consumer outside `registry.rs`, no `region_id` anywhere in
> `query_read.rs`, and no region carrier on `StageTrace`. So the *why-these* readout of §3 cannot
> name a grouping. Filed as
> [survey declares a region disclosure that no code delivers](./01a01f21-c2ab-78b0-ada5-e8190d9c0814)
> and sequenced as **Beat 0**, ahead of everything here. The data is already returned by
> `__temper_ungated_survey` and dropped in the Rust assembly, so it is a carrier, not a computation.

- `/api/query` accepts a `Composition` and answers a `QueryResponse` —
  `crates/temper-api/src/routes.rs:166` routes `handlers::query::query`.
- `/api` is proxied wholesale to the upstream API host — `PROXIED_ROOTS = ['/mcp', '/oauth',
  '/.well-known', '/api']`, `packages/temper-ui/src/lib/server/proxy.ts:28`. `apiPost` exists at
  `packages/temper-ui/src/lib/server/api.ts:49`. So a SvelteKit server load can POST a composition
  with **zero new plumbing**.
- The whole contract is already generated into the UI —
  `packages/temper-ui/src/lib/types/generated/query.ts`, 1365 lines, `Composition`, `QueryResponse`,
  `StageTrace`, `Extent`, `ViaEntry`, `RegionHit`.
- And nothing in `packages/temper-ui/` calls `/api/query` or `/api/search`. The UI's graph reads are
  nine bespoke `/api/graph/*` endpoints in
  `packages/temper-ui/src/lib/server/graph-reads.ts:20-117`.

The Atlas was built before the composition contract existed. The successor is a client of it.

### The three mechanics, quoted

**`survey`** — `crates/temper-core/src/types/query/registry.rs`:

```rust
accepts_bounds:      vec![IdKind::Cogmap, IdKind::Context],
accepts_bound_terms: vec![BoundTerm::Regions],
bound_ceilings:      BTreeMap::from([(BoundTerm::Regions, 20)]),
produces:            Some(IdKind::Resource),
discloses:           vec![Disclosure::Region],
```

and `migrations/20260816000020_survey_act.sql`: *"survey produces RESOURCES, not regions; regions
become trace disclosure."* This is the pivot **enforced in the read**. A surface built on this act
cannot violate `no-derived-thing-poses-as-authored` by accident, because the act does not hand it a
region to draw.

**`follow-from`** — `registry.rs:392` carries `bound_ceilings: {Limit: 50}`, and its disclosure is
the whole graph:

> `via: Array<ViaEntry>` — *"How a walk reached this resource — one entry per edge it was reached
> by."* Each entry: `seed_id`, `source_id`, `target_id`, `edge_kind`, `label`, `polarity`.
> (`packages/temper-ui/src/lib/types/generated/query.ts:808ff`, `1295ff`)

Uncapped by design, and the reason is measured rather than assumed: *"`[measured on prod —
2026-08-14]` at the deliberate worst case (25 highest-degree seeds, depth 3) the whole walk holds
9,434 entries with 124 on one node, while the page that ships carries 125 in total and at most 9 on
any row."*

**`find-resources-with`** — `registry.rs:266-278`:

```rust
accepts_bounds:  vec![IdKind::Context, IdKind::Cogmap],
accepts_filters: vec![FilterField::Resource],
bound_ceilings:  BTreeMap::new(),          // no ceiling at all
produces:        Some(IdKind::Resource),
```

It takes no intention and orders nothing, so — per its own doc-comment — *"a stage running it cannot
appear in `returns`… rows with no ordering quantity have nothing to score them, and the assembler
would drop every one while reporting `disposition: answered`."* **It can only be piped.** That
constraint shapes §2.

### The cost finding that makes N-anchor fan-out viable

`crates/temper-services/src/backend/query_read.rs:162-168`:

```rust
fn texts_to_embed(c: &Composition) -> BTreeSet<String> { … }
```

> *"**Distinct query TEXT, not per stage** — … Two stages naming the same string must not pay ONNX
> twice; and they must not be able to receive two *different* vectors for one question."*

So **N survey stages sharing one question cost one embedding**, not N. Combined with unbounded union
arity (`validate/shape.rs:153-175` refuses only `inputs.len() < 2`, and caps *only* ordered ops —
`difference` — at two) and no stage-count limit anywhere in the validator, a fan-out across every
readable anchor is expressible today at one inference.

---

## 1. Route shape

**One route: `/graph/[owner]`. Everything else is query params.**

The reason is structural, not aesthetic. The anchor set is **0..N** — the unaddressed door binds
many, a named place binds one — and they are *the same screen with a different bound count*. A path
segment carries exactly one anchor, so `/graph/[owner]/[context]` would force two route shapes for
one screen, and would make entry depend on how the reader organized their work. That is the clause
`entry-does-not-presume-organization`.

| Param | Composition part | Absent means |
|---|---|---|
| `q` | the `Intention.query` on every survey stage | see §2 — depends on what `in` names |
| `in` | the bound `IdSet`s: `ctx:@owner/<slug>` / `map:<uuid>`, repeatable | every readable anchor, bounded and declared |
| `from` | `follow-from` seeds — resource uuids | the walk seeds from the upstream stage |
| `sel` | the detail rail selection | nothing selected |

**The invariant this buys: the URL is a projection of the composition.** No state on screen that the
URL does not describe; no param that does not name a part of the plan. A reader can copy a URL and
get the same answer, and the surface cannot hold a hidden mode.

**`in` carries a whole ref, not a bare slug.** `/graph/[owner]` names the reader whose graph this is,
but the anchors they may read are not all theirs — a team context is `+team-slug/<slug>` and is
routinely in reach. Scoping `ctx:` against the route's `[owner]` would make team contexts
inexpressible at the very door whose purpose is spanning every readable anchor, which is the clause
`cross-kind-relationship-is-reachable` broken by a URL convention. So the grammar is
`ctx:@owner/<slug>` / `ctx:+team/<slug>` / `map:<uuid>` — a cogmap needs no owner, being addressed by
uuid. Resolution belongs in `vault-url.ts` beside the builders, one parser, one authority.

`packages/temper-ui/src/lib/vault-url.ts` stays the single URL authority. `contextGraphHref`
(`vault-url.ts:23`) is rewritten there and every caller follows. `/vault/[owner]/[context]/graph` is
already a 308 shim onto `contextGraphHref`
(`packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/graph/+page.server.ts:10`) and keeps
working unchanged.

**This section is what unblocks task
[Atlas nav: in-page table⇄graph toggle](./019f4a25-75ab-7650-b602-eb68594de2fc)**, whose
`[re-triaged — 2026-08-17]` note holds it *"deliberately sequenced after the successor surface's
route shape is decided… not blocked on the successor being built — only on its doors being named."*
The doors are named here. `sel` keeps the established ephemeral-selection posture
(`replaceState`, documented at `AtlasPage.svelte:13`); `q`, `in` and `from` push history, which is
what B1 asked for.

### Stale addresses, and how much of the problem dissolves

The register's refusal face requires that *"a reference to derived structure that no longer exists
is declined **as a stale address**, and the reader is landed somewhere real."* Its
`[amended — 2026-08-17]` note escalated this: `survey` **discloses `region_id` to the caller**, and
region identity is not durable — `assert_region`
(`crates/temper-substrate/src/write.rs:673`) reuses the row when the member set is unchanged and
**mints a new id otherwise**.

Under this route shape, **no region id enters any URL**:

- `in` carries anchors — `kb_contexts` / `kb_cogmaps` rows, durable.
- `from` carries resource uuids — `kb_resources` rows, durable.

A `from` seed that no longer resolves is a **deleted resource**: an honest 404 about the reader's own
material, not a stale address for derived structure. This deletes today's
`compositionOrPanorama` / `contextCompositionOrPanorama` catch-and-redirect pair
(`graph/[owner]/+page.server.ts:44`, `:73`), which exist precisely because region ids are in
URLs today.

**What remains, stated rather than waved away:** a `region_id` disclosed in the trace and rendered in
the *why-these* readout (§3) can name a region re-minted between the read and a click on it. That is
the one surface the refusal-face clause still governs, and it is a panel affordance, not a route.
Requirement: a readout region reference that no longer resolves renders as *"this grouping has been
re-derived"* — never as an error, and never as the reader's mistake.

---

## 2. The three entries

All three produce the same kind of screen. They differ only in how the seed set is obtained.

### 2.1 Unaddressed, with a question — `?q=X`

```
s₁ … sₙ :  survey(bound = anchorᵢ, intention = X)      ceiling: regions ≤ 20, default 3
u       :  union(s₁ … sₙ)
w       :  follow-from(seeds = u)                       ceiling: limit ≤ 50
returns :  s₁ … sₙ  and  w
```

The surveys and the walk are **returned as separate keys**, which is not a convenience — it is the
contract's own protection. `QueryResponse.returned` is *"a map rather than a list, and that is the
structural half of `no-cross-act-ranking`. Arms are keyed separately and there is no merged ordered
list anywhere for two acts' rows to fall into"* (`query.ts:696ff`). The surveys are scored by
`region_score`, the walk by `graph_score`; the scales are different and one of them is `Unbounded`.
**The surface must not merge them into one ranked list**, and must not present either number as a
score to the reader — see §5's open ruling.

This entry is `cross-kind-relationship-is-reachable` **by construction**: one answer spanning context
anchors and cogmap anchors, with no container-kind axis anywhere in it.

Both stage families are legitimately returnable: `survey` produces `Resource` and orders by
`region_score`; `follow-from` produces `Resource` and orders by `graph_score`. Combinators can never
be returned (`query.ts:1135ff`, `produced_ids`: *"a combinator can never be a returned stage at
all"*), which is why `u` is a pipe and not an output.

### 2.2 A named cogmap, no question — `?in=map:<uuid>`

The cogmap's telos is a **declared resource**: `kb_cogmaps.telos_resource_id`
(`migrations/20260624000002_canonical_functions.sql:689`), surfaced by
`GET /api/cognitive-maps/{id}/analytics` (`crates/temper-api/src/routes.rs:142`) and flagged
`is_telos` by `cogmap_show` (`migrations/20260724000040_cogmap_foundations_read_gate.sql:18`).

So: read the telos resource's prose, use it as `X`, then run §2.1 with N = 1. The surface **says**
what it did — *"surveying under this map's charter"* — with a link to the charter resource. Surveying
a telos-governed distillation under its own telos is the map answering the question it exists to ask.

### 2.3 A named context, no question — `?in=ctx:@owner/<slug>`

```
m :  find-resources-with(bound = context)     no ceiling; pipe only
w :  follow-from(seeds = m)                    ceiling: limit ≤ 50
returns : w
```

A context has **no declared telos** — only a `telos_centroid` vector.
`migrations/20260712000060_context_telos.sql:3`: *"A cogmap orients by a DECLARED charter
(`kb_cogmaps.telos_resource_id`). A context has no…"* The centroid is a `vector(768)`, is not a DTO
field, and round-tripping 768 floats through a browser to reconstruct a question nobody asked would
be mechanism pretending to be meaning.

**So a context with no question shows everything.** That is the honest answer for a container of the
reader's own work, and it needs no intention at all. `find-resources-with` carries no ceiling
whatsoever, so the seed set genuinely is every visible resource in the context.

`follow-from` walks *"at least one hop"* (`registry.rs`, `orders_by.means`), so the seeds themselves
are **not** in `w`. The context's own rows come from the list read the table half already makes
(`ResourceListParams` already accepts `context_ref` —
`crates/temper-workflow/src/types/resource.rs:64`), and `ViaEntry.seed_id` names which seed each
reached node descends from, so the client can draw seeds and neighbours as one graph.

**This is deliberately unranked**, and the legibility burden lands entirely on §3's declaration. If
it proves illegible at real context sizes, that is a measurement to act on, not a reason to
pre-emptively add ranking — a decision recorded here so it is not silently reversed.

### 2.4 A named place with a question

`?in=…&q=X` — §2.1 with N = 1. Both anchor kinds behave identically here.

---

## 3. What is drawn, and how the surface declares its kind

**There is exactly one mark vocabulary, everywhere, at every depth.**

- **Nodes** are `ResourceHit.resource` — a `ResourceView`, *"the same projection `list`/`show`/
  `create`/`update`/`annotate` and both search arms answer in"* (`query.ts:800ff`).
- **Edges** are `ViaEntry` — real `kb_edges` rows, as stored.

Nothing else is ever a mark. This is how `navigation-never-silently-changes-kind` is satisfied:
**not by careful labelling but because there is no second kind to change into.** The surface cannot
violate the clause without someone adding a mark type, which is a visible, reviewable act.

### The *why-these* readout

Derived structure appears in exactly one place: a panel, fed from `QueryResponse.trace`, that reads
as **machine reasoning about the answer** and never as a thing in the graph. It carries which regions
matched and their `region_score` (survey's `Disclosure::Region`, arriving as `RegionHit`), the
per-stage `terms_applied`, `input_ids` / `input_unusable` / `produced_ids`, and `narrowed_by`.

`no-derived-thing-poses-as-authored` becomes a testable structural property: **no `RegionHit` ever
reaches the canvas.** Regions reach the readout and nothing else.

The readout is also the answer to `no-internal-vocabulary-is-load-bearing`: it must be readable
without the words *region*, *salience*, *wayfind* or *survey*. It says *"these came from N groupings
of your work"*, not *"3 regions by region_score."*

### The bound declaration — always on screen, plain, never dismissible

**Three axes, and only two have machinery.**

| Axis | Ceiling | Reported by |
|---|---|---|
| **Anchor set** — how many of your places were asked | **24**, client-side `[decided — 2026-08-20]` | **nothing** — the client's own record, and the only axis with a true denominator |
| **Regions per anchor** | `default 3, max 20` (`registry.rs:499`) | `terms_applied[regions]` — **only if the client NAMES the term**; see below |
| **Walk size** | **`Limit: 50`** (`registry.rs:392`) | `Extent` — *complete* or *more exist*, never a count. **`survey` never reports either** |

The walk ceiling is the one that matters most and the register never named it: **the walk contributes
at most 50 nodes**, against a corpus of thousands.

**But 50 is not the screen's total, and it would be an error to design as though it were.** In §2.1
the surveys are returned too, and `survey` declares only `BoundTerm::Regions` — **no `Limit`**. It
returns every visible member of the matched regions, so its row count is bounded in *regions* and
unbounded in *rows*. The screen therefore holds (all members of up to N × `regions_n` regions) + (≤50
walked nodes). In §2.3 there is no survey stage at all and the walk's 50 is the whole cap, with the
context's own rows arriving from the list read, which carries its own pagination.

This is why `legible-at-the-sizes-the-corpus-actually-reaches` is *helped* by the ceilings but not
handed to us by them — the unbounded arm is real — and exactly why
`legibility-is-never-bought-with-silent-omission` carries the weight rather than the ceilings
carrying it.

Two of the three report themselves honestly and the surface's only job is to **stop discarding
them**. `terms_applied` is *"the APPLIED value of every admitted term: the page this stage actually
RAN with, clamped to the act's published ceiling and defaulted where the caller named nothing"*, and
`temper-services` pins the other half — *"reporting the request back would make `terms_applied` an
echo rather than a disclosure"* (`query.ts:1135ff`). `Extent` is `complete | partial |
indeterminate{reason}`, carried *"for every stage, not only the returned ones"* (`query.ts:463`
and per-stage on `StageTrace`).

**The third axis is the surface's own debt.** Anchor-set truncation happens in the client *before the
composition exists*, so no `Extent` can ride on it. The surface must declare it itself, from its own
records.

**There is no denominator for two of the three axes, and the surface must not invent one**
`[corrected — 2026-08-20]`. This section first said the line reads `50 of 314 reachable · 3 of 47
groupings`. It cannot. `StageResult.total` is set to `None` **unconditionally** —
`crates/temper-services/src/backend/query_read.rs:582`, with the reasoning attached:

> *"Carried only by acts that can produce one WITHOUT a second query, and none can: the fragments
> return a page, not a count. **Absent rather than guessed from the page size.**"*

And `Extent` is a saturation test rather than a count — `query_read.rs:709`:
`Some(limit) if produced >= *limit => Extent::Partial`.

| Axis | Numerator | Denominator |
|---|---|---|
| Walk size | rows returned | **none.** `Extent` says *more exist*; nothing says how many |
| Groupings per anchor | `terms_applied[regions]` — what ran | **none** from the composition |
| Places asked | anchors asked — the client enumerated them | **yes**, genuinely known |

The clause never required a denominator. It requires that a partial view not be indistinguishable
from a complete one, and `Extent` draws exactly that line truthfully. **The surface refusing to
manufacture precision is the same discipline as the substrate refusing to** — the same instinct that
forbids presenting `region_score` as a score in §5. Chasing the missing denominators with extra count
reads was considered and refused `[decided — 2026-08-20, Pete]`: it would cost a read per axis per
view and require building the number the substrate deliberately declined to build.

### The anchor-set ceiling `[decided — 2026-08-20, Pete, in Beat A]`

**Ceiling 24. Order: `resource_count` DESC, ties by `ref` ASC.**

Decided against a measurement, as §7 required. The heaviest real reader of the system — 2,330
resources — holds **12 anchors**: 8 contexts and 4 cogmaps.

```
contexts (8)                              cogmaps (4)
2066 @j-cole-taylor/temper                 817 temper-self-cognition        (406 regions)
 355 @j-cole-taylor/tasker                  38 storyteller-system-design     (32 regions)
 191 @j-cole-taylor/storyteller             23 system-default                 (0 regions)
 145 @j-cole-taylor/working-agreements      14 cognitive-maps-for-storyteller (12 regions)
  59 @j-cole-taylor/learning-maths
   8 @j-cole-taylor/knowledge
   1 @j-cole-taylor/general
   0 +temper-system/github-readonly
```

24 is 2× the measurement, so **truncation does not fire for any real reader today** — the ordering
rule is a safety net rather than routine behaviour, which is deliberate: a ceiling that fires
routinely would make a reader's ordinary act (creating a context) silently change what the
unaddressed door asks.

**Why `resource_count`, and why the alternatives are unavailable rather than merely worse.** Only
three fields span both anchor kinds — `resource_count`, `name`, `ref`. A recency ordering is
**inexpressible**: `CogmapSummary` carries no timestamp at all (`id, name, owner_ref, ref,
region_count, resource_count, team_ids, telos_resource_id, charter_statement`) while a context
carries `created`/`updated`, and ordering contexts by recency and cogmaps by something else would be
the kind-dependent behaviour `cross-kind-relationship-is-reachable` exists to forbid. Ordering by
`id` — UUIDv7, so its leading bits are a timestamp — is **wrong for a reason worth recording**, since
it looks like a free recency: `@me/general` is `00000000-…-0003-000000000006` and `system-default` is
`00000000-…-0005-000000000001`, seeded sentinels rather than v7, so they sort to one extreme
regardless of age. `ref` ASC drops `@j-cole-taylor/temper` (2,066 resources) before
`+temper-system/github-readonly` (0), because `+` precedes `@` in ASCII.

So `resource_count` DESC is the least-lossy of what is actually available: it drops the emptiest
anchors first, and an anchor with zero resources — which exists today — is dropped before any anchor
with material. `ref` ASC breaks ties so the choice is deterministic and a URL is reproducible.

### Two findings that change what the line can say `[found in Beat A grounding — 2026-08-20]`

**1. `terms_applied[regions]` is ABSENT unless the client names the term.** `applied_terms` defaults
**only** `Limit`, and only from a published ceiling — `registry.rs:689`: *"`Regions` deliberately does
not: `wayfind_region_scores` has its own funnel default (3) beneath a ceiling of 20, and defaulting
to the ceiling here would widen every unbounded survey sevenfold while claiming to describe the
deployed system."* Its own test pins it (`registry.rs:1678-1682`):

```rust
assert_eq!(applied_terms(&BTreeMap::new(), &survey).get(&BoundTerm::Regions), None);
```

A survey that names nothing therefore **runs at 3 and reports nothing**, and this axis would have no
source at all. **So the builder names `regions` explicitly on every survey stage.** The disclosure is
only real if the plan asks for it — which makes this axis, uniquely, one the *client* has to earn.

**2. `survey` reports `Extent::Indeterminate` unconditionally** — before any row counting, on the way
in (`query_read.rs:737-743`): *"a region funnel produces its candidate set rather than selecting from
one, so there is no remainder to report."* It is also the arm with no `Limit`. So in §2.1 **only the
walk can ever say complete or partial**, and a line that aggregated the arms could never say
*complete* for the flagship entry no matter what the corpus did.

**Presentation.** A persistent, non-dismissible line — chrome, not a warning. **The arms are declared
separately and never aggregated** `[decided — 2026-08-20, Pete]`, so no arm's truthfulness is diluted
by another's: the survey arm carries a count and makes no remainder claim, the walk carries its
`Extent`.

```
Showing 31 from your places · 50 followed on · more exist · 3 groupings asked · 7 of 7 places
Showing 12 from your places · 50 followed on · more exist · 3 groupings asked · 24 of 40 places
Showing 50 followed on · more exist · groupings not applicable · 1 of 1 place
```

The third is a §2.3 context entry: no survey arm at all, so the *from your places* figure is absent
rather than zero and the groupings axis says **not applicable**.

Present whether the view is complete or partial, so *complete* is something the reader is **told**
rather than something they infer from silence. This is the strongest available reading of
`legibility-is-never-bought-with-silent-omission`, and it is deliberately not the cheaper "show a
marker when something was dropped" — under that design the absence of a marker becomes the signal,
and a bug that suppresses it is invisible.

**A context entry runs no survey**, so its groupings axis is *not applicable* rather than zero. It
says so; a missing axis and an exhausted one must never render alike.

### Three measurements from the first real response `[found in Beat B — 2026-08-20]`

Every number in §2 and §3 above came from reading code, from the old surface's reads, or from
synthetic fixtures. These come from POSTing the builder's own output at the deployed `/api/query`.

**1. `survey` ignored its funnel width entirely, and `terms_applied` said otherwise.**
`__temper_ungated_survey` never filtered `wayfind_region_scores` on `in_top_n`, so every survey
joined the whole candidate pool: widths 1, 3 and 20 returned identical 406-region, 731-row answers,
and every anchor disclosed exactly its full region count. The flagship entry drew **2,947 marks**.
Fixed as **Beat 0.5** (task `01a01fe6-4b96-7bb0-ac98-18dc2a8f33be`, migration `20260820000010`),
which brings the same screen to a measured **~160**. §3's *"unbounded in rows"* was therefore
understating it: the arm was unbounded in **regions** too, against the field that claimed otherwise.

**2. The mark vocabulary needs a dedupe, and the layout target in §6 is understated.** `ViaEntry` is
*"one entry per edge it was reached by"* — per **(seed, edge)** pair, so one edge repeats once per
seed that reached it. On the real 50-node walk:

```
via entries (raw): 1973
distinct edges:     102      19.3x inflation
max via on one node: 98
max degree by distinct edges: 25
```

So the canvas must collapse `via` on `(source_id, target_id, edge_kind, label)` before drawing, or it
puts 1,973 edge marks where 102 belong. And §6's `legible-at-the-sizes-the-corpus-actually-reaches`
witness says *"worst-case `via` density — 9 edges on one node, from the prod measurement"*: that 9
came from the predecessor's own read. **This surface's measured worst case is 25**, so the witness
target is 50 nodes / 102 edges / max degree 25.

**3. A stale CLI silently drops a new field, and `--check` is free.** `temper query --format json`
re-serializes through the local CLI's types, and `#[serde(default)]` on `StageTrace::disclosed_regions`
means a CLI older than the field reports its **absence** rather than failing. The first reading of
this arc's central disclosure was therefore wrong in the safest-looking direction; the raw wire
carries it. Read the wire when the question is whether a field arrives. Separately,
`temper query --plan … --check` consults **no server and needs no token**, and it refuses an
ill-formed plan outright — it would have caught Beat A's intentionless surveys at authoring time.

---

## 4. What is replaced, and in what order

Replace `/graph` **in place**. One arc, and the receiver for displaced structure ships **before** the
marks that displace into it are deleted, so no clause is uncovered at any point.

### 4.1 Sequence

| Beat | Contents | Why here |
|---|---|---|
| **0** | Make `survey`'s region disclosure real — [01a01f21-c2ab-78b0-ada5-e8190d9c0814](./01a01f21-c2ab-78b0-ada5-e8190d9c0814) | The only backend work in the arc, and a pre-existing contract defect. Beat A's readout cannot name a grouping until it lands |
| **0.5** | `survey` honors its funnel width — [01a01fe6-4b96-7bb0-ac98-18dc2a8f33be](./01a01fe6-4b96-7bb0-ac98-18dc2a8f33be) `[added — 2026-08-20]` | Found from Beat B, fixed before it. Without it the flagship entry draws 2,947 marks and the bound line's second axis reports a clamp that never applied |
| **A** | The composition builder + the bound declaration, as pure modules with tests. No rendering. | Every witness in §6 that is machine-decidable lands here, against no UI |
| **B** | `/graph/[owner]` rebuilt on Beat A: the four params, the three entries, node/edge canvas on the surviving layout modules, the *why-these* readout | The successor, shipped |
| **C** | **The receiver** — [Atlas analytics readout](./019f0e9a-f0ce-7de2-a848-0d3e4cd3add4), cogmap arm: telos, staleness, regulation against the existing endpoint, declaring itself as analysis | Must precede D |
| **D** | Delete the evicted modules; retire the tier model; point `contextGraphHref` at the new shape | Nothing is displaced into nowhere |

#### The displacement is at B, not D `[ruled — 2026-08-20, Pete]`

The table above puts *"must precede D"* on Beat C, and the paragraph opening §4 says the receiver
ships first *"so no clause is uncovered at any point."* Grounding in Beat B found that premise wrong
about **which beat displaces**, and the ruling that followed is recorded here rather than left to be
re-derived.

`AtlasPage` is rendered by exactly two routes — `routes/(app)/graph/[owner]/+page.svelte` and
`routes/dev/atlas/+page.svelte`. **B rebuilds the first**, so `TierPanorama`, `TerritoryCircle`,
`RegionHoverCard` and the residual tray leave the reader's path the moment B merges, whatever
`contextGraphHref` emits. D then deletes files B already orphaned. The nav flip is not what does it.

**And Beat C as scoped does not receive what B displaces.** `CogmapAnalyticsRow`'s own doc says so:
*"The map-level analytics picture as returned by `cogmap_analytics`: the telos charter resource id,
staleness, and the regulation set. **Per-region scalar metrics are a SEPARATE read
(`cogmap_region_metrics`)**"* (`types/generated/cognitive_maps.ts:48`). What B takes off the path is
per-region metrics — `RegionHoverCard.svelte:17-19` renders `memberCount` · `salience` · `coherence`
— and the superseding decision names exactly that payload as the receiver's: *"Region analytics get
their own place… **Salience, coherence and member counts** carry enough axes to support a topographic
reading."* Telos, staleness and regulation are surfaced **nowhere** in the UI today (measured twice on
C's task), so B cannot displace them.

Two rulings, both Pete's:

- **B merges first; C stays before D.** Between B and C the region field is reachable from the API
  and the terminal but not the UI — the same state as before the Atlas ever drew it, and the
  pre-existing condition C's task was written to complain about. `displaced-structure-remains-reachable`
  is **uncovered for that window, declared rather than silent**. This is the same posture §4.3
  already takes for the `AtlasCanvas.svelte:53` lie, which is why *"no clause is uncovered at any
  point"* was already not literally held.
- **Beat C's scope widens** to carry the per-region metrics read alongside telos/staleness/regulation.
  Without it C never covers the clause, whenever it merges.

Task [019f0e9a](./019f0e9a-f0ce-7de2-a848-0d3e4cd3add4) folds in as Beat C. Its
`[contradicted — 2026-08-16]` amendment already withdrew the lens-alternative criterion and gated its
context arm on D6, which is absent — **the context arm stays out of this arc**, declared rather than
attempted.

### 4.2 Module disposition

**Survives, several strengthened** — `camera.ts`; `layout/forceNeighborhood.ts` (**exactly what the
successor draws**); `labels.ts`, plus the collision handling **G2** asked for, which becomes central
once every screen is nodes; `neighbors.ts`, `trail.ts`, `payloadRows.ts`, `eventSummary.ts`,
`relativeTime.ts`, `TrailRail.svelte` (plus **N1** body/excerpt and **N2** richer hover, both folded
forward from [Graph Atlas C3.1](./019f2fbe-f4ac-7e83-955e-c4dc885856f3) by the pivot);
`marks/Edge.svelte`, `marks/NodeChip.svelte`, `marks/NodeHoverCard.svelte`,
`marks/OrphanNodeMark.svelte`; `CompositionA11yList.svelte`; `palette.ts`, `homeTint.ts`.

**Deleted in Beat D** — `territory.ts`; `layout/forceTerritories.ts`, `layout/packTerritories.ts`,
`layout/cogmapTerritories.ts`, `layout/hull.ts`, `layout/bridges.ts`, `layout/homeLayout.ts`;
`residualTray.ts`; `marks/TerritoryCircle.svelte`, `marks/BridgeRibbon.svelte`,
`marks/RegionHoverCard.svelte`; `TierHome.svelte`, `TierPanorama.svelte`, `HomeA11yList.svelte`,
`ResidualTray.svelte`; and the tier model wholesale — `nav.ts`, `viewData.ts`, `marks.ts`,
`scopeChips.ts`, `legend.ts` / `AtlasLegend.svelte` (the edge-grammar legend survives only if the
edge vocabulary still needs one; decide in Beat B against the built canvas, not here).

`crumbModel.ts` / `AtlasCrumb.svelte` survive as files and are **rewritten**: there are no tiers to
ascend, only the params of §1.

**Out of scope, named so it is not mistaken for oversight**: the nine `/api/graph/*` endpoints lose
their only caller in Beat D. Deleting them is Rust and a separate PR; this arc is deliberately
frontend-only.

### 4.3 The one live bug, and where it goes

[Graph Atlas C3.1](./019f2fbe-f4ac-7e83-955e-c4dc885856f3)'s surviving remainder is
`AtlasCanvas.svelte:53-54` — verified still present:

```
cogmapId && tier === 2
  ? 'Node neighborhoods are not available in cogmap view yet — return to the map to explore its regions.'
```

It fires exactly when a cogmap node genuinely has no neighbours, telling the reader a feature is
missing when the true answer is *"there are none"* — the live instance of
`no-reader-is-left-to-blame-themselves` on a surface people use today.

**Not fixed. Beat D deletes it** `[decided — 2026-08-20, Pete]`. The case for a separate immediate
fix was that the lie stays live for the length of the arc; the case against is that it is a string in
a file this arc removes, and paying a PR to edit a doomed file buys a shorter exposure on a code path
reached only by a cogmap node with zero neighbours.

**The cost is real and is accepted, not waved away:** until Beat D merges, a reader who drills a
neighbourless cogmap node is told a feature does not exist when the answer is *"there are none."* The
clause it violates stays violated for that window.

**Consequence for the task.** [Graph Atlas C3.1](./019f2fbe-f4ac-7e83-955e-c4dc885856f3) was
re-scoped by its `[verification sweep — 2026-08-17]` down to exactly this one bug, with everything
else measured as fixed, dead, or folded forward. With the bug absorbed into Beat D, **the task has no
remaining content of its own** and should be closed against this spec rather than left open as
apparently-live work.

---

## 5. Constraints inherited, not resolved here

Both are open rulings the register already carries. Building on `survey` does not settle either, and
this spec must not read as though it had.

**The blend is an open ruling.** `survey` orders by `region_score` = `0.4·sal_norm + 0.6·query_cos +
0.05·prior`, spanning `[-0.57, 1.05]` — it can be negative and it exceeds 1. Whether the `sal_norm`
term violates the query goal's *the-question-decides-within-an-act* is **OPEN by ruling
`[2026-08-14, Pete]`**. Consequence for this surface: **never present `region_score` to the reader as
a score.** The readout may say these groupings matched and in what order; it may not print the
number or imply a calibrated scale.

**Query-time lens selection is a declared hole, by ruling.** `survey` passes `p_lens = NULL`
*definitionally* — *"the lens is a clustering-time parameter; NULL reads the baked salience"* — and
`migrations/20260816000020_survey_act.sql` records *"The lens selector at query time is a declared
hole: re-lensing regions under a different telos at read time is a future capability with no use case
today."* This surface offers **no lens control**. Task 019f0e9a's original lens-alternative criterion
is already withdrawn on that ruling.

**Boundary against goal `019fb559-7191-75a3-99d4-879090c60e94`** (closed completed 2026-08-05): it
owns whether wayfind's *ranking* is fair and its *self-report* honest. This surface must not re-claim
those clauses. It inherits one remainder — round-robin admits one region per map per round, so reach
is bounded by `regions_n`, and §3's declaration is where that bound becomes visible.

---

## 6. Witnesses

The register requires witnesses authored **inside** the build, each failing against the state its
clause claims to change. Most are unit tests over pure functions, which is a deliberate consequence
of Beat A existing: the composition builder and the bound declaration are pure, so they are testable
without a browser.

| Clause | Witness | Beat |
|---|---|---|
| `surface-declares-its-kind` | **Judged — not machine-decidable.** Perspective and two exemplars are named in the register: a reader who does not know how Temper derives anything, shown the successor, asked *"what am I looking at"* and *"why does clicking this navigate into a graph"*. A reader session, not a test | B |
| `navigation-never-silently-changes-kind` | The mark vocabulary is exactly `{node, edge}`; the test fails the moment a third is added | B |
| `entry-does-not-presume-organization` | The builder produces a valid composition for 0 anchors, 1 anchor, 40 anchors, and for an anchor with zero regions | A |
| `legible-at-the-sizes-the-corpus-actually-reaches` | Layout holds at the walk ceiling (50 nodes) with worst-case `via` density — 9 edges on one node, from the prod measurement | B |
| `cross-kind-relationship-is-reachable` | A union spanning a context anchor and a cogmap anchor returns one answer with both represented | A |
| `displaced-structure-remains-reachable` | The analytics place exists, declares itself as analysis, and is reachable — **and Beat D does not merge before it does** | C |
| `no-derived-thing-poses-as-authored` | No `RegionHit` reaches the canvas; regions reach only the readout | A + B |
| `no-reader-is-left-to-blame-themselves` | A stale readout region reference renders as *"re-derived"*, not as an error | B |
| `legibility-is-never-bought-with-silent-omission` | Three tests, one per axis: each asserts the declaration renders the **applied** value from `terms_applied` / `Extent`, never the requested one | A |
| `no-internal-vocabulary-is-load-bearing` | The readout's rendered strings contain none of *region*, *salience*, *wayfind*, *survey* | A |
| `the-unstructured-reader-is-never-worse-off` | A reader with many resources and **zero** regions still gets a graph — §2.3's path needs no region at all | A |

That last row is worth stating plainly, because it is the actor the predecessor failed. A context
entry runs `find-resources-with → follow-from` and **touches no region**, so a reader whose corpus
has never clustered gets the same surface as one whose corpus has.

### Fixtures and the harness

`/dev/atlas`'s fixture corpus was refreshed against the measured substrate by
[019fbac4](./019fbac4-50c1-75a0-8528-f64030453bfc) (done). The successor needs its own fixtures — a
captured `QueryResponse` per entry, including a partial one and a zero-region one — because the vault
surface is undevelopable locally (1 local resource against 2,330 in prod), which is the standing
finding behind [/dev/vault render harness](./019f6d08-8b33-7f30-a438-8487261d5f23).

---

## 7. Open, and deliberately not closed here

- ~~**Anchor-set bounding policy.**~~ **CLOSED `[decided — 2026-08-20, Pete]`, in Beat A and against
  a measurement — see §3's *The anchor-set ceiling*.** Ceiling **24**, ordered `resource_count` DESC
  then `ref` ASC.
- ~~**Whether §2.3 is legible at real context sizes.**~~ **CLOSED `[decided — 2026-08-20, Pete]`, in
  Beat C and against a measurement — see §8.** The measurement arrived for §2.1 rather than §2.3:
  **80 of 155 nodes at degree zero**, post-Beat-0.5, on the deployed substrate. The response is a
  **declared field**, not a ranking — the degree-zero nodes are drawn in their own band beneath the
  connected core and captioned in the reader's words. Every node is still drawn, the mark vocabulary
  is still two, and no order was invented.
- **Rate-shaped axes remain open**, exactly as the register says. Derived structure settles
  asynchronously with respect to the reader's own writes. This spec reduces the exposure — no region
  id in a URL — and does not close the axis.
- **The legend's fate**, per §4.2.

---

## 8. `[built — 2026-08-20]` Beat C — the receiver, and the legibility ruling it carried

Beat C shipped the analysis door and the unconnected field. Two rulings were taken before any code
was written, both Pete's, and both changed what got built.

### 8.1 Where the receiver lives `[ruled — 2026-08-20, Pete]`

**A new route, `/graph/[owner]/analysis?in=<one anchor>`, linked from the *why-these* panel.**

§4.2's architecture is that derived structure lives in exactly one panel, and that claim is
load-bearing enough that `WhyThese.svelte` says a reader can confirm it by looking. A second section
inside that panel would have kept the claim literally true while putting a 501-row analytic table
into a navigation sidebar. A separate route does not falsify the claim — it scopes it: *on the
graph page*, derived structure is in one panel; the analysis door is not the graph page, and its
first line says so.

**One anchor at a time, and that is a measurement rather than a simplification.** Measured against
the deployed substrate:

```
                     cogmap (406 regions)    context @me/temper (501)
centrality           0 → 2342.2              0 → 276
reference_standing   0 → 96                  0 → 9
internal_tension     0 → 4.7                 0 → 0      ← identically zero, all 501
content_cohesion     0.879 → 1.000           0.872 → 1.000
telos_alignment      0.593 → 1.000           0.679 → 0.984
salience             median 0.95, max 497.65 median 0.55, max 69.54
member_count         61% are singletons      23% are singletons
```

The same quantity spans an order of magnitude more on one place than the other, so one ranked list
across two places is arithmetic on incommensurable quantities — and the order it produced would look
exactly as authoritative as a real one. Places the reader also named are **linked**, never merged.

### 8.2 How the numbers are presented `[ruled — 2026-08-20, Pete]`

**Raw figures at the substrate's own precision, beside the span this place measures. Never a
percentage, bar, meter, ratio or 0–100 scale**, asserted by a test that walks every metric cell.

The pull to normalise is strongest exactly here, and a figure that merely *looked* calibrated would
settle an open ruling silently. Two consequences the table above forces:

- **A constant quantity is said once, not tabulated.** `internal_tension` is identically `0` across
  all 501 groupings of `@me/temper`. It gets no column and one sentence — *"Every grouping here
  measures 0."* An ordering over 501 identical zeroes is an order made of noise, which is what the
  two rejected presentation options would have produced.
- **`null` is a dash and never a zero.** Measured: 4 of 406 groupings have no cohesion, 13 of 501 on
  the context. And a metrics read that *did not answer* is **unknown**, not absent — captioning 501
  groupings *"not computed"* on evidence the surface does not have is the same error as calling a
  grouping re-derived on an incomplete lookup.

Each machine name leads with plain words and carries a one-line gloss, so
`no-internal-vocabulary-is-load-bearing` holds without hiding the substrate's own field names.

### 8.3 Two findings from grounding

- **`regulation` is empty on every readable map.** All four, measured. One third of Beat C's
  original payload has nothing to show anywhere in the live system, so the empty state is the
  routine case rather than an edge case and reads as a fact about the map, not a failed lookup.
- **The context arm stays out, declared.** D6 is still unshipped and a context has no charter and no
  regulation set *even in principle* — so the page says what a context is rather than reporting a
  failed lookup, and no peer field is fabricated.

### 8.4 The unconnected field `[decided — 2026-08-20, Pete]` — §7's ruling, answered

**Every degree-zero node is drawn, in a declared band beneath the connected core, captioned in the
reader's own words:** *"80 of these 155 are not connected to anything else in this answer."*

Three things it deliberately is not:

- **Not a new mark.** `GraphPage.component.test.ts` asserts the canvas's mark classes are exactly
  `['edge', 'node-chip']`, and that test is currently what covers
  `navigation-never-silently-changes-kind`. The field is a *place on the canvas*; the divider and
  caption are chrome, deliberately not wrapped in a classed `<g>`.
- **Not a ranking.** Placement preserves the order the answer returned. §2.3 ruled unranked-
  everything is the design and that its failure mode is a measurement rather than a licence to rank.
- **Not a bound.** Nothing is withheld. If the band genuinely cannot hold them at the tightest
  spacing, the caption states the remainder — `legibility-is-never-bought-with-silent-omission` is
  exactly the clause a quiet truncation would break.

### 8.5 What the committed fixture cannot witness

`graph-successor-flagship.json`'s trim rule keeps every survey hit a `via` entry references, which
keeps the **connected** hits by construction and only four arbitrary unconnected ones per stage. So
the fixture reads **10 of 52** nodes at degree zero where the live response reads **80 of 155**. The
ruling was made on the wire; the fixture can witness that the field exists and is captioned truly,
and nothing about the population. This is recorded in the fixture's own
`_trimmed.degree_zero_NOT_witnessable`, and is the same species as the 101-edge collapse that block
already names: **a trim that preserves one property destroys another.**

### 8.6 What Beat C did not do

- **The loader has never met a server.** Every read path was verified `200` against the deployed API
  with a real token, and the components are tested against those exact untrimmed payloads — but
  `+page.server.ts` itself is exercised by nothing, because session auth is a browser cookie flow.
  The same gap that hid three defects in Beat A, narrowed but not closed.
- **§2.3 still has no wire capture**, unchanged from Beat B.
