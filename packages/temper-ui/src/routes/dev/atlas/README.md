# Atlas render harness (`/dev/atlas`)

A **dev-only** route that renders the real `AtlasPage` shell against captured,
real-shaped JSON fixtures — **no auth, no server reads, no merge-to-prod**. Vercel
previews can't carry Auth0, so authenticated Atlas UI was previously only
verifiable in prod post-merge (see the `reference_vercel_preview_no_auth0_verify_in_prod`
memory). This harness closes that gap: legend layout, territory-interior legibility,
and the responsive pass are all iterated in-branch.

The route `throw error(404)`s outside `dev`, so it is inert in any deployed build.

## Running

```bash
cd packages/temper-ui
bun run dev
# open http://localhost:5173/dev/atlas
```

Pick a **scenario** and a **viewport** preset (or type w/h). The scenario buttons are derived
from the bundle's own keys, so adding a scenario to the fixtures adds its button — see
[What the scenarios cover](#what-the-scenarios-cover) for what each one pins.
The frame clips like a real bounded viewport and is drag-resizable from its corner.
On a fresh checkout the harness runs against the committed synthetic fixtures — no
local capture required.

### The two context-door scenarios (Beat E)

- **`contextPanorama`** — the context door's Tier 0: several goal-container territories
  with a heavy-tailed `member_count` spread (so the `log1p` intensity ramp is visible)
  plus a non-empty residual tray (buckets for the resources that reach no container).
- **`contextDrill`** — a Tier-1 container drill (`focus: container`, `coreHome: 'context'`):
  the goal seed plus its members, mixing `home: 'context'` (rounded-square) and
  `home: 'cogmap'` (circle) nodes so both cross-home mark shapes render under the
  inverted radial.

These two were originally **hand-authored** synthetically, because the context door predated
any deployed instance of it and there was nothing live to capture. **No longer** — as of the
2026-08-01 refresh every scenario in the bundle is captured from a real database through the
real reads, the context door included. Nothing here is hand-shaped.

## Fixtures

Fixtures are a single bundle keyed by scenario, each value a full `AtlasViewData`
(the exact object the `/graph/[owner]` page load returns). The loader reads, in
precedence order:

1. **`static/dev/atlas-fixtures.local.json`** — your own raw capture, if present.
   **Gitignored** (holds real titles/handles/ids from a personal team). Use it to
   eyeball the harness against real data.
2. **`static/dev/atlas-fixtures.json`** — the **committed**, synthetic,
   personal-data-free bundle. The default: drives the harness on a fresh checkout,
   and is guarded by `src/lib/graph/atlas/fixtures.test.ts` (every scenario present +
   full `AtlasViewData` key set + no personal-data leak). The key-set assertion is
   pinned to the type via `satisfies Record<keyof AtlasViewData, true>`, so a page-load
   shape change fails `bun run check` until the fixtures are regenerated.

### Regenerating fixtures

Two steps: **capture** a raw bundle from a live database into the local override, then
**sanitize** it into the committed default.

**1. Capture** with `crates/temper-api/examples/capture_atlas_fixtures.rs`. It calls the same
`graph_service` / `context_graph_service` functions the HTTP handlers call and serializes their
real return types, so the payloads are the wire DTOs **by construction** — there is no
hand-transform step that could drift. Read that file's header beside this section; each
`capture_*` in it names the handler it mirrors.

```bash
# From the repo root. SQLX_OFFLINE is NOT optional — see below.
SQLX_OFFLINE=true \
DATABASE_URL="$(neonctl connection-string main \
    --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543 \
    --role-name neondb_owner --database-name neondb 2>/dev/null | tail -1)" \
  cargo run -p temper-api --example capture_atlas_fixtures -- \
    --handle <your-profile-handle> \
    --out packages/temper-ui/static/dev/atlas-fixtures.local.json
```

Four things worth knowing before you run it:

- **`SQLX_OFFLINE=true` is required.** `sqlx`'s compile-time `query!` macros read
  `DATABASE_URL`, so without it the *build* verifies every macro in the dependency tree
  against production instead of the committed `.sqlx` cache — slow, and it fails outright
  whenever prod's schema and `migrations/` have not converged. `cargo make` sets this
  globally, which is why the hazard only appears on a bare `cargo run`.
- **The session is read-only** (`default_transaction_read_only = on`), set on connect rather
  than trusted to the call sites.
- **Anchors are discovered, never hardcoded.** The tool picks the densest cogmap, a cogmap with
  material and no regions, the densest context, a context with no containers, and an empty one
  — then derives every region and node id from the payload it just read. Region ids are
  ephemeral (the steward re-sweeps clusters and folds the old rows); a hardcoded one previously
  produced a hard 500.
- **It reports its picks to stderr**, including region / singleton / orphan counts per anchor.
  That output is the record of which real places the bundle stands on. When a scenario's shape
  has vanished from the corpus it prints a `warning:` and skips it rather than emitting
  something that no longer means what the scenario name claims — at which point
  `fixtures.test.ts` fails on the missing scenario. Skipping is never silent.

<details>
<summary>Superseded: the browser-console capture (kept for reference)</summary>

The predecessor recipe drove a logged-in browser and captured SvelteKit's `__data.json`,
because the `/api/graph/*` reads are server-side and never touch the browser network tab. It
worked, but it needed a human to paste a script into devtools — so it was not reproducible by
anyone who was not in the session, and automated downloads throttled. It also had no
context-door path. Superseded by the Rust capture above.

```js
(async () => {
  // devalue unflatten (SvelteKit __data.json is flattened)
  const unflatten = (values) => {
    const hydrated = new Array(values.length), seen = new Array(values.length).fill(false);
    const h = (i) => {
      if (i === -1) return undefined; if (i === -3) return NaN; if (i === -4) return Infinity;
      if (i === -5) return -Infinity; if (i === -6) return -0; if (i === -2) return undefined;
      if (seen[i]) return hydrated[i]; seen[i] = true; const v = values[i];
      if (v === null || typeof v !== 'object') { hydrated[i] = v; return v; }
      if (Array.isArray(v)) { if (typeof v[0] === 'string') { hydrated[i] = v; return v; }
        const a = []; hydrated[i] = a; for (const j of v) a.push(h(j)); return a; }
      const o = {}; hydrated[i] = o; for (const k in v) o[k] = h(v[k]); return o;
    };
    return h(0);
  };
  const grab = async (qs) => {
    const r = await fetch('/graph/@me/__data.json' + (qs ? '?' + qs : ''), { headers: { 'x-sveltekit-invalidated': '01' } });
    const j = await r.json();
    const nodes = j.nodes.filter((n) => n && n.type === 'data').map((n) => unflatten(n.data));
    return nodes.find((d) => d && ('focus' in d || 'territories' in d || 'teams' in d)) ?? nodes[nodes.length - 1];
  };
  const home = await grab('');
  const cogmaps = home?.home?.research ?? [];
  // richest research cogmap = the one with the most materialized regions
  let best = null, bestRegions = -1;
  for (const c of cogmaps) {
    const p = await grab('cogmap=' + c.id);
    const regions = (p?.territories?.territories ?? []).filter((x) => x.kind === 'region');
    if (regions.length > bestRegions) { bestRegions = regions.length; best = { id: c.id, panorama: p, regions }; }
  }
  const COGMAP = best.id;
  // a region whose composition (Beat D drill) includes a context-homed node, so
  // nodeSelectedContext exercises the context-homed "View full resource" rail
  let pick = null;
  for (const rg of best.regions.slice(0, 12)) {
    const dr = await grab('cogmap=' + COGMAP + '&focus=territory:' + rg.id);
    const ns = dr?.neighborhood?.nodes ?? []; const ctx = ns.find((n) => n.home === 'context');
    if (!pick && ns.length > 1) pick = { rg, dr, ns, ctx };
    if (ctx) { pick = { rg, dr, ns, ctx }; break; }
  }
  if (!pick) { const rg = best.regions[0]; const dr = await grab('cogmap=' + COGMAP + '&focus=territory:' + rg.id); pick = { rg, dr, ns: dr?.neighborhood?.nodes ?? [], ctx: null }; }
  const REGION = pick.rg.id, REGION2 = best.regions.find((r) => r.id !== REGION)?.id ?? REGION;
  const NODE = pick.ns.find((n) => n.home === 'cogmap')?.id ?? pick.ns[0]?.id;
  const LEAF = [...pick.ns].sort((a, b) => (a.degree ?? 0) - (b.degree ?? 0))[0]?.id ?? NODE;

  // Beat D: a territory focus is the region → resources COMPOSITION drill (facets +
  // the context-resources they were derived_from); a `~`-join unions regions. Context
  // nodes open the rail via `?sel=node` on top of the territory focus (not a drill).
  const bundle = { _meta: { synthetic: false, captured_from: 'temperkb.io/graph/@me', note: 'real personal capture (gitignored)' } };
  bundle.home = home;
  bundle.cogmapPanorama = best.panorama;
  bundle.regionDrill = pick.dr;
  bundle.regionDrillUnion = await grab('cogmap=' + COGMAP + '&focus=territory:' + REGION + '~' + REGION2);
  bundle.nodeNeighborhood = await grab('cogmap=' + COGMAP + '&focus=node:' + NODE);
  bundle.nodeSelected = await grab('cogmap=' + COGMAP + '&focus=node:' + NODE + '&sel=node:' + NODE);
  bundle.leafBare = await grab('cogmap=' + COGMAP + '&focus=node:' + LEAF + '&sel=node:' + LEAF);
  if (pick.ctx) bundle.nodeSelectedContext = await grab('cogmap=' + COGMAP + '&focus=territory:' + REGION + '&sel=node:' + pick.ctx.id);

  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([JSON.stringify(bundle)], { type: 'application/json' }));
  a.download = 'atlas-fixtures.local.json'; document.body.appendChild(a); a.click(); a.remove();
  console.log('captured scenarios:', Object.keys(bundle).filter((k) => k !== '_meta'));
})();
```

(If Chrome blocks the download — a "multiple downloads" prompt in the omnibox — click Allow.)

</details>

**2. Sanitize** — the capture is raw (real titles, handles, excerpts, ids) and this repository
is **public**, so the committed bundle is always the sanitized one:

```bash
cd packages/temper-ui
node scripts/sanitize-atlas-fixtures.mjs   # → static/dev/atlas-fixtures.json (commit this)
bun run test src/lib/graph/atlas/fixtures.test.ts   # verify the committed bundle is clean
```

The sanitizer remaps every UUID and replaces sensitive free-text with deterministic synthetic
values while preserving the exact structure — so the committed bundle stays schema-honest but
carries no personal data. Keep the raw `.local.json` around locally; the loader prefers it when
present.

Three of its rules are load-bearing and easy to break by "simplifying":

- **Replacement preserves the original's LENGTH** (to the nearest word). The harness exists to
  check legibility at the sizes the corpus actually reaches, and a label's rendered width is the
  thing being checked — so a 90-character region label must not sanitize down to two words, or
  the harness reports a layout as legible that production truncates.
- **`label` is two different fields under one key.** An *edge* label is relationship grammar
  (`derived_from`, `advances`) and must survive; the legend renders it. A *territory* label is
  not — `graph_cogmap_territories` computes it as `COALESCE(reg.label, seen.rep_title)`, so an
  unlabelled region borrows a member's resource **title**, and a container's label *is* its
  goal's title. The rule is path-scoped, not key-scoped.
- **The leak guard is positive, not a denylist.** `fixtures.test.ts` asserts every free-text
  value is built from the sanitizer's own word bank. A denylist only catches strings someone
  thought to list, and the Rust capture opened three surfaces the browser capture never had:
  `excerpt` (real first-paragraph prose), `actor_name` (real display names), and territory
  labels. Both guards are kept — they fail on different things.

### What the scenarios cover

The bundle is a corpus-shape suite, not a screenshot set. Each entry below is a shape measured
on prod **2026-08-01**; the pre-refresh bundle (captured 2026-07-09) could express none of the
last four, so a design validated against it was validated against a corpus that no longer
exists.

| Scenario | Shape it pins |
|---|---|
| `home` | **Concentration, not average** — one enormous anchor and a tail of near-empty ones |
| `cogmapPanorama` | **Singleton-dominated** — 64.5% of live cogmap regions hold exactly one member |
| `regionDrill` / `regionDrillUnion` | Region composition; one region, and a `~`-joined union |
| `nodeNeighborhood` / `nodeSelected` / `nodeSelectedContext` / `leafBare` | Node tiers, both homes, and the neighbour-less leaf |
| `contextPanorama` / `contextDrill` | **A genuinely clustered anchor** — containers with real multi-member spread |
| `coldStartCogmap` | **Material, zero live regions.** 6 of 11 non-empty anchors, incl. the L0 kernel |
| `coldStartContext` | Volume with **no container structure** — every resource in the residual tray |
| `residualDrill` | **Region-less material is reachable** — 22.6% of active resources are in no live region |
| `emptyAnchor` | A wholly empty anchor renders as a view, not a failure |

**Live vs folded.** `kb_cogmap_regions` retains superseded rows under `is_folded`, and folded
rows are **84%** of the table. Every consumer that matters filters `NOT is_folded`. The capture
tool inherits this for free by going through the service layer; a dump-based transform that
forgot it would inflate every region count ~6× and produce fixtures no surface would ever
receive.

#### Known gap: context regions are not capturable

There is no scenario for *"a context's own regions"* and this is a substrate limit, not an
omission. `graph_cogmap_territories` filters `WHERE reg.cogmap_id = p_cogmap`
(`migrations/20260713000050_region_visible_member_count.sql`), and a context region carries
`cogmap_id IS NULL` — so the read is structurally blind to them, exactly as
`20260713000010_anchor_orientation_reads.sql`'s own header says. `TerritoryKind::Cogmap` has no
producer at all. The context door draws **goal-rooted containers plus a residual tray**, not
regions.

This matters because the two-kinds-of-region contrast — a cogmap region is a connected clump of
the *declared* graph (`w_cos = 0`), a context region is a *semantic neighbourhood*
(`w_cos = 1`) — is a finding the design has to answer, and only one half of it is reachable
through the API today. `contextPanorama` shows the container panorama, which is a different
object. Recorded as design-phase input.

A consequence worth stating so it is not mistaken for sloppiness: **`coldStartCogmap` and
`coldStartContext` are cold in two different senses**, because the two doors read different
things. A cogmap panorama draws *regions*, so its cold case is "zero live regions". A context
panorama draws *goal-rooted containers*, so its cold case is "zero containers" — the context
door cannot see whether the context has regions at all. Both are the real thing a reader
arriving at that door encounters with nothing derived to organize by; neither is a
substitute for the other.
