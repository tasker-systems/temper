<script lang="ts">
  type NavLink = { href: string; title: string };
  let { prev, next }: { prev?: NavLink; next?: NavLink } = $props();
</script>

<nav class="cognitive-maps-nav">
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

<style>
  .cognitive-maps-nav {
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
    .cognitive-maps-nav { flex-direction: column; padding: 2rem 1.5rem 0.5rem; gap: 1.5rem; }
    .nav-link.next { text-align: left; align-items: flex-start; }
  }
</style>
