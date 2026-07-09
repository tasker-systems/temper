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

Pick a **scenario** (home / nodeNeighborhood / nodeSelected / nodeSelectedContext /
cogmapPanorama / leafBare / regionDrill / regionDrillUnion / contextPanorama /
contextDrill) and a **viewport** preset (or type w/h).
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

Unlike the other eight scenarios, these two were **hand-authored** synthetically
(exactly conforming to `AtlasViewData` / `ContextPanorama` / `AtlasSubgraph`), **not
captured** from prod: the context door predates any deployed instance of it, so there
was nothing live to capture. Regenerate them by editing the committed bundle directly
(the capture console script below has no context-door path).

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

Two steps: **capture** a raw bundle from prod into the local override, then
**sanitize** it into the committed default.

**1. Capture** from the live app's SvelteKit data endpoint (`__data.json`), which carries
the exact page-load output. From a logged-in `temperkb.io/graph/@me` browser tab, paste
this into the devtools console. It auto-derives every id (picks your richest research
cogmap, finds a region with a context-homed composition node, and a low-degree leaf), so
there is nothing to fill in by hand:

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

**2. Sanitize** — move the raw capture into place as the (gitignored) local override,
then generate the committed, personal-data-free default from it:

```bash
mv ~/Downloads/atlas-fixtures.local.json packages/temper-ui/static/dev/atlas-fixtures.local.json
cd packages/temper-ui
node scripts/sanitize-atlas-fixtures.mjs   # → static/dev/atlas-fixtures.json (commit this)
bun run test src/lib/graph/atlas/fixtures.test.ts   # verify the committed bundle is clean
```

The sanitizer remaps every UUID and replaces sensitive free-text (titles, names,
handles, slugs) with deterministic synthetic values while preserving the exact
structure — so the committed bundle stays schema-honest but carries no personal data.
Keep the raw `.local.json` around locally; the loader prefers it when present.
