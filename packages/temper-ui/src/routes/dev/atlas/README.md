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

Pick a **scenario** (home / nodeNeighborhood / nodeSelected / cogmapPanorama /
leafBare / regionDrill / regionDrillUnion) and a **viewport** preset (or type w/h).
The frame clips like a real bounded viewport and is drag-resizable from its corner.
On a fresh checkout the harness runs against the committed synthetic fixtures — no
local capture required.

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
the exact page-load output. From a logged-in `temperkb.io/graph/@me` browser tab,
in the devtools console:

```js
// devalue unflatten (SvelteKit __data.json is flattened)
const unflatten = (values) => {
  const hydrated = new Array(values.length), seen = new Array(values.length).fill(false);
  const h = (i) => {
    if (i === -1) return undefined; if (i === -3) return NaN;
    if (i === -4) return Infinity; if (i === -5) return -Infinity; if (i === -6) return -0;
    if (i === -2) return undefined; if (seen[i]) return hydrated[i]; seen[i] = true;
    const v = values[i];
    if (v === null || typeof v !== 'object') { hydrated[i] = v; return v; }
    if (Array.isArray(v)) {
      if (typeof v[0] === 'string') { hydrated[i] = v; return v; } // type tag — pass through
      const a = []; hydrated[i] = a; for (const j of v) a.push(h(j)); return a;
    }
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

const COGMAP = '<your cogmap id>';    // from grab('') → .home.research[].id
const REGION = '<a region territory id>'; // from grab('cogmap='+COGMAP) → .territories.territories[] where kind==='region'
const REGION2 = '<a second region id>';   // another region in the same cogmap (for the union)
const NODE = '<a node id>';           // from a composition drill → .neighborhood.nodes[].id

const LEAF = '<a neighbor-less leaf node id>'; // a node whose neighborhood is empty

// Beat D: a territory focus is the region → resources COMPOSITION drill (facets +
// the context-resources they were derived_from); a `~`-join unions regions.
const bundle = {
  _meta: { captured_from: 'temperkb.io/graph/@me', note: 'full PageData per scenario' },
  home: await grab(''),
  cogmapPanorama: await grab('cogmap=' + COGMAP),
  regionDrill: await grab('cogmap=' + COGMAP + '&focus=territory:' + REGION),
  regionDrillUnion: await grab('cogmap=' + COGMAP + '&focus=territory:' + REGION + '~' + REGION2),
  nodeNeighborhood: await grab('cogmap=' + COGMAP + '&focus=node:' + NODE),
  nodeSelected: await grab('cogmap=' + COGMAP + '&focus=node:' + NODE + '&sel=node:' + NODE),
  leafBare: await grab('cogmap=' + COGMAP + '&focus=node:' + LEAF + '&sel=node:' + LEAF)
};
const a = document.createElement('a');
a.href = URL.createObjectURL(new Blob([JSON.stringify(bundle)], { type: 'application/json' }));
a.download = 'atlas-fixtures.local.json';
a.click();
```

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
