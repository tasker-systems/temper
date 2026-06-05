<script lang="ts">
  import type { Snippet } from 'svelte';

  /* Renders a cognitive-maps visualization: an SVG diagram in a framed figure,
     with the source's "Shows" line as the caption and the "Honest basis" schema
     citation kept beneath it — the line that keeps the visual truthful (it
     depicts what the artifact actually does). The markdown source's "For
     successor" guidance is dropped here, since this component is that successor.

     `placement` is HERO for a page's anchor visual (larger frame) or INLINE for
     a supporting one. `diagram` is the SVG snippet; `shows` / `honestBasis`
     carry the verbatim source prose. */

  let {
    placement,
    fidelity,
    diagram,
    shows,
    honestBasis,
    maxWidth,
  }: {
    placement: 'HERO' | 'INLINE';
    /** The source "Fidelity —" value, e.g. `conceptual` or `conceptual / illustrative`. */
    fidelity: string;
    diagram: Snippet;
    shows: Snippet;
    honestBasis: Snippet;
    /** Optional per-figure width override (e.g. "460px") for a sparse panel that
        reads too large at the placement default. */
    maxWidth?: string;
  } = $props();
</script>

<figure
  class="viz-figure"
  class:hero={placement === 'HERO'}
  class:supporting={placement === 'INLINE'}
  style={maxWidth ? `max-width: ${maxWidth}` : undefined}
>
  <div class="viz-canvas">
    {@render diagram()}
  </div>
  <figcaption class="viz-caption">
    <p class="viz-shows">{@render shows()}</p>
    <p class="viz-basis">
      <span class="viz-basis-label">Drawn from</span>
      {@render honestBasis()}
      <span class="viz-fidelity">· {fidelity}</span>
    </p>
  </figcaption>
</figure>

<style>
  .viz-figure {
    max-width: 800px;
    margin: 2.5rem auto;
    background: var(--obsidian-3);
    border: 1px solid var(--rule);
    padding: 1.75rem 1.75rem 1.5rem;
  }
  .viz-figure.hero {
    padding: 2.25rem 2rem 1.75rem;
  }
  /* Supporting (INLINE) figures sit narrower than the page's HERO anchor so
     they read as in-flow detail, not a second hero. NB: the class is
     `supporting`, not `inline` — `.inline` is a Tailwind display utility
     (display:inline) that would override the figure's block layout and break
     max-width. */
  .viz-figure.supporting {
    max-width: 600px;
  }

  .viz-canvas {
    /* The SVG scales to width; this just bounds the hero's vertical presence. */
    margin-bottom: 1.4rem;
  }

  .viz-caption {
    border-top: 1px solid var(--temper-blue-border-dim);
    padding-top: 1rem;
  }
  .viz-shows {
    margin: 0 0 0.6rem;
    font-family: var(--font-serif);
    font-style: italic;
    font-size: 0.92rem;
    line-height: 1.6;
    color: var(--chalk);
  }
  .viz-shows :global(strong) { color: var(--parchment); font-weight: 400; font-style: normal; }
  .viz-shows :global(em) { color: var(--temper-blue); }
  .viz-shows :global(code) { font-family: var(--font-mono); font-size: 0.82em; color: var(--temper-blue); font-style: normal; }

  .viz-basis {
    margin: 0;
    font-family: var(--font-mono);
    font-size: 0.66rem;
    line-height: 1.7;
    letter-spacing: 0.01em;
    color: var(--graphite);
  }
  .viz-basis-label {
    text-transform: uppercase;
    letter-spacing: 0.18em;
    color: var(--temper-blue);
    margin-right: 0.4rem;
  }
  .viz-basis :global(code) {
    font-family: var(--font-mono);
    color: var(--chalk);
  }
  .viz-basis :global(strong) { color: var(--parchment); font-weight: 500; }
  .viz-basis :global(em) { font-style: normal; color: var(--chalk); }
  .viz-fidelity { color: var(--graphite-2); }
</style>
