<script lang="ts">
  import type { Snippet } from 'svelte';

  let { children }: { children: Snippet } = $props();
</script>

<div class="cli-block">
  {@render children()}
</div>

<style>
  .cli-block {
    width: 100%;
    background: rgba(255, 255, 255, 0.02);
    border: 1px solid rgba(255, 255, 255, 0.06);
    padding: 1.2rem 1.5rem;
    font-family: var(--font-mono);
    font-size: 0.8rem;
    text-align: left;
  }

  .cli-block :global(.cli-prompt) {
    color: var(--chalk);
    margin-bottom: 0.8rem;
  }

  .cli-block :global(.cmd) {
    color: var(--temper-blue);
  }

  .cli-block :global(.flag) {
    color: rgba(255, 255, 255, 0.3);
  }

  .cli-block :global(.cli-results) {
    font-size: 0.7rem;
    color: var(--graphite);
  }

  .cli-block :global(.cli-result) {
    display: flex;
    justify-content: space-between;
    padding: 0.3rem 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  }

  .cli-block :global(.cli-result:last-child) {
    border-bottom: none;
  }

  .cli-block :global(.cli-score) {
    color: var(--temper-blue-dim);
  }
</style>
