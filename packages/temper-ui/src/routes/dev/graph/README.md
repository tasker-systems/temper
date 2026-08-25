# Graph render harness (`/dev/graph`, `/dev/analysis`)

Two **dev-only** routes that render the real `GraphPage` and `AnalysisPage` against committed,
personal-data-free fixtures — **no auth, no server reads, no merge-to-prod**. Both
`throw error(404)` outside `dev`, so they are inert in any deployed build.

## Why this exists, which is not "developer convenience"

**Vercel previews cannot carry Auth0.** So authenticated UI is otherwise observable only in
production, post-merge. `/dev/atlas` closed that gap for the predecessor and was deleted in Beat D
along with the page it rendered; nothing replaced it, and the successor surface shipped four beats
without a place to look at it.

`src/test/README.md` names three layers and this is the third: *"appearance and legibility, on
realistic data, judged by a human eye."* The first two structurally cannot see what it sees — jsdom
computes no layout and `ResizeObserver` is a no-op stub, so **no test in this repository may claim
anything is legible.** One session on the live surface produced three findings no test could have
caught: a list that reads as ranked, a button that does not read as a button, and a floor number
nobody has measured. Every one needed a person looking at a rendered surface.

## Running

```bash
cd packages/temper-ui
bun run dev
# http://localhost:5173/dev/graph
# http://localhost:5173/dev/analysis
```

Pick a scenario and a viewport. The scenario buttons are derived from the bundle's own keys, so a
re-capture that adds a scenario adds its button. Each scenario prints its own `_why` above the
frame, and — where it has one — **what it cannot witness**, in the fixture's own words.

### The viewport presets are thresholds, not devices

Two live rulings on this surface are about size specifically, and neither is observable in jsdom:

- **`CANVAS_FLOOR_PX = 704`** (`$lib/graph/instrument.ts`) is deliberately unmeasured. *Why these*
  yields to a strip below it. The presets bracket it: 770 (above), 704 (at), 610 (below).
- **`.instrument`'s `@media (max-width: 900px)` stacking rule has never once fired** — the
  panel-bearing selector out-specifies it. The 900 and 760 presets are where a reader would find
  out whether that matters.

## What the scenarios cover

The bundle is a **corpus-shape suite**, not a screenshot set. Every anchor below was *discovered*
at capture time, never hardcoded — region ids in particular are ephemeral, and a hardcoded one
previously produced a hard 500 in the predecessor's capture path.

| Scenario | The branch, and the shape it pins |
|---|---|
| `entry` | **The screen every reader meets first.** No question, no seeds — `GET /api/graph/entry`. Carries the unconnected band at its real size |
| `entryZeroRegion` | The entry read confined to a map with **zero live regions**. Still ranks, still draws — regions are not what `eligible` counts |
| `entryTooLittleStructure` | **Rung 2** — `eligible === 0`, the verdict that replaces the canvas with a sentence. The only scenario that reaches it |
| `question` | §2.1 — unaddressed with a question. One survey per anchor, unioned, walked. **Untrimmed** |
| `mapCharter` | §2.2 — a map surveyed under its own charter, borrowed off the list row by `questionFor` |
| `mapColdStart` | §2.2 on a map with material and **zero regions**. Draws nothing — see *What this found* |
| `contextEverything` | §2.3 — a context with no question shows everything in it, at real corpus scale |
| `contextZeroRegion` | §2.3 on a context with **355 resources and zero regions**. Draws a real graph |
| `traversal` | The `?from=` handoff — a walk that runs no composition. **Had no caller at all until this bundle** |

`/dev/analysis` runs off `graph-analysis-anchors.json`, which was already untrimmed: as captured on
**2026-08-20**, a context with 501 groupings, a cogmap with 406 and an analytics row, and a cogmap
that has **never materialized a region** — the last being the screen
`displaced-structure-remains-reachable` is judged on.

**`[2026-08-25]` That capture date is what the missing context analytics row means now.**
`/api/contexts/{id}/analytics` answers the staleness half for a context, so a context HAS an
anchor-level readout — the bundle carries none because the capture predates that door. Read the
line above as a fact about the CAPTURE, not about the world; `$lib/graph/harness.ts` records the
same remainder at the point where it infers an anchor's kind from that row's presence.

## What this found before anyone looked at it

**§2.2 on a zero-region map draws nothing, and §2.3 on one draws fine.** Spec §6 asserts
`the-unstructured-reader-is-never-worse-off` as *"A reader with many resources and zero regions
still gets a graph — §2.3's path needs no region at all."* True of §2.3; false of §2.2. A map with
no question borrows its charter, and a question routes through `survey`, which is **region-scored** —
so on a map that has never materialized a region the survey returns `disposition: 'empty'`, the
union is empty, `follow-from` seeds from nothing, and the bound line truthfully reports a walk that
completed over zero rows.

Pinned in `fixtures.test.ts` **as an assertion, not a skip**, so that fixing it fails the test.

## Regenerating the fixtures

Two steps: **capture** raw into the gitignored local file, then **sanitize** into the committed one.

### 1. Capture

```bash
cd packages/temper-ui
TEMPER_TOKEN="$(temper auth export-token | tr -d '"')" \
  bun run scripts/capture-graph-fixtures.ts --out src/test/fixtures/graph-harness.local.json
```

Four things worth knowing:

- **No browser is involved.** The graph route's reads are plain REST and the `temper` CLI already
  holds a production token, so the capture needs an authenticated *session*, not an authenticated
  *browser*. This is the one operational fact the task that commissioned the harness had wrong.
- **The tool imports the real plan builder.** `buildGraphPlan`, `readableAnchors` and `questionFor`
  are the very modules `+page.server.ts` imports, so a captured response answers the composition the
  route would actually have sent — by construction, with no hand-written plan free to drift.
- **It reports every pick to stderr.** That output is the record of which real places the bundle
  stands on. A shape that has vanished from the corpus is skipped with a `warning:`, never emitted
  as something else, and `fixtures.test.ts` then fails on the missing scenario. Skipping is never
  silent.
- **`_anchors` is captured, not reconstructed.** The load builds its anchor → home-label map from
  two list reads; the harness would otherwise carry a second copy free to disagree, and a node's
  home label is on screen.

### 2. Sanitize

The capture is raw — real titles, refs, handles, excerpts and ids — and **this repository is
public**, so the committed bundle is always the sanitized one.

```bash
node scripts/sanitize-graph-fixtures.mjs \
  src/test/fixtures/graph-harness.local.json src/test/fixtures/graph-harness.json
bun run test src/lib/graph/fixtures.test.ts   # verify the committed bundle is clean
```

The sanitizer remaps every UUID and replaces sensitive free text with deterministic synthetic
values **while preserving the exact structure** — every key, number, timestamp, hash and edge label
is the real one. Four of its rules are load-bearing and easy to break by "simplifying":

- **Replacement preserves the original's LENGTH** (to the nearest word). The harness exists to check
  legibility at the sizes the corpus actually reaches, and a label's rendered width is the thing
  being checked — so a 90-character region label must not sanitize down to two words.
- **And its DISTINCTNESS.** A first version preserved length only; two region labels collided onto
  one string, `GraphPage` renders groupings in a keyed `{#each}`, and twelve component tests failed
  with `each_key_duplicate`. That is this repo's standing lesson one level down — *a trim that
  preserves one property destroys another* — with the remap as the trim.
- **Some keys mean two things, and only a PATH can tell them apart.** `label` is relationship
  grammar under `via[]`/`edges[]` and a borrowed resource title under `shape[]`/`shape_rows[]`.
  `name` is a stage id under `stages[]` and a cogmap's name under `borrowedFrom`. `question` is the
  reader's own words when they asked, and **a map's authored charter** when `questionFor` borrowed
  it. All three were live leaks caught after the key list looked complete.
- **The leak guard is positive, not a denylist.** `fixtures.test.ts` asserts every free-text value in
  a sanitized position is built from `scripts/graph-synthetic-vocabulary.mjs`'s word bank. A denylist
  only catches strings someone thought to list — the `borrowedFrom.name` leak above was found by one
  *only* because that map's name happened to contain the word `temper`.

Keep the raw `.local.json` locally; it is gitignored.

## What the harness does NOT cover

- **Anything address-decided.** `no-place-resolved` and `nothing-to-ask` are refusals settled from
  the URL above any read, and a fixture is an answer that already came back. The component suite
  covers both.
- **Rung 2 with material.** `entryTooLittleStructure` reaches `eligible === 0` and also has
  `in_scope === 0`, so it renders the degenerate sentence — *"You can read 0 resources here, but
  nothing is linked to anything else"* — rather than the one the rung exists for. **No readable
  anchor on the corpus has `eligible === 0` and `in_scope > 0`**; that was measured across every
  anchor at capture time, not assumed. Declared in the fixture's own `_does_not_witness`, which the
  harness prints above the frame.
- **The `?sel=` rail.** Every scenario renders with nothing selected. Adding a selection is a
  fixture change, not a code one.
