# Vault render harness (`/dev/vault`)

A **dev-only** route that renders the artifact surfaces this branch shipped — the REAL
resource-detail page, the real governed-families list, and the real vault filter bar — against
committed, personal-data-free fixtures. **No auth, no server reads, no merge-to-prod** (404s
outside `dev`). Same gap as [`../graph/README.md`](../graph/README.md): previews cannot carry
Auth0, so authenticated UI is otherwise observable only in production, post-merge.

## What to look at

1. **Resource detail, in situ** — the whole real page (body, history, connections) with the Data
   artifacts section below the body, and a trail whose latest events are `data_artifact_committed`
   rows, so the summary lines read where the reader stands. The fixtures cover the whole closed
   vocabulary: current/member/pinned intents, folded (dimmed, still present), all four conformance
   states, a `null`-content artifact, and sizes from bytes to a megabyte.
   - The **toggle** swaps in an empty list: the absence contract is that the section renders
     NOTHING and the page reads exactly as a resource that owns no artifacts always has.
2. **Governed families** — one advisory and one enforcing (amber, v3) family; schema opens on
   click. On the real context page this renders below the list, and renders nothing for a context
   that declares no families.
3. **Ownership filter** — the tri-state `data artifacts` select on the real FilterBar.
   **Look only**: it is wired to the real router, so changing it navigates the real (auth-required)
   app — locally that is a login dead end, by design rather than by accident.

## Running

```bash
cd packages/temper-ui
bun run dev
# http://localhost:5173/dev/vault
```

Fixtures live beside the route in `harness.ts` — typed against the generated wire types, so a
drift between what the API returns and what the harness shows is a type error, not a silent one.
