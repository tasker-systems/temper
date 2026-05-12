<script lang="ts">
  import type { Snippet } from 'svelte';
  import { page } from '$app/state';
  import Footer from '$lib/components/landing/Footer.svelte';
  import TheoryNav from './TheoryNav.svelte';

  let { children }: { children: Snippet } = $props();

  type NavLink = { href: string; title: string };

  const ORDER: NavLink[] = [
    { href: '/theory',                title: 'What Temper is building toward' },
    { href: '/theory/ontology',       title: 'Ontology' },
    { href: '/theory/manifold',       title: 'The manifold' },
    { href: '/theory/time',           title: 'Time' },
    { href: '/theory/deformation',    title: 'Deformation' },
    { href: '/theory/perspectives',   title: 'Perspectives' },
    { href: '/theory/translation',    title: 'Translation' },
    { href: '/theory/schema',         title: 'Schema' },
    { href: '/theory/open-questions', title: 'Open questions' },
  ];

  // Derive current index from the route. SvelteKit route ids look like
  // "/(public)/theory" or "/(public)/theory/ontology"; strip the group.
  const currentHref = $derived.by(() => {
    const id = page.route.id ?? '';
    return id.replace('/(public)', '') || '/theory';
  });

  const currentIndex = $derived(ORDER.findIndex((link) => link.href === currentHref));
  const isEntry = $derived(currentHref === '/theory');
  const prev = $derived(currentIndex > 0 ? ORDER[currentIndex - 1] : undefined);
  const next = $derived(
    currentIndex >= 0 && currentIndex < ORDER.length - 1 ? ORDER[currentIndex + 1] : undefined
  );
</script>

{#if !isEntry}
  <div class="theory-backlink">
    <a href="/theory">← Theory</a>
  </div>
{/if}

{@render children()}

{#if !isEntry}
  <TheoryNav {prev} {next} />
{/if}

<Footer />

<style>
  .theory-backlink {
    max-width: 800px;
    margin: 0 auto;
    padding: 2.5rem 2.5rem 0;
  }
  .theory-backlink a {
    font-family: var(--font-mono);
    font-size: 0.7rem;
    letter-spacing: 0.12em;
    color: var(--graphite);
    text-decoration: none;
    text-transform: uppercase;
    transition: color 0.2s;
  }
  .theory-backlink a:hover { color: var(--temper-blue); }

  /* Shared prose styles for theory pages.
     Pages wrap their main prose region in `<div class="theory-page">` so
     these globals don't bleed onto unrelated routes. */
  :global(.theory-page ul),
  :global(.theory-page ol) {
    font-family: var(--font-serif);
    font-size: 1rem;
    color: var(--chalk);
    line-height: 1.8;
    margin: 0 0 1rem 1.5rem;
    padding: 0;
  }
  :global(.theory-page li) { margin-bottom: 0.4rem; }
  :global(.theory-page li strong) { color: var(--parchment); font-weight: 400; }

  :global(.theory-page blockquote) {
    margin: 1.25rem 0;
    padding: 0.5rem 0 0.5rem 1.25rem;
    border-left: 2px solid var(--temper-blue-border);
    font-family: var(--font-serif);
    color: var(--parchment);
    font-style: italic;
  }
  :global(.theory-page blockquote strong) { font-style: normal; color: var(--parchment); }

  :global(.theory-page code) {
    font-family: var(--font-mono);
    font-size: 0.85rem;
    color: var(--temper-blue);
  }

  :global(.theory-page a) {
    color: var(--temper-blue);
    text-decoration: none;
    border-bottom: 1px solid var(--temper-blue-border-dim);
    transition: border-color 0.2s;
  }
  :global(.theory-page a:hover) { border-bottom-color: var(--temper-blue); }

  :global(.theory-page h3) {
    font-family: var(--font-serif);
    font-size: 1.15rem;
    font-weight: 400;
    color: var(--parchment);
    margin: 1.5rem 0 0.75rem;
  }

  :global(.theory-page table.theory-table) {
    width: 100%;
    border-collapse: collapse;
    margin: 1.5rem 0;
    font-family: var(--font-serif);
    font-size: 0.95rem;
    color: var(--chalk);
  }
  :global(.theory-page table.theory-table th),
  :global(.theory-page table.theory-table td) {
    text-align: left;
    padding: 0.6rem 0.75rem;
    border-bottom: 1px solid var(--rule);
    vertical-align: top;
  }
  :global(.theory-page table.theory-table th) {
    font-family: var(--font-mono);
    font-size: 0.7rem;
    font-weight: 400;
    color: var(--temper-blue);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    border-bottom-color: var(--temper-blue-border-dim);
  }
</style>
