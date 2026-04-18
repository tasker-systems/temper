<script lang="ts">
  import CliBlock from './CliBlock.svelte';
</script>

<section class="hero">
  <h1>Clarify your <em>intention</em></h1>
  <p class="tagline">
    Know what was decided, what's deferred, and what comes next.
    Every session builds on the last.
    <span class="tagline-brand">temper</span> your context.
  </p>
  <div class="hero-ctas">
    <a href="/auth/login" class="primary">Get Started</a>
    <a href="https://github.com/tasker-systems/temper" class="secondary">View on GitHub</a>
  </div>
  <div class="cli-wrapper">
    <CliBlock>
      <div class="cli-prompt">
        <span class="flag">$</span> <span class="cmd">temper search</span> "authentication decisions" <span class="flag">--context backend</span>
      </div>
      <div class="cli-results">
        <div class="cli-result"><span>decision/api-auth-strategy.md</span><span class="cli-score">0.94</span></div>
        <div class="cli-result"><span>research/oauth-provider-comparison.md</span><span class="cli-score">0.87</span></div>
        <div class="cli-result"><span>session/2026-03-28-auth-implementation.md</span><span class="cli-score">0.82</span></div>
      </div>
    </CliBlock>
  </div>
</section>

<style>
  .hero {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    text-align: center;
    padding: 6rem 2.5rem 4rem;
  }

  h1 {
    font-family: var(--font-serif);
    font-size: clamp(2.4rem, 5vw, 3.8rem);
    font-weight: 300;
    line-height: 1.2;
    margin-bottom: 1.5rem;
    letter-spacing: 0.02em;
    color: var(--text);
  }

  h1 em {
    color: var(--blue);
    font-style: italic;
  }

  .tagline {
    font-family: var(--font-serif);
    font-size: 1.1rem;
    color: var(--text-dim);
    font-style: italic;
    max-width: 36em;
    margin-bottom: 3rem;
    line-height: 1.7;
  }

  .tagline-brand {
    color: var(--blue);
    font-style: normal;
    font-family: var(--font-mono);
    font-size: 1rem;
    letter-spacing: 0.05em;
  }

  .hero-ctas {
    display: flex;
    gap: 1rem;
    margin-bottom: 4rem;
  }

  .hero-ctas a {
    font-family: var(--font-mono);
    font-size: 0.8rem;
    padding: 0.6rem 1.5rem;
    text-decoration: none;
    letter-spacing: 0.05em;
    transition: all 0.2s;
  }

  .hero-ctas .primary {
    border: 1px solid var(--blue-border);
    color: var(--blue);
  }

  .hero-ctas .primary:hover {
    background: rgba(126, 184, 218, 0.1);
  }

  .hero-ctas .secondary {
    border: 1px solid rgba(255, 255, 255, 0.12);
    color: var(--text-dim);
  }

  .hero-ctas .secondary:hover {
    border-color: rgba(255, 255, 255, 0.25);
    color: var(--text-mid);
  }

  .cli-wrapper {
    width: 100%;
    max-width: 620px;
  }
</style>
