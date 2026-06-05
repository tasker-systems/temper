<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
  import ConceptGraph from '$lib/components/cognitive-maps/ConceptGraph.svelte';
</script>

<svelte:head>
  <title>Cognitive maps — temper</title>
  <meta
    name="description"
    content="A cognitive map is a telos-seeded region of an event-sourced substrate where humans and agents grow a shared, situated understanding together. The shape proven by the data model — and the operating questions still open."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/cognitive-maps</div>
  <h1 class="t-hero-title">A region where understanding <em>grows</em></h1>
  <p class="tagline t-tagline">
    The shape proven by the data model, and the operating questions still
    open — shown from one running example.
  </p>
</section>

<div class="cognitive-maps-page">

<Section label="What it's for">
  <p>
    Our work on <a href="/theory">theory</a> ends on a hard claim —
    <em>the system does not store knowledge</em>; it stores data and the traces
    of acts, and computes projections so that a perspective engaging them can
    <em>produce</em> knowledge. A cognitive map is where that production is given
    a home and a direction. Compressed to a sentence:
  </p>
  <blockquote>
    Temper is an event-sourced coordination substrate whose organizing purpose
    is to be economical with attention. A cognitive map is a telos-seeded region
    of that substrate where humans and agents grow a shared, situated
    understanding together — and everything else is a projection over it.
  </blockquote>
  <p>The rest of this set is that sentence, working.</p>
</Section>

</div>

<div class="graph-region">
  <p class="graph-lead t-strip">Eleven pages, one running example — held together like this</p>
  <ConceptGraph />
</div>

<style>
  .hero {
    min-height: 50vh;
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    text-align: center;
    padding: 5rem 2.5rem 2rem;
  }
  .hero-label { margin-bottom: 1.5rem; }
  .hero h1 { margin-bottom: 1.5rem; }
  .tagline { max-width: 36em; }

  .graph-region {
    max-width: 800px;
    margin: 2rem auto 0;
    padding: 0 2.5rem;
  }
  .graph-lead {
    text-align: center;
    margin: 0 0 0.5rem;
  }
</style>
