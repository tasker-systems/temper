# The handoff, and the arm vocabulary stops lying

`[2026-08-21]` Design spec for **chunk D** of
[Grounding and navigation are different acts](./2026-08-20-grounding-and-navigation-split-design.md).
Supersedes nothing. Sits under
[The graph surface shows the reader's own material](./019fbaac-96e2-7620-ace2-667a0f8ff000).

The parent spec already rules D's three visible pieces — **§7.1** the bound line, **§7.2** *Why
these* as provenance, **§10.2** the URL grammar, **§10.3** ground-once-then-hand-off. None of them
is re-argued here.

**What the parent does not cover, and this does: the arm vocabulary.** §11 deferred `REACHED`'s
false claim to D — *"its arm vocabulary changes here"* — and the hub-stranding spec §5.5 deferred
the ring to D for the same reason. Neither said what the vocabulary should *become*. That is this
document, and it is the part of D that has already failed twice.

---

## 1. The finding — the arm is not one thing, and has not been since chunk A

### 1.1 What is on disk

```ts
// packages/temper-ui/src/lib/graph/model.ts:29
export type NodeArm = 'seed' | 'survey' | 'walk';
```

```ts
// packages/temper-ui/src/lib/graph/presentation.ts:32
export function describeArm(arm: NodeArm): string {
  switch (arm) {
    case 'seed':   return 'In the places you asked about';
    case 'survey': return 'From your places';
    default:       return 'Followed on from your work';
  }
}
```

`NodeArm` names **which stage of a composition produced this row** — its own doc comment says so:
*"The three are exactly the three arms the bound line declares."* `describeArm` translates that into
**a claim about what the reader did**. Those two coincided exactly as long as the composition was
the only read on the surface. It has not been since chunk A.

### 1.2 Three channels consume it, not one

| channel | site | what it does today |
|---|---|---|
| the **ring** | `GraphCanvas.svelte:106`, `:243` — `armsDistinguish(model.nodes)` && `node.arm !== 'walk'` | withdrawn on the entry read (one arm) `[#741]` |
| the force **core** | `GraphCanvas.svelte:72` — `coreOf: (n) => armById.get(n.id) !== 'walk'` | every entry node is core |
| the **words** | `describeArm` → `GraphA11yList.svelte:29` (heading), `presentation.ts:77` (hover `reached` row), `NodeRail.svelte:83` (`HOW` row) | *"In the places you asked about"* |

The ring is the channel the task named. **It is one of three, and the words are the ones a reader
has actually complained about.**

### 1.3 Three live instances of one lie, two of them known

**(a) The unaddressed entry.** `buildEntryGraph` sets `arm: 'seed'` on every node
(`model.ts:242`), so both the hover card and the accessibility heading assert *"in the places you
asked about"* on a screen where the box was empty. Filed as
[The hover card claims a question the reader never asked](./01a0215d-65d5-7373-92c4-c1559dd911d4);
`[observed on production — 2026-08-21]` on all 130 cards and on the single a11y heading.

**(b) The ring on all 130 marks.** Withdrawn by #741 via `armsDistinguish`, and correctly so.
`[observed on production — 2026-08-21]` **no mark carries a ring.**

**(c) `[found — 2026-08-21, this session]` "Walk from here →", live today, filed nowhere.**
`withGraphSeed` deletes `q` (`vault-url.ts:273`), and the load's entry test requires an empty seed
list:

```ts
// routes/(app)/graph/[owner]/+page.server.ts:116
const isEntry = !question.text && address.seeds.length === 0;
```

So a hop runs the **composition** with the hopped-from node as `arm: 'seed'`
(`+page.server.ts:94`, `model.ts` `buildGraph`), and its hover card reads
`reached: in the places you asked about` about a node the reader hopped from. Same lie, third door,
and the door D replaces.

### 1.4 Why fixing the strings does not close this

Each instance was produced by a **new view meeting an old label**. A fourth view produces a fourth
instance. The acceptance clause D carries is class-level:

> **Whatever the arm vocabulary becomes, every channel that encodes it distinguishes something on
> every view that draws it** — the ring's two failures are the exemplars.

A global label function that asserts a reader act, applied across every read, cannot satisfy that by
construction: it is a claim made in one place about screens built somewhere else.

---

## 2. `[ruled — 2026-08-21, Pete]` The arm becomes a per-view fact, and the read supplies its words

> **A read declares the arms it produced and what to call them. No read may name another read's
> arms, and nothing outside a model may translate one.**

The ruled shape — recorded because it is the decision, not because it is the implementation:

```
buildEntryGraph  → arms: [{ key:'ranked', label:'What your work is built around' }]
buildTraversal   → arms: [{ key:'from',    label:'Where you hopped from',  reached:false },
                          { key:'reached', label:'Reached from there',     reached:true  }]
buildGraph       → arms: [ seed / survey / walk, today's three words, unchanged ]
```

`describeArm`'s global switch is deleted. Its three call sites read the label off the model.

**Three consequences worth naming rather than discovering:**

- **The entry read's heading stops being a lie without anyone remembering to fix it.** It is not
  that `'seed'` got a better word; it is that `buildEntryGraph` can no longer reach a word written
  for a composition.
- **`armsDistinguish` is unchanged and stays derived from the nodes actually drawn**
  (`presentation.ts:292`). A legend entry with no nodes must not light a channel. #741's rule —
  *"the ring encodes a contrast between arms; where every node shares one there is no contrast"* —
  is exactly the rule that generalises, and it generalises **only** if the count comes from the
  marks rather than from the declaration.
- **`reached` becomes a declared property of an arm, not the string `'walk'`.** `coreOf` and the
  ring both currently hard-code `!== 'walk'` (`GraphCanvas.svelte:72`, `:243`) — a global check that
  no per-view vocabulary can satisfy.

### 2.1 What each channel encodes after D — **AMEND** (§7 of the parent authorizes the vocabulary change; §5.5 of the hub-stranding spec defers the ring here)

`[ruled — 2026-08-21, Pete]` **The ring encodes the view's standing point.** Ringed = the mark(s)
this view was built from; bare = what following edges reached from them.

| view | arms | ring | core |
|---|---|---|---|
| entry | one | **none** — unchanged from #741 | all marks |
| traversal | `from` · `reached` | the hopped-from mark | the hopped-from mark |
| composition | seed · survey · walk | unchanged | unchanged |

This is the same computation the code already performs, re-grounded. **What changes is that its
legend stops disagreeing with it.**

### 2.2 The remainder this does not close

**The ring has no on-screen legend anywhere.** `grep -rn "legend"` over the graph components returns
only `AnalysisPage.svelte:137`. A sighted reader is shown a distinction and never told how to read
it; the fact is reachable only through the accessibility list's headings. **Not fixed here** — it is
new chrome the parent spec did not ask for, and D is already the beat that owns three things filed
elsewhere. Named so it reads as a known remainder rather than an oversight.

---

## 3. The address — **CONFORM** to §10.2, which is ruled

```
/graph/@me?q=<grounding question>&from=<node-ids>&depth=<n>
```

Four facts about what is on disk:

- **`from` already parses** — `params.getAll('from')` (`vault-url.ts:203`) and is emitted by
  `graphHref` (`:222`). No change.
- **`depth` does not exist anywhere in the URL layer.** Not in `GraphAddress` (`vault-url.ts:145`),
  not in `parseGraphAddress`, not in `graphHref`. **EXTEND**, authorized by §10.2.
- **`withGraphSeed` deletes `q`** (`vault-url.ts:273`). §10.2 rules the opposite: *"`q` survives the
  handoff as provenance, and that is what makes §7.2 work."* **AMEND**, cited.
- **A hop already pushes** — `goto(withGraphSeed(...))` with no `replaceState`
  (`NodeRail.svelte:56`), against `close`'s deliberate `replaceState`. §10.2's *"each hop pushes so
  Back walks the reader's path"* is **already satisfied** and needs no change. Recorded so nobody
  re-implements it.

`[ruled — 2026-08-21, Pete]` **`depth` is grammar-only in D.** Parsed, emitted, clamped `1..=3` to
match the service (`graph_service.rs`: `depth.clamp(1, 3)`), and every hop writes `depth=1`. No
control ships. The spec ruled a grammar, not a widget.

**No edge id ever appears in a URL** (§10.2). Nothing here introduces one; `withGraphSelection`
already carries a bare resource uuid and says why (`vault-url.ts:241`).

---

## 4. The handoff itself — **CONFORM** to §10.3, which is ruled

`isEntry` (`+page.server.ts:116`) splits two paths today. D makes it three:

| address | read |
|---|---|
| no `q`, no `from` | `GET /api/graph/entry` — unchanged |
| `from` present | **`GET /api/graph/traverse`** — the composition no longer runs |
| `q`, no `from` | the composition — unchanged |

**Chunk B landed with no caller.** `GET /api/graph/traverse` exists
(`crates/temper-api/src/handlers/graph.rs:207` → `graph_service::traversal_slice`) and
`grep -rn traverse packages/temper-ui/src` finds nothing outside generated-type comments. D is its
first caller. This is the same shape chunk A shipped in — *"69 green tests and zero callers, and
three defects fell out the moment its output met a real server"* — and it should be said in the PR
rather than discovered.

**The walk is not confined to the grounding's result set, and that is the ruling rather than a
gap.** `traversal_slice` calls `graph_induced_edges` over the reader's whole visible corpus. §10.3:
*"you traverse the graph as normal **without a question locking you in**."* The consequence is
load-bearing for §5 and §6 below: **`q` is where the reader started, never a filter still in
force**, and no sentence on a traversed screen may imply otherwise.

**The band works on a traversal without a second design.** `AtlasSubgraph` carries `AtlasNode`, so
`corpusDegree` is populated exactly as on the entry read, and `describeNodeLinks`
(`presentation.ts`) gives an unconnected hop-neighbour `0 drawn here · N in your corpus` rather than
`0 links`. The hub-stranding repair generalises for free. **Check it on production rather than
assuming it** — it has never been rendered from this read.

---

## 5. The bound line's traversed shape — **EXTEND**, authorized by §7.1

> *"It must not keep displaying the grounding query's counts — on hop three those describe a screen
> the reader is no longer looking at. It must not disappear either: it is deliberately chrome, not a
> warning."* — §7.1

The incumbent is the pattern to follow, not to invent around. `renderBoundLine` (`bound.ts:206`)
already carries two shapes — the composition's axes and the entry read's `orientation`
(`bound.ts:209`, added by chunk C via `declareEntryBounds`, `bound.ts:149`). **A third sibling, a
third branch. That is the whole of §7.1's *"an addition to its vocabulary, not a rewrite."***

**What a traversal actually knows, and the honest declaration that follows:**

- Marks drawn, and how many it hopped from.
- The depth it walked.
- **It withheld nothing at that depth** — `traversal_slice` returns everything reached. So there is
  no `drawn of eligible` ratio to state and **one must not be manufactured**; `declareEntryBounds`'
  own rule applies verbatim — *"absent rather than zero… reporting them as zeros would describe a
  composition that returned nothing, which is a different claim."*
- **It cannot know whether a deeper hop would find more**, and must not say. `Extent` has a state
  for exactly this and `extentPhrase` already renders it (`bound.ts`).

**`places` and `groupings` are absent on a traversed view, not zero.** No composition ran, so no
axis has a source — the same reasoning `declareEntryBounds` records.

### 5.1 `[found — 2026-08-21]` The seed clamp is not reported, and its own comment says it is

```rust
// crates/temper-services/src/services/graph_service.rs
/// Clamped loudly: the drop is reported, never silent.
const TRAVERSAL_MAX_SEEDS: usize = 250;
```

The clamp emits `tracing::warn!` and nothing else. **Nothing reaches the client**, so a clamped
traversal draws a smaller graph and the bound line declares it complete —
`legibility-is-never-bought-with-silent-omission`, in the one axis D is adding.

**It cannot fire from D's own surface**, because a hop names one node (`withGraphSeed` replaces
`from`). It is reachable only from a hand-written URL. **Filed separately, not fixed here** — it is
Rust, it is a different PR, and D must not silently absorb it. What D owes is that its own emitted
addresses cannot trigger it, which the one-node hop satisfies by construction.

---

## 6. *Why these* becomes provenance — **CONFORM** to §7.2, whose *recommended* option is ruled

> *"Declare itself as the grounding that the current view descends from — recommended. It stops
> claiming to explain the current screen and becomes provenance for it."* — §7.2

`[ruled — 2026-08-21, Pete]` **Provenance only.** The question, the places, and the route back
survive. **The stage accounting and the grouping list do not** — they were measured for a screen the
reader has left, and no composition ran for this one.

The heading changes with them. `<h2>Why these</h2>` (`WhyThese.svelte`) is itself a claim about the
marks on screen, and on a traversed view it is false in the same way `REACHED` is.

Three things this must preserve, each with the reason it exists:

- **The route back.** §7.2's *"disappear"* option was rejected precisely because it *"loses the
  reader's route back to how they got here."* The link is the address without `from`/`depth`, which
  the grammar makes trivially constructible.
- **`displaced-structure-remains-reachable`.** The per-place *"How these were measured"* links
  (`WhyThese.svelte`, `data-testid="measured-links"`) are what pays that clause. They describe the
  **grounding**, not the traversal, so they survive under a sentence that says so.
- **The question is not still narrowing.** Per §4, the walk left the question's space. The panel
  must say the reader *started* there — not that this is what the question returned.

**Today the panel renders only when `data.readout` is non-null** (`GraphPage.svelte:129`), which is
composition-only. So *"disappear"* is what ships if this is skipped, and §7.2 named that the second
best of three.

---

## 7. Clause impact

| Clause | Impact |
|---|---|
| `legibility-is-never-bought-with-silent-omission` | **The one at risk, and §5 is what protects it.** Covered on a traversed view only once that view carries its own bound declaration — §7.1's own warning, and the reason D cannot ship §3–4 without §5 |
| `no-reader-is-left-to-blame-themselves` | **One standing violation removed** (§1.3a, and §1.3c with it). Removing violations is not coverage of a standing boundary — the amendment's own words, third instance running |
| `no-derived-thing-poses-as-authored` | **Preserved and cheapened.** A traversal returns no groupings, so the panel that could pose has nothing to pose with. `salience` is dropped by every builder and stays dropped |
| `navigation-never-silently-changes-kind` | **Preserved structurally.** D adds no mark; the vocabulary stays `{node, edge}` and `GraphPage.component.test.ts` fails if a third class appears |
| `surface-declares-its-kind` | **Touched, not closed.** §7.2's provenance form is the second half of what the clause was pointed at. It still has no witness — **fourth amendment running**, and D must not be recorded as closing it |
| `entry-does-not-presume-organization` | Untouched. D changes no selection |

---

## 8. Build order

Two PRs, split on **coherence**.

| | | Depends on |
|---|---|---|
| **D1 — the vocabulary** | §2, §2.1. `NodeArm` → read-supplied legend; `describeArm`'s switch deleted; `reached` becomes a declared arm property; `coreOf` and the ring stop hard-coding `'walk'`. Entry read's heading and hover row become true. **No traversal.** Closes [the `REACHED` defect](./01a0215d-65d5-7373-92c4-c1559dd911d4) | — |
| **D2 — the handoff** | §3, §4, §5, §6. `depth` in the grammar; `q` survives a hop; `from` routes to `/api/graph/traverse`; the traversal model; the bound line's third shape; *Why these* as provenance | D1 |

**D1 stands alone**: it removes a live false claim and leaves the ring exactly where #741 left it —
withdrawn, because the entry read still has one arm.

**§5 and §6 do not split out of D2.** §7.1 is explicit that it *"is the one that regresses covered
clauses if it is skipped"*: a traversal shipped without its own bound declaration puts
`legibility-is-never-bought-with-silent-omission` into regression on `main` for the length of a PR
window. The read and what it owes the reader land together.

## 9. Acceptance

From the task, unchanged:

- A reader who traverses away can still tell **how many of how many**, and the count describes the
  screen they are on.
- *Why these* never claims to explain a screen it does not explain.
- Back walks the reader's path rather than leaving the site.
- **Every channel that encodes the arm distinguishes something on every view that draws it.**
- `surface-declares-its-kind` still needs a reader who did not build it. **D does not close it.**

Added here:

- **No rendered string asserts a reader act that did not happen on the view rendering it** — the
  class, not the three instances. The check that means something: `describeArm`'s global switch is
  gone, so no read can reach another read's words.

## 10. How this gets verified — plan for it

**A PR preview cannot reach any authenticated page.** `/graph/@me` returns `?error=auth_state_lost`
because Auth0 callbacks are registered per domain, and a local database typically holds a handful of
resources. **No PR preview in this arc has ever witnessed a graph screen.** Carried as
[A PR preview cannot witness any authenticated surface](./01a0255e-aac2-7382-b470-7e60205fdf69).

So the strongest **pre-merge** evidence is a render-level component test —
`render(GraphPage, { data })` against a realistic model, the idiom
`GraphPage.component.test.ts` already establishes — and the real witness arrives **after the
deploy**. Say so in the PR rather than letting green read as confirmation.

**D is mostly URL grammar and pure functions, which tests cover well. The arm vocabulary is exactly
the part that is not testable that way, and it is the part that has failed twice.** What a test can
witness: that no view draws a channel with nothing to distinguish, and that a label comes from the
model that produced the node. What only a reader on production can witness: whether *"Where you
hopped from"* is what a person standing on that screen understands the ring to mean.

## 11. What this does not do

- **It does not fix the seed-clamp silence** (§5.1). Rust, separate PR, unreachable from D's own
  addresses.
- **It does not give the ring a legend** (§2.2).
- **It does not close `surface-declares-its-kind`.** Nothing built closes a judged clause.
- **It does not touch selection, K, the floor, or the ranking.** §5.3's narrowing and the K=130
  arithmetic are settled; see
  [The band is where the hubs go](./2026-08-21-hub-stranding-is-a-telling-failure-design.md) §2.
- **It does not do chunk E** — deleting `contexts/panorama`, `contexts/composition` and
  `graph_traverse`. That depends on C, not D.
