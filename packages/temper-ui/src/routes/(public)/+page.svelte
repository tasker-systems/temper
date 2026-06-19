<script>
  import Hero from '$lib/components/landing/Hero.svelte';
  import Section from '$lib/components/landing/Section.svelte';
  import Footer from '$lib/components/landing/Footer.svelte';
</script>

<Hero />

<!-- The trunk -->
<Section label="What Temper is">
  <h2>An event-sourced coordination <em>substrate</em></h2>
  <p>
    Temper is a coordination substrate whose organizing purpose is to be
    <strong>economical with attention</strong>. A cognitive map is a
    telos-seeded region of that substrate where humans and agents grow a
    shared, situated understanding together — and everything else, personal
    knowledge management included, is a projection over it.
  </p>
  <p>
    Three ways in, depending on what you came for.
  </p>
</Section>

<!-- The three doors -->
<Section label="Start here">
  <div class="doors">
    <div class="door">
      <span class="door-role">The what</span>
      <a class="door-title" href="/cognitive-maps">
        Cognitive maps<span class="arrow" aria-hidden="true">→</span>
      </a>
      <span class="door-desc">
        The concrete on-ramp: telos-seeded regions, shown proven in the
        schema. Begin the graph-walk here.
      </span>
    </div>

    <div class="door">
      <span class="door-role">Running it</span>
      <a class="door-title" href="/operating">
        Operating<span class="arrow" aria-hidden="true">→</span>
      </a>
      <span class="door-desc">
        For the evaluator: what the architecture fixes, what a deployment
        shapes, and what stays genuinely yours.
      </span>
    </div>

    <div class="door">
      <span class="door-role">The projection</span>
      <span class="door-title door-title--static">Using Temper</span>
      <span class="door-desc">
        One view over the substrate — the personal-knowledge view, for the
        people and the agents who work in it.
      </span>
      <div class="door-links">
        <a href="/builders">For builders <span aria-hidden="true">→</span></a>
        <a href="/agents">For agents <span aria-hidden="true">→</span></a>
      </div>
    </div>
  </div>

  <div class="ascent">
    Looking for the why beneath all three? The commitments the whole system
    answers to live in <a href="/theory">Theory</a>.
  </div>
</Section>

<Footer />

<style>
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
  .ascent { font-family: var(--font-serif); font-size: 0.9rem; color: var(--graphite); line-height: 1.7; margin-top: 2.25rem; }
  .ascent a { color: var(--temper-blue); text-decoration: none; transition: color 0.2s; }
  .ascent a:hover { color: var(--parchment); }
</style>
