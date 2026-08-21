# The band is where the hubs go — and no selection can reach them

Task [The unconnected band is where the most-connected material goes — degree 87 draws zero links](./01a024d3-2a16-78b1-9e7e-a0e98bd87e0e).
Continues [Grounding and navigation are different acts](./2026-08-20-grounding-and-navigation-split-design.md), chunks A/B/C
shipped. **Sequenced before chunk D, not merged into it.**

## 1. The ruling, and it was measured rather than argued

The task filed three candidate fixes and forbade choosing among them by argument, because
asserting-instead-of-measuring is how §10.1 shipped false. The measurement was run against
production first, with its criteria declared before the queries.

> **It is a telling failure. Option 3.** Not because it is cheapest — because options 1 and 2 were
> measured and **cannot reach these nodes at any drawable size**. That is arithmetic, not a trade.

## 2. The measurement

Production, profile `j-cole-taylor` (`019d4add-f49d-7c43-a87d-dda470e5dd9c`), read-only.
Baseline reproduces the shipped read exactly:

```
in_scope 3583 · eligible 2499 · drawn 130 · induced_edges 275 · cut_degree 11 · max_corpus_degree 87
```

`in_scope`/`eligible` have moved from the 3574/2498 of §10.1.1 — the corpus grew between the two
measurements. Everything the ranking decides is unchanged.

### 2.1 C1 — the band is *definitionally* the hub band

All 26 stranded nodes carry corpus degree **≥ 11**, and they must: **11 is the cut**. There is no
weakly-connected material in the band and there cannot be. Min 11, max 87.

| | nodes | min corpus degree | avg | max |
|---|---|---|---|---|
| band | 26 | **11** | 24.7 | 87 |

**`0 links` is false for every row of it, not only for `Maintenance`.** This is the finding that
reframes the defect: the band is not "the weak stuff plus one anomaly", it is a homogeneous set of
well-connected resources whose neighbours all sit below the cut.

### 2.2 C2 — the drawing is a picture, not rubble

Connected components over the induced subgraph at K=130:

| component size | count | nodes |
|---|---|---|
| 92 | 1 | 92 |
| 4 | 1 | 4 |
| 2 | 4 | 8 |
| 1 | 26 | 26 |

**92 of the 104 connected nodes are one component.** This mattered and could have gone the other
way: had the "connected" 104 been thirty shards, no caption would have saved the screen and the
verdict would have been *selection failure*. A sentence can sit beside a structure; it cannot sit
beside rubble.

### 2.3 C3/C5 — no selection reaches these nodes at a drawable size

For each stranded node, the K at which its **best** neighbour is guaranteed to clear the corpus
cut (ties inside a degree are ordered by id, so this is the guaranteed-inclusion K):

| best neighbour's corpus degree | stranded nodes | K required | example |
|---|---|---|---|
| 10 | 1 | 189 | `G3 machine-principal arc complete…` |
| 8 | 1 | 281 | `Temper UI (SvelteKit)` |
| 7 | 1 | 353 | `Operational memory travels with the work…` |
| 4 | **7** | **739** | **`Maintenance`**, `Tasker Framework`, `Temper`, `MCP`, … |
| 3 | 6 | 995 | `The system is the authority…`, `FFI Architecture`, … |
| 2 | 6 | 1434 | `The domain's load-bearing invariants…`, `path-to-alpha`, … |
| 1 | 4 | **2499** | `The working contract…`, `A green signal is not the property you wanted…` |

- **Connecting `Maintenance` costs K=739** — past `ENTRY_MAX_K` (600, `graph_service.rs:568`) and
  roughly **3× the 250-mark ceiling** §2 of the predecessor spec measured.
- **Connecting all 26 costs K=2499**, which is `eligible` — the entire connected corpus.
- Only **3 of 26** are reachable below K=400.

**This kills option 2 outright.** "Rank on a blend of corpus and induced degree, iterating until the
set is self-supporting" has exactly two fixed points on this corpus: evict the hubs, or draw
everything. A self-supporting set that omits the reader's most-connected resource is strictly worse
than one that strands it — the complaint becomes *"my busiest goal is not on the screen at all."*

### 2.4 C4 — option 1 buys `band = 0` by making the canvas worse

Pulling each stranded node's single best-connected neighbour from below the cut (m=1):

| | drawn | induced edges | band | largest component | components > 1 | bare pairs |
|---|---|---|---|---|---|---|
| baseline | 130 | 275 | 26 | 92 | 6 | 4 |
| +best neighbour | 154 | 311 | **0** | **92** | 21 | **17** |

**The band goes to zero and the picture does not grow by a single node.** The 26 stranded marks
become ~17 free-floating two-mark dumbbells scattered through the force layout. Added material
averages corpus degree **3.3** (max 10, min 1; 10 of 24 at ≤ 2) — at m=3 it is 199 drawn, avg 2.9,
43 of 69 at ≤ 2.

This is the trade stated plainly: **a declared place on the canvas is exchanged for undeclared
scatter**, and the screen stops meaning *"your most-connected material"* and starts meaning *"your
most-connected material plus arbitrary chaff hanging off it."* `band = 0` is a metric improving
while the thing it proxies for gets worse.

### 2.5 What the measurement does not cover

- **One profile's corpus.** Every number here is `j-cole-taylor`. The *shape* of the argument —
  hub-and-leaves stars strand under degree ranking — is structural, but the K values are not.
- **It does not establish that the chosen caption works.** A caption is judged by a reader.
  Measurement ruled out the two selection-side options; it did not witness that option 3 lands.

## 3. What the failure actually is

Two statements on screen, both produced by machinery that is behaving exactly as designed:

- `presentation.ts:198` — *"26 of these 130 are not connected to anything else in this answer."*
- `GraphA11yList.svelte:34-35` — *"`Maintenance` — goal in @j-cole-taylor/temper, 0 links"*

The first is **true and misleading**; the second is **false as read**. Neither is a bug in the
selection, and `legibility-is-never-bought-with-silent-omission` is satisfied throughout — the
bound line declares its remainder correctly. The screen misleads anyway, which is the whole point:
a clause can be covered while the reader is still told something they know to be untrue.

## 4. `[ruled — 2026-08-21, Pete]` §5.3 is narrowed, not overturned

§5.3 ruled: *"Only one degree ever reaches the screen, and it is the derived one."* Option 3 cannot
be built under it. The ruling is **AMENDED**:

> The corpus figure may reach the screen **only inside a sentence that states its relationship to
> the drawn one**. A bare second number beside a mark stays forbidden.

The original hazard is preserved exactly. §5.3's reason was *"a node can carry `degree: 12` and show
zero edges, and a reader has no way to reconcile that but to doubt themselves"* — two quantities
under one name, with the relationship left for the reader to guess. `0 drawn here · 87 in your
corpus` is one sentence that supplies the relationship, so there is nothing left to reconcile. What
stays banned is the shape §5.3 was actually pointed at: a lone `87` on a hover card beside three
strokes.

## 5. The design

### 5.1 The corpus degree is carried, under a name that is not `degree` — **EXTEND**

`AtlasNode.degree` is **already on the wire** and already the corpus figure
(`graph_service.rs:600-603`: *"the node payload carries `AtlasNode.degree` = corpus degree… §5.3's
ruling… is a claim about the screen, not the wire"*). `buildEntryGraph` currently discards it:

```ts
// model.ts:210-212
// Recomputed below over the drawn edges. Starting from the wire's corpus degree and
// incrementing would blend two different quantities into one number.
degree: 0,
```

That comment stays true and the recompute stays. What changes is that the wire's figure is kept
**beside** it under a second name rather than dropped. Authorized by §4 above.

**The field is nullable, and that is load-bearing rather than defensive.** `ResourceView` carries no
degree — verified: `grep -n degree resource_view.ts` returns nothing — so `buildGraph`, the
composition path, genuinely cannot supply one. `null` means *this read did not report a corpus
degree*, never *zero*.

### 5.2 The two paths do not say the same sentence — **CONFORM**

This is the step an implementer is most likely to get wrong, so it is stated before the code it
governs.

On the **entry read**, a band member is guaranteed corpus degree ≥ the cut (§2.1) — "unconnected"
can only ever mean *connected to material below the cut*. On a **composition answer**, degree zero
in the answer may genuinely mean zero edges anywhere; that path has no corpus figure to say
otherwise. **`describeUnconnected` must therefore be told which fact it holds, not handed a new
sentence.** Giving both paths the entry read's wording would put a claim on the composition screen
that nothing measured — the same defect class, one surface over.

`describeUnconnected` is pure and covered by `presentation.test.ts`; this is an addition to its
vocabulary, conforming to the shape §7.1 already established for `renderBoundLine`.

### 5.3 The accessibility list stops asserting `0 links` — **AMEND**

`GraphA11yList.svelte:34-35` renders the derived degree with the bare word `links`. On the entry
read that is the false half of the defect and the *first row a screen-reader user meets*. It must
carry the same relationship the caption does. Authorized by §4.

The heading is a separate defect and is **not** touched here — see §6.

### 5.4 A reader must be able to tell *which* marks, without hovering — **EXTEND**

Acceptance criterion 1. The caption names a count; the reader must be able to bind it to marks. The
band is already a declared place on the canvas (`GraphCanvas.svelte:58,89-90` — a rule, a sentence,
and a packed field beneath the core), so the binding largely exists. What is missing is that the
sentence beside it currently describes the wrong thing.

**No new mark may be introduced.** `GraphPage.component.test.ts` fails if a third mark class
appears, and that test is what currently covers `navigation-never-silently-changes-kind`.

### 5.5 The ring is not drawn where it distinguishes nothing — **AMEND**

`buildEntryGraph` sets `arm: 'seed'` on all 130 nodes (`model.ts:216`); `GraphCanvas.svelte:228`
passes `seed={node.arm !== 'walk'}`; `NodeChip.svelte:64` rings on it. **Every mark on the entry
canvas is ringed, so the channel spends ink on a constant** — and it is the channel a reader has
already misread once.

`[ruled — 2026-08-21, Pete]` **The ring encodes a contrast between arms; where every node shares
one arm there is no contrast, so no ring is drawn.** This is a property of the *view*, computed
from the model, not a special case for the entry read — a composition answer that happens to return
one arm also draws none, correctly.

Deliberately **not** repurposed to mark the band. The arm vocabulary is chunk D's subject, and
re-encoding it here is the merge the spec already ruled against for `REACHED`.

## 6. What this does not do

- **It does not fix `REACHED — in the places you asked about`.** `presentation.ts:66` and
  `GraphA11yList.svelte:29` claim a question the reader never asked, on all 130 cards. It is
  separately filed (`01a0215d-65d5-7373-92c4-c1559dd911d4`) and is **arm vocabulary** — the same
  reason §5.5 declines to repurpose the ring. It goes with D.
- **It does not change the selection.** K stays 130, the floor stays 1, `graph_visible_degree_ranking`
  is untouched. §2 is the evidence that it should be.
- **It does not give the reader a way to reach the 87.** Telling someone their goal connects to 87
  undrawn things is a door only once traversal exists. **That is chunk D**, and it is what makes
  this caption an invitation rather than a dead end. Named here so the dependency is not discovered
  later.
- **It does not close `surface-declares-its-kind`**, which still has no witness.
- **It does not exercise rung 2**, which has never fired against real data — 2,499 eligible means
  it cannot on this corpus.

## 7. Acceptance, against the task's four

1. *A reader meeting a stranded hub can tell why without hovering* — §5.2, §5.4.
2. *The ring distinguishes something here or is not drawn* — §5.5, not drawn.
3. *The remainder stays declared* — the bound line is untouched; `describeUnconnected`'s `undrawn`
   arm is untouched. A test must pin that `legibility-is-never-bought-with-silent-omission` did not
   regress, rather than the change merely not appearing to touch it.
4. *The choice is made against a measurement* — §2, run before the design and with its criteria
   declared first.

## 8. `[built — 2026-08-21]` What shipped, in the words that reach the reader

Branch `jct/entry-band-telling`. TypeScript only — no SQL, no Rust, no migration.
`graph_visible_degree_ranking` and `ENTRY_DEFAULT_K` are untouched, which §2 is the evidence for.

**The caption**, when the read reports corpus figures for the whole band:

> 26 of these 130 are not connected to anything else drawn here — but each connects to 11 to 87
> things elsewhere in your corpus.

**The accessibility row**, for a mark with no stroke and connections in the corpus:

> Maintenance — goal in @j-cole-taylor/temper, `0 drawn here · 87 in your corpus`

**The hover card** gains one row beside the `0 edges` chip: `connects to · 87 things not drawn here`.

**Where the evidence is absent, nothing changes.** `describeUnconnected` takes the band's corpus
figures as a required argument and falls back to the answer-scoped sentence unless **every** member
reports a positive one. `buildGraph` passes `null` throughout — `ResourceView` carries no degree —
so the composition screen's caption is byte-identical to what it was, and the pre-existing component
test that pins that string is what catches a regression. A reported **zero** takes the plain
sentence too: that resource really is connected to nothing.

`NodeChip`'s `seed` prop became **`ringed`**. The old name asserted a fact about the node when the
value is a decision about the view — the canvas passes `armsDistinguish(model.nodes) && arm !==
'walk'` — and that mismatch is what let a ring fire on all 130 marks unnoticed. The ring circle
carries `class="arm-ring"` so its **absence** is assertable; it is a bare `<circle>` inside the
existing `<g class="node-chip">`, so the two-mark vocabulary is unchanged.

`GraphNode` gained `corpusDegree: number | null`. It joins `degree` on
`model.captured.test.ts`'s field-shape guard on the same footing: both count the reader's own edges
and differ only in the set counted over. `salience` remains excluded — it is inferred *about* a
resource, which is the line that guard draws.

### 8.1 What the tests witness, and how that was checked

611 green in `temper-ui`; `svelte-check` clean. Four **bite probes** were run, each reverting one
change and confirming the failure lands where it should:

| reverted | fails |
|---|---|
| the ring gate (`rings &&`) | the render-level *"draws NO ring"* test, alone |
| the caption's use of its evidence | 3 unit + 1 render test |
| the accessibility row | 1 unit + 1 render test |
| `buildGraph` fabricating a corpus degree | 2 unit tests **and the pre-existing composition-caption test** |

The last is the one worth naming: the guard against the two reads sharing a sentence was **already
in the suite** and needed no new test — it pins the composition caption byte for byte.

A separate assertion keeps the ring test from passing vacuously: it fails if the flagship fixture
ever collapses to a single arm, which is the condition under which *"no rings on the entry read"*
would stop being evidence of anything.

### 8.2 What is still not witnessed

- **No reader has seen this.** The measurement ruled out two options; it did not establish that the
  caption lands. `surface-declares-its-kind` remains without a witness, unchanged.
- **The caption is not yet a door.** It names 87 undrawn connections and offers no way to reach
  them. That is chunk D, and until D lands this is a truthful dead end rather than an invitation.
- **`REACHED` still claims a question nobody asked**, on all 130 cards — arm vocabulary, sequenced
  with D.
- **One profile's corpus**, as everything in §2 is.
- **Nothing has been seen rendered against real data.** The local database holds **1** active
  resource, so a dev-server look would draw an empty canvas and witness nothing; the render-level
  component tests are the strongest evidence available short of deploying. That is the same gap
  chunk A shipped through — *"the modules are green, so the question left is whether the thing they
  compose into renders at all"* — and it closes on the deploy, not here.
- **The caption is a single SVG `<text>`, and SVG text does not wrap.** The new sentence is roughly
  twice the old one's length (~125 characters against ~65). At `font-size: 11` in a 992px-wide band
  that leaves headroom, and the worst realistic case — large counts plus the `undrawn` clause —
  still fits by arithmetic. It has not been measured on a rendered page, and a narrower viewport is
  where it would first run off.
