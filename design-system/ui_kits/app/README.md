# App UI Kit — Temper authed vault browser

Reconstructed from `_source/routes/(app)/+layout.svelte`, `_source/lib/components/Sidebar.svelte`, `VaultGrid.svelte`, `ResourceMetaHeader.svelte`, `FacetChips.svelte`, `CommandPalette.svelte`, `MarkdownRenderer.svelte`.

## Files
- `index.html` — the authed shell. Click a context in the sidebar to browse; click a resource to read it. ⌘K opens the palette.
- `AppShell.jsx` — the two-column flex layout (sidebar + main).
- `Sidebar.jsx` — context list, recent, footer with user.
- `SearchBar.jsx` — the sticky `Search the vault…` header with ⌘K.
- `CommandPalette.jsx` — modal overlay with fuzzy results.
- `VaultGrid.jsx` — resource card grid for a context.
- `ResourceView.jsx` — editorial hero + facet chips + rendered markdown.
- `MarkdownDemo.jsx` — one fake rendered markdown article.

## Coverage
Every screen has one interactive path: **Sidebar → Context → Grid → Resource**. Palette opens via ⌘K or the search bar. All UI is cosmetic — no persistence.
