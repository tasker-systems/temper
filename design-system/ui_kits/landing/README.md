# Landing UI Kit — Temper marketing site

Reconstructed from `_source/routes/(public)/+page.svelte` and `_source/lib/components/landing/*.svelte`. Renders the marketing home with real components, no backend required.

## Files
- `index.html` — the rendered landing page, loads all components
- `Nav.jsx` — sticky top nav with brand mark + links + CTA
- `Hero.jsx` — headline + deck + CLI block + workflow strip
- `Section.jsx` — the signature blue-rail `<Section label="...">` wrapper
- `CliBlock.jsx` — terminal specimen with commands/flags/output coloring
- `AgentTranscript.jsx` — agent↔user exchange block
- `Workflow.jsx` — the 4-step temper init/add/search/sync strip
- `Concepts.jsx` — the 6-card vocabulary grid (Goals/Tasks/…/Concepts)
- `Footer.jsx` — minimal footer with brand + links
- `Brand.jsx` — the threaded-t SVG + wordmark at three sizes

## Coverage
- Hero, premise, how-it-works, vocabulary, agents, cloud — all five marketing sections render.
- Nav, footer, CLI blocks, agent transcript, concept cards — the full component roster.
- What's omitted: no real routing, no docs page content. Links don't navigate.
