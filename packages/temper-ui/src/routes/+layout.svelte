<script>
  import '../app.css';
  import Nav from '$lib/components/landing/Nav.svelte';

  let { children } = $props();
</script>

<svelte:head>
  <title>temper — clarify your intention</title>
  <meta name="description" content="CLI-first knowledge base with semantic search, frontmatter-driven structure, and cloud sync." />
</svelte:head>

<div class="app">
  <Nav />
  <main>
    {@render children()}
  </main>
</div>

<style>
  .app {
    min-height: 100vh;
    background: var(--bg);
    color: var(--text);
    font-family: var(--serif);
    line-height: 1.7;
    -webkit-font-smoothing: antialiased;
  }
</style>
