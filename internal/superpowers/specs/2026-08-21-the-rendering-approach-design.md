# The rendering approach — an act is acknowledged, and a region says which of four states it is in

`[2026-08-21]` Design spec for **Phase 1** of
[The surface tells the reader what it is doing](./01a02654-a920-7123-9c24-328e50022ffe).
Task: [The rendering approach](./01a02655-3b83-7c13-a2fd-b67166b06c44). Supersedes nothing.

**Phase 1 is the rendering approach. Phase 2 is the reads themselves** — the anchor fan-out, the
composition, the re-fetch on panel open — and nothing here changes a single API call.
`[ruled — 2026-08-21, Pete]` *"Both, perceived first."*

---

## 1. The finding — this is two problems, and they were being treated as one

Every one of the twelve server loads awaits to completion before anything paints, and **no route
returns an unawaited promise**. That much was expected. What grounding changed is the shape of the
remedy.

### 1.1 What is on disk

| shape | routes | reads in `load` | value from streaming |
|---|---|---|---|
| **redirect** | `vault/`, `vault/…/[doc_type]/[ident]` | 0 | none — out of scope |
| **single-read** | `vault/[owner]/[context]`, `vault/all`, `vault/search`, `admin/access` | **1** | **low** |
| **scaffold + fill** | `vault/r/[ident]` | 1, then `Promise.all([content, trail, edges])` | **high** |
| **heavy composite** | `graph/[owner]`, `graph/[owner]/analysis` | 9 / 2 | **high** |

Two corrections to what was assumed before looking:

- **`admin/access` is a single-read load.** Its six `await`s are mostly in **form actions**, not in
  `load`.
- **Two of the twelve are pure `redirect()`** with no reads at all.

Environment, checked rather than assumed: **SvelteKit 2.x, Svelte 5.x**, so top-level promises are
already not auto-awaited and nothing needs upgrading. `routes/(app)/` and `routes/(public)/` are
already separate route groups. `export const ssr` / `csr` / `prerender` appear **nowhere** — every
route is on defaults. `navigating` is used **nowhere**.

### 1.2 The consequence — the split is by clause, not by route

**A single-read page has nothing to stream behind it.** The read *is* the page; streaming it would
paint an empty frame and then a list, which is strictly worse than what ships today. So the register's
one-class equivalence claim over "the authenticated application routes" — which that register itself
flags as its most attackable — **does not hold**, and this is the concrete reason.

Two mechanisms, each serving a different clause:

| Mechanism | Clause it serves | Scope |
|---|---|---|
| **Navigation feedback** | `every-act-is-acknowledged` | **All routes**, one layout-level treatment |
| **Streaming from the server load** | `arriving-and-settled-are-distinguishable` | Only the two shapes that have a remainder |

The cheapest change covers the most surfaces. Eight routes gain nothing from streaming and
everything from the first row.

---

## 2. The mechanism — **CONFORM** to what SvelteKit already offers

Await only what the first paint needs; return the rest as promises; consume with
`{#await}` / `{:then}` / `{:catch}`. The framework documentation is explicit about ordering:

> *"make sure the `await` happens at the end, otherwise we can't start loading comments until we've
> loaded the post"*

**No new endpoints, and the access token stays server-side.** All twelve loads keep their shape.

**Approach rejected: client fetch through `_internal/*` endpoints.** The precedent exists
(`CommandPalette.svelte` → `/_internal/search`), and Phase 1 builds **none**. Once panels are handled
by streaming, no genuine refetch-without-navigation case survives. It costs a hand-written endpoint
per read and gives up SSR for that data.

**Approach deferred: shallow routing.** See §4.

---

## 3. The contract — **EXTEND**, authorized by the register's clauses

`[ruled — 2026-08-21, Pete]` The evidence standard is **a behavioural contract witnessed in
component tests**, not numbers. Numbers are Phase 2 and arrive after deploy.

### 3.1 There is no time bound, and that is the design

The task body originally recorded *"an act changes the screen within a stated bound."* **That is
withdrawn.** In Pete's words it was *"mixing a measure with a framework."*

A millisecond budget in a component test measures the CI machine. The property actually wanted is
structural — **the first paint does not depend on the streamed data** — and it is deterministic:
render with a promise that never settles, and if the scaffold is missing, the dependency exists.

### 3.2 The five parts

| | Contract | Rendered against |
|---|---|---|
| **C1** | The scaffold does not depend on the fill | a never-settling promise → frame, title and every region's pending marker present |
| **C2** | A pending region declares itself **in text** | same → each streamed region carries words, so it reaches the accessibility tree rather than being a silent shimmer |
| **C3** | Failure is a third state, not a stuck second one | a **rejected** promise → failure text present, pending marker **absent** |
| **C4** | No two region states present alike | render twice and compare — the four states must differ pairwise |
| **C5** | Navigation is acknowledged | `navigating` non-null → indicator; null → none |

**C4 is differential**: it asserts two renders are *not the same* rather than asserting what either
looks like. That is the negative-face clause stated as directly as it can be, and it survives a later
redesign of both states, which a fixed-string assertion would not.

### 3.3 The gap between the tests and the clauses, stated because it is easy to bank wrongly

**The tests assert *difference*. The clauses assert *distinguishability*.** C4 is satisfied by a
single character, or one pixel of colour. The clause says no two states may present alike **to a
reader** — and a grey *"no history"* beside a grey *"history unavailable"*, differing only in wording
at 11px, passes C4 and fails the clause.

This is the shape of D1's circular witness: an assertion that looks like it covers the clause and
covers something adjacent and weaker.

**So the contract requires the four states to differ by more than one channel** — words, not only
colour; a marker, not only opacity. That is what makes C4 check something a person could actually
resolve, and it shrinks the judged remainder to its real size rather than its rhetorical one.

---

## 4. The panel case — shallow routing is **deferred**, and the cost is named

Opening the rail has two separable defects:

1. **Its own reads block before anything paints** — `Promise.all([body, trail])` inside the load.
2. **It re-runs the entire page load**, re-fetching 130 nodes and 275 edges to show a panel.

**Streaming fixes (1) completely.** The rail's title, doc type, home and neighbour rows all come from
`model.nodes`, **already on the client**. Only the excerpt and trail are reads. The rail frame paints
fully populated with two regions arriving.

**(2) is a reads problem, not a rendering one**, and belongs to Phase 2 by the phase split.

**The cost shallow routing would have to justify.** #744 established that no read can forget the
selection: `GraphRead` is a subtraction type, so a branch that tries to decide `selected` is an excess
property and **fails to compile**. That guarantee was bought after the defect shipped and sat for
months — clicking any mark on the entry read opened nothing, for every node. Moving selection into
client-side page state **gives it back**, and shallow routing additionally needs a no-JS fallback and
modifier-key bail-outs.

Carried as [Opening a panel re-runs the whole page
load](./01a0265c-7064-7371-993c-b907dd46f9f5).

### `[verified — 2026-08-21]` The open question is answered, and it narrows the problem

This section used to end *"whether SvelteKit re-runs this load on a search-param change at all, or
whether fine-grained `url.searchParams` dependency tracking makes the question moot. Unverified, and
it would change the answer completely."* It did change it.

**SvelteKit tracks search params individually.** `load_data.js:64` and `client.js:877` both record
each param a load reads — `uses.search_params.add(param)` — and `client.js:1010-1011` re-runs a load
only when a param **it actually read** has changed. Fine-grained tracking exists and is on.

So the re-run is not the framework being coarse. **This load re-runs on a panel open because it
reads `sel`** — `parseGraphAddress` reads `url.searchParams` (`vault-url.ts:209`) and pulls the
selection out of it — and reading it makes all nine reads depend on it. It reads `sel` for one
reason: to resolve the selection server-side and start the rail's two reads.

That reframes the deferral. The problem is not *"shallow routing is complex"*; it is **one query
param, read in a load that needs it only for a panel**. Shallow routing is one way to stop reading
it, and it is no longer obviously the cheapest — which is what the Phase 2 task should now weigh.
`[Pete — 2026-08-21]` *"I don't understand why the graph needs to reload when a panel opens — if the
code is coupled in that way, that's a bad design."*

---

## 5. Error handling — four states, two catches, and a degradation target that is wrong today

### 5.1 `[found — 2026-08-21]` A failed read is indistinguishable from an empty one, twice

```ts
// NodeRail.svelte
const history = $derived(trail ? trailModel(trail) : []);
```

A **failed** trail read degrades to `null` → `[]`. A resource with **genuinely no history** →
`trailModel({events: []})` → `[]`. Identical. The excerpt has the same shape: `readResourceBody`
returns `null` on failure, and a resource with no body is also `null`.

**This is live on the panel this goal's first reports were filed against.** It is what forced the
register amendment of the same date — the negative face had enumerated the dangerous state pairs by
hand and missed *failed vs empty*.

### 5.2 The degradation policy is right; its target is not

The existing rule — *"a failed side-read must never take down a screen whose marks are all drawn"* —
**is correct and stays**. What changes is what it degrades **to**. Degrading to `null` asserts *"there
is nothing here"*, which is a claim about the reader's material that nothing verified. It must degrade
to a **named failure**.

A failed region says **what** failed — *"history unavailable"*, never *"something went wrong"* — so
the rest of the panel stays trustworthy.

### 5.3 Two catches that look like one concern

- **`.catch()` attached at promise creation** prevents an unhandled rejection **crashing the server**.
  SvelteKit's own `fetch` is handled; a manually created promise is not — and every read in this
  codebase is manually created.
- **`{:catch}` in the template** renders the failure.

**Neither substitutes for the other, and having one makes it easy to believe you have both.** This
will bite on the first route converted.

### 5.4 The refusal — the one duration in the design

The register's refusal face allows the system to **decline to keep waiting**, which is well-formed
rather than a failure: the reader is told the system gave up.

This is the **only** place a number is required. `[ruled — 2026-08-21, Pete]` **8 seconds** —
*"we have to bound it somewhere."*

The register still specifies no duration and deliberately never will: a budget is a build decision,
not an invariant, so this ruling changes no clause. It is recorded here and in
`src/lib/server/bounded.ts`, and nowhere else, so a number that turns out wrong is changed in one
place. **No measurement backs it**; instrumentation is Phase 2.

---

## 6. Testing

The idiom exists — `render(Page, { data })` in `*.component.test.ts`. The new capability is that
`data` may hold a promise, making the branches directly renderable. No timers, no fake clocks.

**Each contract is probed**, per standing discipline: break the exact invariant asserted and confirm
the failure lands in the right direction. C3's probe is the one that matters most — make the failure
branch render the pending marker, and C3 must go red. That is the perpetual-skeleton bug, caught by
construction.

**Plus one route-level guard**: assert the load returns an **unsettled promise** rather than a value.
Cheap, and it catches the regression most likely to actually happen — *someone adds an `await` and
quietly restores blocking*.

### 6.1 What none of this witnesses

**The streaming transport.** A jsdom component test renders the template's branches. It does not
exercise the server serializing an unsettled promise, the chunked response, or the client reattaching
to it. If that breaks, every one of C1–C5 stays green.

That is *"69 green tests and zero callers"* in a new costume. `temper-e2e` exists and is the only
thing that could witness it end to end. **Named as an uncovered remainder, not built here.**

---

## 7. Clause impact

Written because the register's rule is that **coverage is never inferred from absence**. Two of these
rows are the reason this section exists.

| Clause | Phase 1's effect |
|---|---|
| `every-act-is-acknowledged` | **Covered for navigations, NOT for form actions** — see §7.1. C5 on every route, C1 wherever there is a scaffold |
| `arriving-and-settled-are-distinguishable` | **Covered.** C2, and it must carry words rather than only a shimmer |
| `no-two-region-states-present-alike` | **Covered, with a stated weakness.** C4 plus the more-than-one-channel requirement. §3.3 names exactly how the test is weaker than the clause |
| `responsiveness-is-never-bought-with-a-false-claim` | **Preserved structurally, and cheaply.** Phase 1 renders only what a read actually returned — there is no optimistic path to go wrong, because none is built. Worth stating rather than assuming: it is preserved by the *absence* of a mechanism, so anything later that adds one puts this clause back in play |
| `working-and-stopped-are-distinguishable` | **PARTIAL, and this is a declared hole.** C3 distinguishes a read that **failed**. A read that simply **never answers** is stopped without failing, and nothing in C1–C5 catches it — only the refusal in §5.4 does, and that is a recommendation Pete has not ruled. **Until a bound exists, a hung read presents as arriving forever**, which is the clause's exact failure |
| `acting-on-a-part-does-not-discard-the-whole` | **PARTIAL, and it very nearly went backwards — see §7.3.** Opening a panel still re-runs the whole load; what changed is that the marks the reader is looking at now survive it. The clause's *visible* failure is closed on the graph canvas; the wasteful re-read behind it is untouched and is §4's deferral |

### 7.3 `[found in review — 2026-08-21]` Streaming made this clause worse before it made it better

The row above used to read *"NOT COVERED… redraws the canvas the reader was looking at."* That
sentence described the behaviour **before** streaming, and streaming changed it: because the load now
returns before the read answers, `data` updated and the `{#await}` fell back to its pending branch.
Measured: **50 marks replaced by "Loading graph…"**, then redrawn. A redraw is not a blank-out, and
the spec had come to disagree with the code in the direction that flatters the work.

**Fixed rather than redeclared.** The marks are held across a navigation that cannot change them —
the key is the address with `sel` removed, since `q`/`in`/`from`/`depth` decide the read and `sel`
decides none of it — so the incoming model is the *same* model and keeping the marks discards
nothing. A changed question takes them down, because marks left under a question the reader has
replaced would be the false claim `responsiveness-is-never-bought-with-a-false-claim` names. The
arriving words still show, over the canvas rather than instead of it.

**The lesson is the shape, not the instance.** A deferral was recorded, the mechanism that made it
worse shipped, and the record kept describing the old behaviour — so the remainder read as *stable*
when it had grown. A declared remainder is a claim about current behaviour and goes stale like any
other.

### 7.1 `[found in review — 2026-08-21]` The first row conflated *routes* with *acts*

It read *"Covered. C5 on every route."* Every route is true and is not the claim the clause makes.

`$navigating` is set only inside SvelteKit's `navigate()` (`client.js:1639`). A **form action**
submitted with `use:enhance` does not go through it — it applies its result and invalidates — so
`navigating` stays `null` and `NavProgress` never appears. The two submissions at
`admin/access/+page.svelte:74,81` — **approve** and **reject** an access request — are acts by any
reading, they are consequential, and they get **no acknowledgement at all**.

The row now says navigations rather than acts. Closing it means acknowledging a submission, which is
a different mechanism from `navigating` and is not built here.

### 7.2 `[found in review — 2026-08-21]` Streamed regions are absent from the SSR HTML

A streamed region renders only its **pending** branch into the server's HTML, by construction — the
value has not arrived when the document is written. So the graph canvas, `GraphA11yList` and the
resource page's markdown body are no longer server-rendered, and **with JavaScript disabled they
never resolve**.

These are authenticated application routes, so search indexing is moot and the register excludes
unauthenticated readers. It is recorded because §4 treats a no-JS fallback as a cost worth naming
when weighing shallow routing, and §9 named no such cost for streaming — an asymmetry that would let
a reader conclude streaming had none.

**The second of those is the honest cost of deferring shallow routing**, and it should be read
alongside §4 rather than separately: the deferral is defensible, and it leaves a clause with no
mechanism until Phase 2.

---

## 8. Build order

| | | Depends on |
|---|---|---|
| **P1a — navigation feedback** | `every-act-is-acknowledged`, all routes, one layout-level treatment. C5 | — |
| **P1b — the four-state vocabulary** | The shared way a region says arriving / present / empty / failed, differing by more than one channel. C2, C4 | — |
| **P1c — the rail** | `vault/r/[ident]` and the graph rail: scaffold from data already held, stream excerpt and trail, degrade to named failure. C1, C3 | P1b |
| **P1d — the heavy composites** | `graph/[owner]`, `analysis` | P1b, P1c |

P1a is deliberately first: it is the smallest change, it covers the most routes, and it is the only
one that helps the eight routes streaming cannot.

---

## 9. What this does not do

- **It does not change any read.** No API call, no query, no fan-out. Phase 2.
- **It does not fix the panel re-fetch.** §4, filed.
- **It does not build `_internal/*` endpoints.** §2.
- **It does not set a duration** other than the refusal's, and that is a recommendation.
- **It does not touch the public route group.** The register excludes unauthenticated readers, and
  that exclusion holds for rendering policy.
- **It does not measure anything.** No instrumentation; the register names that axis thin and leaves
  it open.

---

## 10. How this gets verified

Component tests are the pre-merge evidence and they are strong for the template's branches and weak
for everything else — §6.1 says exactly which.

**A PR preview cannot reach any authenticated page**, unchanged from the graph arc: `/graph/@me`
returns `?error=auth_state_lost` because Auth0 callbacks are per-domain. So the real witness arrives
**after the deploy**, and the thing to look at is not a stopwatch — it is whether a region that is
still arriving ever looks like one that came back empty.
