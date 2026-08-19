# Atlas Beat B — Home reframe: the build / research field (spec)

**Status:** implementation spec, ready for plan. Second beat of the Atlas reshape
(built after A; sequenced ahead of C deliberately — see below).
**North star:** `docs/superpowers/specs/2026-07-06-atlas-reshape-projection-class-north-star.md` (vault research `019f39ca`).
**Companion:** builder/researcher personas decision doc (vault `@me/temper`, written alongside this spec).
**Goal:** `019f28a1`. **Builds on:** Beat A field-effect + force layout (shipped, held on `jct/atlas-reshape`).

---

## 0. Why B before C (the sequencing decision)

The north star's advisory order was A → C → B. We deliberately do **B (Home) next**.
Home is the **entry** to the whole Atlas; reshaping it first lets us walk the user's own
**orient → wayfind** path and let that lived flow *shape* everything downstream. C and
later beats will very likely be **markedly changed** by the model this beat establishes —
that reshaping is the intended payoff of starting at the user flow, not a risk to avoid.

## 1. Purpose (the act this surface serves)

Home is the **orientation** surface for *your whole footprint*: `(substrate, perspective,
time) → small structured survey`. Its job is to let someone — human or agent, in the
moment they arrive — grasp *what they can do and reach* at a glance, and step into it.
The orientation contract governs: **few sized containers, linked, not a wall**; magnitude
visible; smallness *is* the attention-preservation.

Today's Home fails this as a **placeholder**: a static three-column `you → teams →
cogmaps` membership graph of uniform door-rects. With enterprise launch approaching, its
deeper failure is an **understanding-blocker**: it is organized around *our* internal
data artifacts — "contexts" and "cognitive maps" — and asks the user to learn temper's
ontology (clustering → regionality → coherence; workflow-scoping contexts) to find their
own work. **No one arrives wanting to learn that.** They arrive to *do a job*.

## 2. The reframe — jobs, not artifacts; postures, not roles

Home is organized around **what you are here to do**, expressed as two **verbs**:

- **build** — use temper's workflow tooling (goals · tasks · sessions · decisions ·
  research) in the contexts your work lives in, personal or team. JTBD: *do and track my
  work.*
- **research** — discovery: create new knowledge across systems and have temper become
  aware of it, and use the maps you can reach to answer questions / do analytic + creative
  work. JTBD: *find, learn, synthesize.*

These are **postures, not roles** — the same person (or agent) flips between them by the
moment. Two load-bearing wording rules:

1. **Verbs, not nouns.** The surface says **build** / **research**, never "builder" /
   "researcher" — a noun asks the user to self-label *who they are*; a verb frames *what
   they are here to do today*.
2. **No ontology leak.** The words "context" and "cognitive map" never appear in the
   Home UI. A context is surfaced as a place *your work lives*; a cogmap as *knowledge you
   can explore*.

(The persona theory — builder/researcher, their overlapping JTBDs — lives in the
companion decision doc "for ourselves," not on the surface.)

## 3. The Home surface — composition + interaction

One field panel under two verb-CTAs. This **reuses Beat A's field-effect + force layout**
for Home itself (the reshape's payoff): hazy glow-field that *resolves* to crisp, sized,
force-separated bodies.

**Layout**
- Two visually pronounced **verb-CTAs**: **build** (left) · **research** (right), each
  with its one-line tagline in subtext.
- A **main field panel** beneath them.
- The **`you` node is dropped** — self is implied (the whole page is yours; the build
  field *contains* your personal contexts as bodies, so "you" needs no glyph).

**Interaction (the heart of the beat)**
- **Rest:** the panel is a **hazy, undifferentiated on-theme field** — Beat A's glow/haze
  language, unresolved. It signals "a rich field is here; pick a lens."
- **Hover `build`:** the field **resolves** to *your contexts* (personal `@me/*` + team
  `+team/*`) — crisp, force-separated, **sized by magnitude**, on the build color, each
  **directly navigable**. The research field dims/hazes behind.
- **Hover `research`:** the alternate color resolves to *the cogmaps you can reach* — sized
  by magnitude, each **directly enterable**. The build field dims behind.
- **Click either CTA:** **commit** — the panel becomes *only* that lens's crisp field, with
  **no** dimmed counterpart. **Back** is a real history step returning to the neutral
  two-CTA selection.

**The two lenses**
- **build lens → your contexts.** Every context your work lives in, across personal and
  all your teams, as sized bodies. Sized by **resource count**. Clicking a body navigates
  **directly into that context's work surface** (today: `/vault/@me/<ctx>` or
  `/vault/+<team>/<ctx>`; the Atlas-native "scope panorama = contexts" treatment is Beat C).
  Bodies may be grouped/tinted by owner-scope (personal vs each team) — a harness-tuned
  detail, not a hard requirement.
- **research lens → your cogmaps.** Every cogmap you can reach (via team associations), as
  sized bodies. Sized by **`region_count`** (already in the read; ships with no new
  derive). Clicking a body **enters the cogmap panorama** (Beat A, shipped). Per-cogmap
  resource-count / connection-density sizing is a **later enrichment**, not a B blocker.

**Why contexts-grain doesn't become "a wall."** The orientation contract forbids a wall
of labels. Lens-gating + the field-effect are exactly what prevent it: you only ever see
*one* resolved lens at a time; bodies are sized, hazy-gated, and force-separated (Beat A's
top-K label gate applies). The set is *your* footprint (member/personal), bounded — not a
global context list. This is the evolution from the north star's "(i) teams-grain" Home to
**contexts-grain via lens**, which serves the JTBD directly.

## 4. Data contract (fixture-first — the UI's needs define the read)

The current Home read (`readAtlasHome` → `AtlasHome { teams, cogmaps }`) does **not** carry
what the build lens needs: it returns *teams*, but the build field is a field of
**contexts** (personal + team), each sized and directly navigable.

**Method — fixture drives contract.** We do **not** guess the read shape up front. Per the
`/dev/atlas` harness workflow (`[[feedback_local_proddata_render_harness_for_ui]]`):

1. **Copy** the committed `home` fixture into the gitignored local override
   (`static/dev/atlas-fixtures.local.json`), **hand-shaped** to the target: a `build` list
   of contexts `{ id, name, owner_ref, resource_count }` and a `research` list of cogmaps
   `{ id, name, region_count }`.
2. **Point the dev server at the copy** and iterate the Home UI against it until the
   interaction + shape are locked (`bun run dev` → `/dev/atlas`, `home` scenario).
3. The locked fixture shape **is** the contract. Only then do we implement the backend to
   produce it: extend the Home read (new `AtlasHome` shape or a sibling read) + any SQL
   function / service change, wire the `ts-rs` types, and swap the harness back to
   real-shaped synthetic fixtures.

**Target shape** (locked against the harness in Task 1 — the spike surfaced that
research needs a scope indicator too, so `research` carries `owner_ref`):

```
AtlasHome {
  build:    [ { id, name, owner_ref, resource_count } ],   // contexts, personal + team
  research: [ { id, name, owner_ref, region_count } ],     // reachable cogmaps + held-by scope
}
```

**`research.owner_ref` is a derived "held-by" scope** — a team `+slug`, or a universal
marker (e.g. `temper`) for the public/system kernel — **not** the raw `team_ids`. The
research lens tints by it (universal = base warm anchor; each team = a warm-band hue),
mirroring how build tints by context owner-scope. Deriving it from a cogmap's team
membership (first/primary team, or `universal` when system/public) is the backend's job
in Task 6.

**Reuse, not duplication.** "Contexts visible to a profile, with sizes" is machinery Beat
C also needs (its team panorama = contexts). Factor it as a shared service read so B's Home
field and C's team panorama draw from one source. All reads stay **visibility-scoped**
(`contexts_visible_to` / `resources_visible_to`) — a context or cogmap the caller can't see
never appears.

## 5. URL / state

The committed lens lives in the URL — consistent with the Atlas "URI frame" (tier derived
from the URL, never stored):

- Neutral (rest) = **no** home-lens param.
- `?home=build` / `?home=research` = committed lens; set with **pushState** so **Back**
  returns to neutral (matches the drill-history convention in `nav.ts`).
- Hover-preview is **ephemeral** (no URL change) — only a **click/commit** writes `?home`.
- Add `buildHomeLensUrl(base, 'build'|'research')` + a parser to `nav.ts` beside the
  existing scope/cogmap builders.

## 6. Accessibility

- **Focus = hover.** Focusing a verb-CTA resolves its field the same way hover does
  (pointer-free users get the preview); Enter/click commits.
- **List fallback.** Each lens has a non-spatial equivalent — a **list of links +
  metadata** (context name · owner-scope · resources; cogmap name · regions) — the
  accessible twin of the field. Reuses Beat A's a11y-list-fallback pattern.
- Every field body is **keyboard-focusable and enterable** (`atlas-focusable`
  role/tabindex, as Beat A). Small/low-magnitude bodies are never keyboard-dead.
- The neutral state announces both acts; committing updates the accessible name to the
  active lens.

## 7. Frontend changes (`packages/temper-ui`)

Exact componentry is locked on the harness; the intended shape:

1. **`components/graph/atlas/TierHome.svelte`** — rebuilt: two verb-CTAs + taglines, the
   field panel, and the hover-resolve / click-commit / Back-to-neutral state machine.
   Drops the `you` node and the three-column `YOUR TEAMS` / `COGMAPS` scaffold.
2. **`lib/graph/atlas/layout/homeLayout.ts`** — replace the three-column `layoutHome` with
   a **field layout** for each lens, built on the Beat A **`forceTerritories`** force
   layout (deterministic, pure, unit-tested) so bodies are sized + separated like the
   panorama. `HomeNode`/`HomeEdge`/`HomeGraph` types change; old three-column tests retire.
3. **Field bodies** reuse Beat A's **`TerritoryCircle`** field-effect (intensity → glow +
   opacity, size → magnitude) and the top-K label gate, tinted per lens (build vs research
   color; add to `palette.ts` as one source of truth).
4. **`nav.ts`** — `?home` lens builder + parser (§5).
5. **`+page.server.ts`** (graph `[owner]`) — the no-scope Home branch consumes the new
   read shape (build contexts + research cogmaps) and passes `?home` through.
6. **A11y list fallback** component/section per lens (§6).
7. **Regenerated `ts-rs` types** for the new `AtlasHome` shape ride along.

## 8. Backend changes (`temper-api` / services / SQL)

Derived **after** the fixture locks the shape (§4). Expected:

- A **Home read** returning build-contexts (personal + team, visibility-scoped, with
  `resource_count`) and research-cogmaps (with `region_count`). New/extended service read;
  new SQL function or a compose of existing visibility functions — decide on efficiency,
  per the north star's data-model guidepost.
- Wire type change on `AtlasHome` (temper-core, `ts-rs`); regenerate TS types.
- If a new/changed SQL macro query lands: regenerate the relevant `.sqlx` cache
  (`prepare-services` / `prepare-api` per the crate the query lives in), and — if a new
  migration — remember the compile-time-embedded migrator gotcha (`touch` the crate `lib.rs`).
- **Additive-on-`main`** invariant holds (new read/columns; no destructive change to
  shipped functions the current UI still calls until the swap lands).

## 9. Testing

- **`homeLayout` / `forceTerritories` reuse** — deterministic positions (same input →
  same output, no `Math.random`); no-overlap/containment for each lens field.
- **Home state machine** — unit tests for the lens reducer: rest → hover-preview (no URL) →
  commit (`?home` set, other field gone) → Back (neutral). Pure where possible.
- **`nav.ts`** — `?home` build/parse round-trip; neutral = absent param.
- **Fixture guard** — extend `fixtures.test.ts`: the `home` scenario carries the new
  build/research shape; key-set pinned to the new `AtlasHome` type via `satisfies`; no
  personal-data leak in the committed synthetic bundle. Update `sanitize-atlas-fixtures.mjs`
  for the new fields.
- **Backend e2e** (`test-db`) — the Home read returns only visibility-scoped contexts +
  cogmaps; a **deny-direction** test (a context/cogmap the caller can't see is absent).
  Run the access-sensitive e2e tier (`[[feedback_access_semantics_changes_need_e2e_tier]]`),
  and — if e2e spawns the CLI — rebuild the bin first
  (`[[feedback_nextest_does_not_rebuild_spawned_temper_bin]]`).

## 10. Decisions (locked on the harness — Task 1 spike)

1. **Rest-state haze = union-haze (LOCKED).** At rest the field renders a **hazy union of
   *both* lenses' bodies** overlaid at low opacity (no labels); hover/commit resolves one
   lens to crisp clarity and fades the other. The slight muddle is *intended*: "select →
   focus attention → reveal clarity" visually foregrounds the page's purpose. Revisit only
   if it wears badly in use.
2. **Per-scope tint, no spatial grouping (LOCKED).** Bodies are tinted by owner-scope; the
   force layout is **not** grouped by scope (organic single field). **Build** (cool family):
   personal `@me` anchors at a base cool blue; each team drifts across a blue→indigo band
   keyed by `owner_ref`. **Research** (warm family): the universal/system kernel anchors at
   base orange; each team drifts across a warm band keyed by `owner_ref`. Tint override on
   `TerritoryCircle` (`tint?` prop) carries it. Hue-spread is tunable; current spread ratified.
3. **Research sizing = `region_count` (LOCKED defer).** Per-cogmap resource-count /
   connection-density enrichment deferred.
4. **Build-body destination = vault (LOCKED, temporary).** Until Beat C, clicking a context
   body lands on `/vault/<owner>/<ctx>`; the Atlas-native contexts panorama is Beat C.

## 11. Scope boundaries / captured for later

- **In scope (B):** the Home surface (two verb-lens field, interaction, a11y), the Home
  **read** (build contexts + research cogmaps, sized, visibility-scoped), `?home` URL
  state, fixture + type regeneration.
- **Beat C (reshaped by this beat):** "scope panorama = contexts" for teams — and now an
  **open thread**: with Home surfacing contexts *directly*, how a **team panorama** (one
  team's contexts + its cogmap doors) is reached is a C-time question. The team-door path
  may be reshaped or retired. Captured here, resolved in C.
- **Deferred:** per-cogmap resource/density sizing (§10.3); Atlas-native build-body
  destination (§10.4 → C); context view re-imagining (Beat E / Chunk D).

## 12. Connections

- North star `019f39ca`; goal `019f28a1`; Beat A spec
  `2026-07-06-atlas-beat-a-cogmap-knowledge-field-spec.md`.
- Framework: projection-class docs `019e54b9` / `019e54bb` / `019e5530` / `019e552c`
  (the orientation attention-contract is the source of the size + gate + "not a wall"
  decisions).
- `[[feedback_local_proddata_render_harness_for_ui]]` (fixture-first loop drives the
  contract), `[[feedback_read_gate_must_match_full_canonical_visibility]]` (deny-direction
  test on the Home read), `[[feedback_access_semantics_changes_need_e2e_tier]]`,
  `[[feedback_nextest_does_not_rebuild_spawned_temper_bin]]`,
  `[[project_graph_atlas_visualization_goal]]`.
