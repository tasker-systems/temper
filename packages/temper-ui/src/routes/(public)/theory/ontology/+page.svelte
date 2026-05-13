<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
</script>

<svelte:head>
  <title>Ontology — temper</title>
  <meta
    name="description"
    content="The four ontological layers: data, intention, information, knowledge. Each derivable from the layers below given the appropriate inputs."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/theory/ontology</div>
  <h1 class="t-hero-title">Ontology: data, intention, information, <em>knowledge</em></h1>
  <p class="tagline t-tagline">
    The model has four ontological layers, each derivable from the
    layers below given the appropriate inputs.
  </p>
</section>

<div class="theory-page">

<Section label="Data">
  <p>
    Data is the substrate — events, recorded, immutable, append-only.
    Not objective in the view-from-nowhere sense: every event has an
    emitter, and the substrate is the perspective-laden record of past
    emissions. The objectivity that's available is weaker and more
    honest — any qualified observer can in principle re-engage the same
    recorded events, but they're never engaging neutral data.
  </p>
</Section>

<Section label="Intention">
  <p>
    Intention is a vector originating at a perspective-point. It has
    direction (what it is about), magnitude (how strongly it points),
    and origin (whose perspective). It has no view from nowhere; every
    intention-vector starts at a located perspective.
  </p>
</Section>

<Section label="Information">
  <p>
    Information is what happens when intention-vectors interact with
    the data substrate. The same data engaged by different intentions
    produces different information. This is the layer where <em>what
    something means</em> lives.
  </p>
</Section>

<Section label="Knowledge">
  <p>
    Knowledge is the relationship between a perspective and the
    information it has produced through intentional engagement with
    data. Knowledge is not a stored thing; it is a relationship state.
  </p>
  <p>Stated formally:</p>
  <blockquote>
    <strong>K = experience(P, I)</strong> where <strong>I = project(Q, F | P)</strong>
  </blockquote>
  <p>
    A perspective P (itself a function of trajectory through time)
    formulates an intention-query Q which projects the data field F
    into an information shape I, and knowledge is the experiential
    engagement of that perspective with that information shape.
  </p>
</Section>

<Section label="Misnomer">
  <h2>Knowledge bases are <em>misnomers</em></h2>
  <p>
    A consequence worth stating up front: <strong>knowledge bases are
    misnomers.</strong> The system stores data and traces of past
    intentional acts (recorded questions, notes, decisions, which
    themselves become further data). It does not store knowledge. It
    enables knowledge to be <em>produced</em> when engaged from a
    particular perspective. The "knowledge" of a knowledge base is
    always potential, never actual, until activated by a perspective.
  </p>
  <p>
    The system's job is never to <em>be right about what something
    means</em> — meaning is not in the system. Its job is to
    faithfully represent data, faithfully record intentions, and
    faithfully compute projections such that perspectives engaging
    with those projections are well-equipped to produce knowledge.
  </p>
  <p>
    None of the pages that follow can claim the system <em>knows</em>.
    They describe the conditions under which knowledge is produced.
  </p>
</Section>

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
</style>
