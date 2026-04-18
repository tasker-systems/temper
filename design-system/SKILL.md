---
name: temper-design
description: Use this skill to generate well-branded interfaces and assets for Temper, either for production or throwaway prototypes/mocks/etc. Contains essential design guidelines, colors, type, fonts, assets, and UI kit components for prototyping.
user-invocable: true
---

Read the `README.md` file within this skill, and explore the other available files.

If creating visual artifacts (slides, mocks, throwaway prototypes, etc), copy assets out and create static HTML files for the user to view. If working on production code, you can copy assets and read the rules here to become an expert in designing with this brand.

If the user invokes this skill without any other guidance, ask them what they want to build or design, ask some questions, and act as an expert designer who outputs HTML artifacts _or_ production code, depending on the need.

## Quick facts

- **Voice:** literate technical; Georgia for reading, JetBrains Mono for doing; one italicized word per heading.
- **Palette:** obsidian `#0a0a0f` ground, `#e8e4df` text, single `#7eb8da` accent. Diagram-only colors live in `colors_and_type.css`.
- **Layout:** single-column, 50rem max, 2px blue left rail on every section body.
- **No icons.** No emoji. The brand mark is the only glyph.
- **Reference UI:** `ui_kits/landing/index.html` and `ui_kits/app/index.html` are working recreations of the two real surfaces.

Always lift tokens from `colors_and_type.css` rather than inventing new ones. When in doubt, open `_source/` and read the production Svelte component.
