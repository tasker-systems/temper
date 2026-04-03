<script lang="ts">
  import type { Snippet } from 'svelte';

  let { label, children }: { label: string; children: Snippet } = $props();
</script>

<div class="section-divider"><hr /></div>
<section class="section">
  <div class="section-inner">
    <div class="section-label">{label}</div>
    {@render children()}
  </div>
</section>

<style>
  .section-divider {
    max-width: 800px;
    margin: 0 auto;
    padding: 0 2.5rem;
  }

  .section-divider hr {
    border: none;
    border-top: 1px solid var(--rule);
  }

  .section {
    max-width: 800px;
    margin: 0 auto;
    padding: 5rem 2.5rem;
  }

  .section-inner {
    border-left: 2px solid var(--blue-border);
    padding-left: 2rem;
  }

  .section-label {
    font-family: var(--mono);
    font-size: 0.65rem;
    letter-spacing: 0.2em;
    text-transform: uppercase;
    color: var(--blue);
    margin-bottom: 1.2rem;
  }

  .section :global(h2) {
    font-family: var(--serif);
    font-size: 1.6rem;
    font-weight: 300;
    margin-bottom: 1rem;
    line-height: 1.3;
    color: var(--text);
  }

  .section :global(h2 em) {
    color: var(--blue);
    font-style: italic;
  }

  .section :global(p) {
    font-family: var(--serif);
    font-size: 1rem;
    color: var(--text-mid);
    line-height: 1.8;
    margin-bottom: 1rem;
  }

  .section :global(p strong) {
    color: var(--text);
    font-weight: 400;
  }
</style>
