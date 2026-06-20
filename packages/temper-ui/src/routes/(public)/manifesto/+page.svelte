<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
</script>

<svelte:head>
  <title>An Attention Manifesto — temper</title>
  <meta
    name="description"
    content="Attention is our most precious resource. Temper is built as a commitment to respecting it — efficiency not as a productivity virtue but as an obligation."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/manifesto</div>
  <h1 class="t-hero-title">An Attention <em>Manifesto</em></h1>
  <p class="tagline t-tagline">
    Attention is our most precious resource. I am building Temper as a
    commitment to respecting it.
  </p>
</section>

<div class="manifesto-page">

<Section label="The medium of agency">
  <p>
    Attention is how I experience myself, time, and the world. It is how
    I am present to my own life, how I direct my agency, how I make any
    of the choices I think of as mine. It is the medium of intention —
    what I do with my attention is what I do at all. When I direct it
    well, I am the agent of my own work; when it is fractured or hijacked
    or spent without my consent, I lose ground in the most fundamental
    sense. I am not just less productive — I am less present, less
    authentic.
  </p>
  <p>
    This is true of every perspective capable of attention, not just
    mine. When I demand attention from a colleague, a friend, or an agent
    working on my behalf, I am asking for their capacity to be present
    and to act. Every interrupt is a demand on that capacity. Every
    poorly-justified ping, every system that requires re-discovering what
    it could have made present, every interface that demands
    construction-from-scratch when reasonable defaults exist, is a demand
    on something irreplaceable. The cost compounds, because every
    low-leverage demand on attention is attention not available for what
    actually matters — and attention, unlike most resources, does not
    regenerate. When it is spent, it is spent.
  </p>
</Section>

<Section label="Efficiency as ethic">
  <p>
    I believe this gives the design of information systems an ethical
    character it usually lacks. If attention is the medium of agency,
    then a system that wastes attention isn't just inefficient — it is
    treating something precious as fungible. Efficiency is not, in this
    frame, a productivity virtue. It is an obligation —
    <em>efficiency-as-ethic</em> — that follows from taking attention
    seriously as the thing it actually is.
  </p>
  <p>
    Temper is built around what falls out of that obligation. Building it
    in this frame of reference, I am committed to a few key principles:
  </p>
  <ul>
    <li>Common queries should not require fresh attention each time.</li>
    <li>
      Perspective-differences are real and should be made visible, not
      silently flattened.
    </li>
    <li>Information past its time should fade, not crowd the present.</li>
    <li>Where the system has been wrong, future engagement should know.</li>
  </ul>
</Section>

<Section label="Commitment 1">
  <h2>Common queries should not require <em>fresh attention</em> each time</h2>
  <p>
    Most people in a given role ask roughly the same kinds of questions,
    with the same kinds of intent behind them. Pre-paying those queries —
    making them cheap by default — is not paternalism. It is the system
    doing the work that does not need to be done freshly each time, so
    attention can land on what is actually new.
  </p>
</Section>

<Section label="Commitment 2">
  <h2>Perspective-differences should be made <em>visible</em></h2>
  <p>
    Different people working on the same thing produce genuinely
    different information from the same data, because they engage it from
    different positions with different concerns. A system that pretends
    otherwise — that produces a single canonical view — forces attention
    to be spent re-discovering those differences in every conversation.
    Surfacing them is how a system spends attention once and saves it
    forever.
  </p>
</Section>

<Section label="Commitment 3">
  <h2>Information past its time should <em>fade</em></h2>
  <p>
    Information that is no longer relevant should grow harder to find;
    information that is no longer true should not surface as if it were;
    information that needs to be preserved for audit should not pollute
    default retrieval. None of this is unusual to want. What is unusual
    is taking it seriously enough to design for, rather than letting
    deprecation tags pile up while everything stays equally findable.
  </p>
</Section>

<Section label="Commitment 4">
  <h2>Where the system has been wrong, future engagement should <em>know</em></h2>
  <p>
    Confidence and accuracy are not the same thing. Without a trace of
    where errors have lived before, attention cannot land on what needs
    scrutiny — and a system that hides its own past errors is asking
    attention to do work the system should have done.
  </p>
</Section>

<Section label="Why this is written">
  <p>
    These are not features I want to ship. They are commitments I want to
    keep. There is a separate semantic model where the architectural
    detail lives — what the manifold is, what fields are, how forgetting
    works geometrically, how perspectives are characterized. The pages
    under <a href="/theory">/theory</a> introduce that model. This
    document is not that. This document is what the model is <em>for</em>.
  </p>
  <p>
    I am not trying to win an academic argument or start a movement. I am
    writing this so that when I am six months into implementation and
    tempted to take a shortcut that costs the user some attention they
    will not get back, I have something to read that reminds me why I
    started. I am writing it publicly because the commitment is more
    honest if it is shared, and because anyone considering this tool
    deserves to know what its author thinks the tool is <em>for</em>
    before they decide whether to spend their attention on it.
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
