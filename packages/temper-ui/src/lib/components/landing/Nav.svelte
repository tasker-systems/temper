<script lang="ts">
  let scrolled = $state(false);
  let usingOpen = $state(false);

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
    <a href="/cognitive-maps">Cognitive maps</a>
    <a href="/operating">Operating</a>
    <div
      class="nav-group"
      onmouseenter={() => (usingOpen = true)}
      onmouseleave={() => (usingOpen = false)}
      onfocusin={() => (usingOpen = true)}
      onfocusout={() => (usingOpen = false)}
      role="none"
    >
      <button
        type="button"
        class="nav-group-trigger"
        aria-haspopup="true"
        aria-expanded={usingOpen}
        onclick={() => (usingOpen = !usingOpen)}
      >
        Using Temper<span class="nav-caret" aria-hidden="true">▾</span>
      </button>
      <div class="nav-menu" class:open={usingOpen}>
        <a href="/builders">Builders</a>
        <a href="/agents">Agents</a>
        <a href="/using-temper">Reference</a>
      </div>
    </div>
    <a href="/theory">Theory</a>
    <a href="/auth/login" class="cta">Get Started</a>
  </div>
</nav>

<style>
  .nav { position: fixed; top: 0; left: 0; right: 0; z-index: 100; padding: 1.2rem 2.5rem; display: flex; align-items: center; justify-content: space-between; transition: background 0.3s, border-color 0.3s; border-bottom: 1px solid transparent; }
  .nav.scrolled { background: rgba(10, 10, 15, 0.95); border-bottom-color: var(--rule); backdrop-filter: blur(12px); }
  .nav-logo { display: flex; align-items: center; gap: 0.5rem; text-decoration: none; color: var(--temper-blue); transition: opacity 0.2s; }
  .nav-logo:hover { opacity: 0.8; }
  .nav-mark { width: 20px; height: 20px; }
  .nav-wordmark { font-family: var(--font-mono); font-size: 0.75rem; font-weight: 500; letter-spacing: 0.15em; }
  .nav-links { display: flex; gap: 1.5rem; align-items: center; }
  .nav-links > a { font-family: var(--font-mono); font-size: 0.7rem; color: var(--graphite); text-decoration: none; letter-spacing: 0.05em; transition: color 0.2s; }
  .nav-links > a:hover { color: var(--parchment); }
  .nav-links .cta { padding: 0.4rem 1rem; border: 1px solid var(--temper-blue-border-dim); color: var(--temper-blue); transition: border-color 0.2s, color 0.2s; }
  .nav-links .cta:hover { border-color: var(--temper-blue); color: var(--parchment); }

  .nav-group { position: relative; display: flex; align-items: center; }
  .nav-group::after { content: ''; position: absolute; top: 100%; left: 0; right: 0; height: 0.85rem; }
  .nav-group-trigger { font-family: var(--font-mono); font-size: 0.7rem; color: var(--graphite); background: none; border: none; padding: 0; margin: 0; cursor: pointer; letter-spacing: 0.05em; display: inline-flex; align-items: center; gap: 0.3rem; transition: color 0.2s; }
  .nav-group-trigger:hover, .nav-group:focus-within .nav-group-trigger { color: var(--parchment); }
  .nav-caret { font-size: 0.6rem; transition: transform 0.2s; }
  .nav-group:hover .nav-caret, .nav-group:focus-within .nav-caret { transform: rotate(180deg); }
  .nav-menu { position: absolute; top: 100%; left: 0; margin-top: 0.85rem; display: flex; flex-direction: column; gap: 0.85rem; min-width: 9rem; padding: 0.95rem 1.05rem; background: rgba(10, 10, 15, 0.97); border: 1px solid var(--rule); backdrop-filter: blur(12px); opacity: 0; visibility: hidden; transform: translateY(-4px); transition: opacity 0.2s, transform 0.2s, visibility 0.2s; }
  .nav-menu.open { opacity: 1; visibility: visible; transform: translateY(0); }
  .nav-menu a { font-family: var(--font-mono); font-size: 0.7rem; color: var(--graphite); text-decoration: none; letter-spacing: 0.05em; transition: color 0.2s; }
  .nav-menu a:hover { color: var(--parchment); }
</style>
