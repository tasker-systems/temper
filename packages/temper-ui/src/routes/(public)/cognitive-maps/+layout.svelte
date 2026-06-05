<script lang="ts">
  import type { Snippet } from 'svelte';
  import { page } from '$app/state';
  import Footer from '$lib/components/landing/Footer.svelte';
  import CognitiveMapsNav from './CognitiveMapsNav.svelte';
  import { PAGES, INDEX, OVERVIEW } from './nav';

  let { children }: { children: Snippet } = $props();

  // Flattened reading order: index, the visual fly-over, then the show pages,
  // the operating-temper hub, and its children (PAGES is already in this
  // order). prev/next walk it.
  const ORDER = [
    { href: INDEX.href, title: INDEX.title },
    { href: OVERVIEW.href, title: OVERVIEW.title },
    ...PAGES,
  ];

  // SvelteKit route ids look like "/(public)/cognitive-maps/..."; strip the
  // route group to match the public hrefs in ORDER.
  const currentHref = $derived.by(() => {
    const id = page.route.id ?? '';
    return id.replace('/(public)', '') || '/cognitive-maps';
  });

  const currentIndex = $derived(ORDER.findIndex((link) => link.href === currentHref));
  const prev = $derived(currentIndex > 0 ? ORDER[currentIndex - 1] : undefined);
  const next = $derived(
    currentIndex >= 0 && currentIndex < ORDER.length - 1 ? ORDER[currentIndex + 1] : undefined
  );
</script>

{@render children()}

<CognitiveMapsNav {prev} {next} />

<Footer />

<style>
  /* Shared prose styles for cognitive-maps pages, mirroring the `/theory`
     tier's recipe. Pages wrap their main prose region in
     `<div class="cognitive-maps-page">` so these globals don't bleed onto
     unrelated routes. */
  :global(.cognitive-maps-page ul),
  :global(.cognitive-maps-page ol) {
    font-family: var(--font-serif);
    font-size: 1rem;
    color: var(--chalk);
    line-height: 1.8;
    margin: 0 0 1rem 1.5rem;
    padding: 0;
  }
  :global(.cognitive-maps-page li) { margin-bottom: 0.4rem; }
  :global(.cognitive-maps-page li strong) { color: var(--parchment); font-weight: 400; }

  :global(.cognitive-maps-page blockquote) {
    margin: 1.25rem 0;
    padding: 0.5rem 0 0.5rem 1.25rem;
    border-left: 2px solid var(--temper-blue-border);
    font-family: var(--font-serif);
    color: var(--parchment);
    font-style: italic;
  }
  :global(.cognitive-maps-page blockquote strong) { font-style: normal; color: var(--parchment); }

  :global(.cognitive-maps-page code) {
    font-family: var(--font-mono);
    font-size: 0.85rem;
    color: var(--temper-blue);
  }

  :global(.cognitive-maps-page a) {
    color: var(--temper-blue);
    text-decoration: none;
    border-bottom: 1px solid var(--temper-blue-border-dim);
    transition: border-color 0.2s;
  }
  :global(.cognitive-maps-page a:hover) { border-bottom-color: var(--temper-blue); }

  :global(.cognitive-maps-page h3) {
    font-family: var(--font-serif);
    font-size: 1.15rem;
    font-weight: 400;
    color: var(--parchment);
    margin: 1.5rem 0 0.75rem;
  }

  /* Tighter section rhythm for essay-form pages — the hairline divider does
     the section-break work, so vertical space stays minimal. */
  :global(.cognitive-maps-page .section) {
    padding: 1rem 2.5rem;
  }

  :global(.cognitive-maps-page table.cognitive-maps-table) {
    width: 100%;
    border-collapse: collapse;
    margin: 1.5rem 0;
    font-family: var(--font-serif);
    font-size: 0.95rem;
    color: var(--chalk);
  }
  :global(.cognitive-maps-page table.cognitive-maps-table th),
  :global(.cognitive-maps-page table.cognitive-maps-table td) {
    text-align: left;
    padding: 0.6rem 0.75rem;
    border-bottom: 1px solid var(--rule);
    vertical-align: top;
  }
  :global(.cognitive-maps-page table.cognitive-maps-table th) {
    font-family: var(--font-mono);
    font-size: 0.7rem;
    font-weight: 400;
    color: var(--temper-blue);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    border-bottom-color: var(--temper-blue-border-dim);
  }
</style>
