<script lang="ts">
  let scrolled = $state(false);

  function handleScroll() {
    scrolled = window.scrollY > 40;
  }
</script>

<svelte:window onscroll={handleScroll} />

<nav class="nav" class:scrolled>
  <a href="/" class="nav-logo">
    <svg class="nav-mark" viewBox="0 0 32 32" xmlns="http://www.w3.org/2000/svg">
      <path d="M 12 7 L 12 25" stroke="currentColor" stroke-width="3.5" stroke-linecap="round" fill="none"/>
      <path d="M 6 13 L 18 13 Q 23 13 25 16.5 Q 27 20 25 24" stroke="currentColor" stroke-width="2.8" stroke-linecap="round" fill="none"/>
    </svg>
    <span class="nav-wordmark">temper</span>
  </a>
  <div class="nav-links">
    <a href="/how-it-works">How it works</a>
    <a href="/builders">Builders</a>
    <a href="/agents">Agents</a>
    <a href="/docs" class="cta">Get Started</a>
  </div>
</nav>

<style>
  .nav { position: fixed; top: 0; left: 0; right: 0; z-index: 100; padding: 1.2rem 2.5rem; display: flex; align-items: center; justify-content: space-between; transition: background 0.3s, border-color 0.3s; border-bottom: 1px solid transparent; }
  .nav.scrolled { background: rgba(10, 10, 15, 0.95); border-bottom-color: var(--rule); backdrop-filter: blur(12px); }
  .nav-logo { display: flex; align-items: center; gap: 0.5rem; text-decoration: none; color: var(--blue); transition: opacity 0.2s; }
  .nav-logo:hover { opacity: 0.8; }
  .nav-mark { width: 20px; height: 20px; }
  .nav-wordmark { font-family: var(--mono); font-size: 0.75rem; font-weight: 500; letter-spacing: 0.15em; }
  .nav-links { display: flex; gap: 1.5rem; align-items: center; }
  .nav-links a { font-family: var(--mono); font-size: 0.7rem; color: var(--text-dim); text-decoration: none; letter-spacing: 0.05em; transition: color 0.2s; }
  .nav-links a:hover { color: var(--text); }
  .nav-links .cta { padding: 0.4rem 1rem; border: 1px solid var(--blue-border-dim); color: var(--blue); transition: border-color 0.2s, color 0.2s; }
  .nav-links .cta:hover { border-color: var(--blue); color: var(--text); }
</style>
