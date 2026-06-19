<script lang="ts">
  import type { Snippet } from 'svelte';
  import { page } from '$app/state';
  import Footer from '$lib/components/landing/Footer.svelte';
  import { PAGES, INDEX } from './nav';

  let { children }: { children: Snippet } = $props();

  // Flattened reading order: the hub, then the four dimensions (in PAGES
  // order). prev/next walk it so the evaluator can read straight through or
  // jump from the hub's dimension list.
  const ORDER = [{ href: INDEX.href, title: INDEX.title }, ...PAGES];

  // SvelteKit route ids look like "/(public)/operating/..."; strip the route
  // group to match the public hrefs in ORDER.
  const currentHref = $derived.by(() => {
    const id = page.route.id ?? '';
    return id.replace('/(public)', '') || '/operating';
  });

  const currentIndex = $derived(ORDER.findIndex((link) => link.href === currentHref));
  const prev = $derived(currentIndex > 0 ? ORDER[currentIndex - 1] : undefined);
  const next = $derived(
    currentIndex >= 0 && currentIndex < ORDER.length - 1 ? ORDER[currentIndex + 1] : undefined
  );
</script>

{@render children()}

<nav class="operating-nav">
  {#if prev}
    <a class="nav-link prev" href={prev.href}>
      <span class="nav-direction">← Back</span>
      <span class="nav-title">{prev.title}</span>
    </a>
  {:else}
    <span class="nav-spacer"></span>
  {/if}

  {#if next}
    <a class="nav-link next" href={next.href}>
      <span class="nav-direction">Onward →</span>
      <span class="nav-title">{next.title}</span>
    </a>
  {:else}
    <span class="nav-spacer"></span>
  {/if}
</nav>

<Footer />

<style>
  /* Prev/next, mirroring the cognitive-maps tier's footer nav. */
  .operating-nav {
    max-width: 800px;
    margin: 0 auto;
    padding: 3rem 2.5rem 1rem;
    display: flex;
    justify-content: space-between;
    gap: 2rem;
    border-top: 1px solid var(--rule);
  }
  .nav-spacer { flex: 1; }
  .nav-link {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    text-decoration: none;
    color: var(--graphite);
    transition: color 0.2s;
  }
  .nav-link:hover { color: var(--temper-blue); }
  .nav-link.next { text-align: right; align-items: flex-end; }
  .nav-direction {
    font-family: var(--font-mono);
    font-size: 0.65rem;
    letter-spacing: 0.15em;
    text-transform: uppercase;
  }
  .nav-title {
    font-family: var(--font-serif);
    font-size: 1.05rem;
    color: var(--parchment);
  }
  @media (max-width: 640px) {
    .operating-nav { flex-direction: column; padding: 2rem 1.5rem 0.5rem; gap: 1.5rem; }
    .nav-link.next { text-align: left; align-items: flex-start; }
  }

  /* Shared prose styles for /operating pages, mirroring the cognitive-maps
     tier's recipe so the promoted pages keep their exact treatment. Pages wrap
     their main prose region in `<div class="operating-page">` so these globals
     don't bleed onto unrelated routes. */
  :global(.operating-page ul),
  :global(.operating-page ol) {
    font-family: var(--font-serif);
    font-size: 1rem;
    color: var(--chalk);
    line-height: 1.8;
    margin: 0 0 1rem 1.5rem;
    padding: 0;
  }
  :global(.operating-page li) { margin-bottom: 0.4rem; }
  :global(.operating-page li strong) { color: var(--parchment); font-weight: 400; }

  :global(.operating-page blockquote) {
    margin: 1.25rem 0;
    padding: 0.5rem 0 0.5rem 1.25rem;
    border-left: 2px solid var(--temper-blue-border);
    font-family: var(--font-serif);
    color: var(--parchment);
    font-style: italic;
  }
  :global(.operating-page blockquote strong) { font-style: normal; color: var(--parchment); }

  :global(.operating-page code) {
    font-family: var(--font-mono);
    font-size: 0.85rem;
    color: var(--temper-blue);
  }

  :global(.operating-page a) {
    color: var(--temper-blue);
    text-decoration: none;
    border-bottom: 1px solid var(--temper-blue-border-dim);
    transition: border-color 0.2s;
  }
  :global(.operating-page a:hover) { border-bottom-color: var(--temper-blue); }

  :global(.operating-page h3) {
    font-family: var(--font-serif);
    font-size: 1.15rem;
    font-weight: 400;
    color: var(--parchment);
    margin: 1.5rem 0 0.75rem;
  }

  /* Tighter section rhythm for essay-form pages — the hairline divider does the
     section-break work, so vertical space stays minimal. */
  :global(.operating-page .section) {
    padding: 1rem 2.5rem;
  }

  :global(.operating-page table.operating-table) {
    width: 100%;
    border-collapse: collapse;
    margin: 1.5rem 0;
    font-family: var(--font-serif);
    font-size: 0.95rem;
    color: var(--chalk);
  }
  :global(.operating-page table.operating-table th),
  :global(.operating-page table.operating-table td) {
    text-align: left;
    padding: 0.6rem 0.75rem;
    border-bottom: 1px solid var(--rule);
    vertical-align: top;
  }
  :global(.operating-page table.operating-table th) {
    font-family: var(--font-mono);
    font-size: 0.7rem;
    font-weight: 400;
    color: var(--temper-blue);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    border-bottom-color: var(--temper-blue-border-dim);
  }
</style>
