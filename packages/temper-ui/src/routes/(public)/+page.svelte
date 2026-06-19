<script>
  import Hero from '$lib/components/landing/Hero.svelte';
  import Section from '$lib/components/landing/Section.svelte';
  import Footer from '$lib/components/landing/Footer.svelte';
  import LedgerProjectionSpiral from '$lib/components/cognitive-maps/diagrams/LedgerProjectionSpiral.svelte';
</script>

<Hero />

<!-- What Temper is — shown first, then named -->
<Section label="What Temper is">
  <LedgerProjectionSpiral id="landing-ledger" />
  <p class="figure-frame">
    An append-only event ledger is the source of truth. Everything above it —
    the graph, the regions, the personal-knowledge view — is a projection
    materialized at read time. A <strong>cognitive map</strong> is a
    telos-seeded region of that substrate where people and agents grow a shared,
    situated understanding together. Press <em>Replay</em> to watch one rise as
    its ledger grows.
  </p>
</Section>

<!-- Start here — three doors mirroring the nav spine -->
<Section label="Start here">
  <div class="doors">
    <div class="door">
      <span class="door-role">What it is</span>
      <a class="door-title" href="/cognitive-maps">
        Cognitive maps<span class="arrow" aria-hidden="true">→</span>
      </a>
      <span class="door-desc">
        The concrete on-ramp — telos-seeded regions where a shared
        understanding grows, shown proven in the schema. Begin the graph-walk
        here.
      </span>
      <div class="door-links">
        <a href="/manifesto">Attention manifesto <span aria-hidden="true">→</span></a>
        <a href="/theory">Theory <span aria-hidden="true">→</span></a>
      </div>
    </div>

    <div class="door">
      <span class="door-role">How to use it</span>
      <span class="door-title door-title--static">Using Temper</span>
      <span class="door-desc">
        Put the substrate to work — as a builder, an agent, or an operator.
      </span>
      <div class="door-links">
        <a href="/builders">Builders <span aria-hidden="true">→</span></a>
        <a href="/agents">Agents <span aria-hidden="true">→</span></a>
        <a href="/operating">Operating <span aria-hidden="true">→</span></a>
        <a href="/using-temper">Reference <span aria-hidden="true">→</span></a>
      </div>
    </div>

    <div class="door">
      <span class="door-role">Get started</span>
      <a class="door-title" href="/auth/login">
        Start using it<span class="arrow" aria-hidden="true">→</span>
      </a>
      <span class="door-desc">
        Sign up, or read the code.
      </span>
      <div class="door-links">
        <a href="/auth/login">Get Started <span aria-hidden="true">→</span></a>
        <a href="https://github.com/tasker-systems/temper" target="_blank" rel="noopener">GitHub <span aria-hidden="true">→</span></a>
      </div>
    </div>
  </div>
</Section>

<Footer />

<style>
  .figure-frame { font-family: var(--font-serif); font-size: 0.95rem; color: var(--chalk); line-height: 1.8; margin-top: 1.5rem; max-width: 42em; }
  .figure-frame strong { color: var(--parchment); font-weight: 400; }
  .figure-frame em { color: var(--temper-blue); font-style: normal; }
  .doors { display: flex; flex-direction: column; gap: 1.25rem; margin-top: 1.5rem; }
  .door { border: 1px solid rgba(255, 255, 255, 0.08); padding: 1.5rem; transition: border-color 0.2s; }
  .door:hover { border-color: var(--temper-blue-border-dim); }
  .door-role { display: block; font-family: var(--font-mono); font-size: 0.62rem; letter-spacing: 0.2em; text-transform: uppercase; color: var(--graphite); margin-bottom: 0.7rem; }
  .door-title { display: inline-flex; align-items: center; gap: 0.5rem; font-family: var(--font-serif); font-size: 1.3rem; font-weight: 300; color: var(--parchment); text-decoration: none; transition: color 0.2s; }
  a.door-title:hover { color: var(--temper-blue); }
  .door-title--static { color: var(--parchment); }
  .door-title .arrow { font-family: var(--font-mono); font-size: 0.9rem; color: var(--temper-blue); transition: transform 0.2s; }
  .door:hover .arrow { transform: translateX(3px); }
  .door-desc { display: block; font-family: var(--font-serif); font-size: 0.95rem; color: var(--chalk); line-height: 1.7; margin-top: 0.6rem; }
  .door-links { display: flex; flex-wrap: wrap; gap: 1.5rem; margin-top: 1rem; }
  .door-links a { font-family: var(--font-mono); font-size: 0.72rem; color: var(--temper-blue); text-decoration: none; letter-spacing: 0.03em; transition: color 0.2s; }
  .door-links a:hover { color: var(--parchment); }
</style>
