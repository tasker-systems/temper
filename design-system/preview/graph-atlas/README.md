# Graph Atlas — design targets

Visual targets from the **2026-07-03 graph-visualization rethink** brainstorm. These are
the reference mockups the spec and implementation plan point at — "what we were aiming for."

**Spec:** [`docs/superpowers/specs/2026-07-03-temper-ui-graph-visualization-atlas-design.md`](../../../docs/superpowers/specs/2026-07-03-temper-ui-graph-visualization-atlas-design.md)

Open the `.html` files directly in a browser (they are standalone; they link `../_shared.css`
for fonts/tokens).

## Files

| File | What it shows |
|---|---|
| `visual-direction.html` | The two directions explored for the whole-graph canvas — **A · Constellation** (evolve today's moody dark-editorial serif) vs. **B · Atlas** (cartographic, dense). **Atlas was chosen** for org-scale legibility; Constellation is kept for reference. Both render the same scene (a team's graph, both homes as peers, one node selected with its event-trail rail) so the comparison is about mood + density, not content. |
| `semantic-zoom.html` | The three semantic-zoom tiers — **0 · Panorama** (team-DAG zones + region/context territories + sparsity-fallback salient nodes), **1 · Territory** (clusters + named salient nodes), **2 · Neighborhood** (full force-directed nodes + edges + trail). Each tier is a different bounded read, so payload never scales with org size. |

## Encoding grammar (both mockups)

- **home** — fill (cogmap-homed / authored) vs. outline (context-homed / resource)
- **doc-type** — hue
- **edge kind** — line style; **polarity** — arrowhead; **weight** — thickness
- **`derived_from`** — the dashed cross-home provenance bridge
- **region / team** — hull / territory tint + label
- **history** — the selected element's event trail in the side rail

## Caveat — colors here are a starting point, not the final palette

The specific hues in these mockups (amber = cogmap/concept, blue = context, violet =
question, green = memory) are indicative. Per the spec's palette guidepost, the production
graph palette is a **Chunk-C** design task: a brighter, more expressive, **accessible +
theme-aware** categorical palette where color-and-tone carry information density across all
doc-types, homes, edge-kinds, regions, and salience — distinct from (and more vibrant than)
the understated blues-and-golds of the site chrome. Don't treat these hex values as
canonical.
