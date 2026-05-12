<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
</script>

<svelte:head>
  <title>Translation — temper</title>
  <meta
    name="description"
    content="Translation between perspectives is not a resolution problem; it is a first-class activity the substrate has to support."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/theory/translation</div>
  <h1 class="t-hero-title"><em>Translation</em></h1>
  <p class="tagline t-tagline">
    Translation between perspectives is not a resolution problem; it is a
    first-class activity the substrate has to support.
  </p>
</section>

<div class="theory-page">

<Section label="Composition">
  <h2>Intention-vectors <em>compose and conflict</em></h2>
  <p>
    Multiple perspectives exerting intentions over the same data region
    produce a vector sum at that region. Aligned intentions produce
    coherent information. Conflicting intentions produce stress in the
    field. The model can in principle distinguish productive
    disagreement (vectors at angles, both contributing magnitude) from
    unresolved disagreement (vectors pointing opposite, summing to
    near-zero with high latent tension).
  </p>
</Section>

<Section label="The problem">
  <p>
    Multiple perspectives engaging the same data field with aligned
    good-faith intentions still produce different information shapes —
    different terms, different weights, different salience structures —
    because each information shape is grounded in a particular
    perspective-vector and there is no neutral ground onto which to
    project them.
  </p>
  <p>
    The ops head's "feature" and the PM's "feature" and the tech lead's
    "feature" are not different names for the same information; they are
    <em>different information</em>, even when they point at substantially
    the same data.
  </p>
  <p>
    This rules out a class of design ambitions knowledge systems
    regularly attempt and fail at: producing a canonical, neutral,
    role-independent view. There is no such view to produce. A system
    that tries to merge perspectives into one frame either silently
    privileges one perspective as canonical, or produces a flattened
    average none of the perspectives actually inhabits. Both are
    translation failures masquerading as resolutions.
  </p>
  <p>
    What the system can do — and what attention-economy demands it do —
    is <strong>make perspective-differences visible without claiming to
    have resolved them</strong>. Rather than producing a neutral view,
    the system can show the same region's information shape from
    multiple perspectives, where the shapes overlap, and where they
    diverge. Translation becomes a first-class activity supported by
    the system, not an outcome it pretends to deliver.
  </p>
  <p>
    A further depth in the irreducibility is worth naming. Each
    perspective's information shape is itself a contingent
    stabilization — the emission a perspective makes is the
    outer-boundary representation of an internal play of forces, not a
    transparent window into a univocal underlying intent. Translation
    is therefore not between two clear views of the same data; it is
    between two boundary-emissions, each of which is already a
    stabilization of internal flux on its side. The model is not just
    acknowledging that perspectives don't converge with each other;
    it's acknowledging that even within a single perspective there is
    no convergence to be reported.
  </p>
</Section>

<Section label="Bridges">
  <h2>Translation events, bridges, and <em>translation-scars</em></h2>
  <p>
    When two perspectives bring their information shapes into dialogue
    and arrive at shared understanding — or fail to — that is an event
    with the same status as a decision or a research note. The trace of
    translation work over time accumulates into a <strong>bridging
    structure</strong> over the base topology, distinct from but related
    to it. Successful translations leave bridges; failed translations
    leave scars <em>at the translation point</em> rather than at the
    underlying data. Both belong in the substrate.
  </p>
</Section>

<Section label="Knowledge as relationship">
  <p>
    Knowledge is the experiential outcome when a perspective engages a
    projection of the represented state. It is a relationship, not a
    thing. The system stores conditions for knowledge production, not
    knowledge itself. <a href="/theory/ontology">The data / information
    / knowledge stratification</a> is the right vocabulary for naming
    the layers of the phenomenon, but the system's representational
    scope ends at projection — knowledge is what happens when a
    perspective engages with a projection.
  </p>
  <p>
    This is the closing commitment of the model, and the place where
    the model's discipline is most visible: it refuses to claim that
    any of what it represents is the thing that matters. What matters
    is what perspectives do with it.
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
