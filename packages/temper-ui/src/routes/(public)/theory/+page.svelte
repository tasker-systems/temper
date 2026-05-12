<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
</script>

<svelte:head>
  <title>What Temper is building toward — temper</title>
  <meta
    name="description"
    content="Attention is the teleological anchor. Temper is built as a commitment to respecting it. An introduction to the theory tier."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/theory</div>
  <h1 class="t-hero-title">What Temper is building <em>toward</em></h1>
  <p class="tagline t-tagline">
    Attention is our most precious resource. I am building Temper as a
    commitment to respecting it.
  </p>
</section>

<div class="theory-page">

<Section label="The anchor">
  <h2>Attention is the <em>teleological anchor</em></h2>
  <p>
    Attention is how I experience myself, time, and the world. It is how
    I am present to my own life, how I direct my agency, how I make any
    of the choices I think of as mine. It is the medium of intention —
    what I do with my attention is what I do at all. When I direct it
    well, I am the agent of my own work; when it is fractured or
    hijacked or spent without my consent, I lose ground in the most
    fundamental sense.
  </p>
  <p>
    This is true of every perspective capable of attention, not just
    mine. When I demand attention from a colleague, a friend, or an
    agent working on my behalf, I am asking for their capacity to be
    present and to act. Every interrupt is a demand on that capacity.
    Every poorly-justified ping, every system that requires
    re-discovering what it could have made present, every interface
    that demands construction-from-scratch when reasonable defaults
    exist, is a demand on something irreplaceable. The cost compounds,
    because every low-leverage demand on attention is attention not
    available for what actually matters — and attention, unlike most
    resources, does not regenerate. When it is spent, it is spent.
  </p>
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
    Temper is built around what falls out of that obligation. Building
    it in this frame of reference, I am committed to a few key
    principles.
  </p>
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
  <h2>Perspective-differences are real and should be made <em>visible</em></h2>
  <p>
    Different people working on the same thing produce genuinely
    different information from the same data, because they engage it
    from different positions with different concerns. A system that
    pretends otherwise — that produces a single canonical view —
    forces attention to be spent re-discovering those differences in
    every conversation. Surfacing them is how a system spends attention
    once and saves it forever.
  </p>
</Section>

<Section label="Commitment 3">
  <h2>Information past its time should <em>fade</em>, not crowd the present</h2>
  <p>
    Information that is no longer relevant should grow harder to find;
    information that is no longer true should not surface as if it
    were; information that needs to be preserved for audit should not
    pollute default retrieval. None of this is unusual to want. What
    is unusual is taking it seriously enough to design for, rather
    than letting deprecation tags pile up while everything stays
    equally findable.
  </p>
</Section>

<Section label="Commitment 4">
  <h2>Where the system has been wrong, future engagement should <em>know</em></h2>
  <p>
    Confidence and accuracy are not the same thing. Without a trace of
    where errors have lived before, attention cannot land on what
    needs scrutiny — and a system that hides its own past errors is
    asking attention to do work the system should have done.
  </p>
</Section>

<Section label="One commitment up front">
  <h2>The system does not store <em>knowledge</em></h2>
  <p>
    These are not features I want to ship. They are commitments I want
    to keep. There is a separate semantic model where the architectural
    detail lives — what the manifold is, what fields are, how
    forgetting works geometrically, how perspectives are
    characterized. The pages under <code>/theory</code> introduce that
    model.
  </p>
  <p>
    One commitment in the model runs through everything that follows,
    and deserves naming up front: the system stores data and traces of
    past intentional acts — recorded questions, notes, decisions,
    which themselves become further data. It does not store
    <em>knowledge</em>. Knowledge is the relationship between a
    perspective and the information that perspective produces through
    engagement with data.
  </p>
  <p>
    The label "knowledge base" is a misnomer in light of this.
    Knowledge is always potential, never actual, until activated by a
    perspective. The system's job is never to <em>be right about what
    something means</em> — meaning is not in the system. Its job is to
    faithfully represent data, faithfully record intentions, and
    faithfully compute projections such that perspectives engaging
    with those projections are well-equipped to produce knowledge.
  </p>
</Section>

<Section label="The model">
  <h2>The shape of the <em>model</em></h2>
  <p>
    The pages here introduce the model in the order the source document
    does. Each is short; each can be read alone; the sequence is the
    most coherent path through.
  </p>
  <ul>
    <li>
      <strong><a href="/theory/ontology">Ontology</a></strong> — Data,
      intention, information, knowledge. The stratified layers.
    </li>
    <li>
      <strong><a href="/theory/manifold">Manifold</a></strong> —
      Positions, fields, streams. The geometry.
    </li>
    <li>
      <strong><a href="/theory/time">Time</a></strong> — Time as a
      primary axis. Events-as-primary. Why this is a substrate
      commitment.
    </li>
    <li>
      <strong><a href="/theory/deformation">Deformation</a></strong> —
      Forming and forgetting. Strong vs. weak. Scarification.
      Self-cohesion and relaxation.
    </li>
    <li>
      <strong><a href="/theory/perspectives">Perspectives</a></strong>
      — Trajectories, not points. Role-perspective vs. individual.
      Access vs. expertise.
    </li>
    <li>
      <strong><a href="/theory/translation">Translation</a></strong> —
      Why translation is irreducible. Bridges. Knowledge as
      relationship.
    </li>
  </ul>
  <p>Two reference surfaces sit alongside:</p>
  <ul>
    <li>
      <strong><a href="/theory/schema">Schema</a></strong> — The
      structural codification: entity types, event structure, topic
      taxonomy, resolved stances. Work in progress.
    </li>
    <li>
      <strong><a href="/theory/open-questions">Open questions</a></strong>
      — What is not yet settled. Updates over time as items resolve.
    </li>
  </ul>
  <p>The model is provisional. It captures a mental picture clearly enough to be argued with.</p>
  <p>
    One symmetry is worth flagging before the model proper. The
    perspective-side has roughly the same shape as the data-side:
    discrete deformations, continuous trajectories, characteristic
    decay rates, prior-and-likelihood structure. Resources and
    perspectives are both on the manifold; both have positions,
    trajectories, and decay; both can be deformed strongly or weakly;
    both have spatial profiles for their effects. This is some
    evidence the primitives are well-chosen rather than ad-hoc — a
    model that requires fewer primitives to describe more phenomena
    is more likely to be sound.
  </p>
</Section>

<Section label="Closing">
  <p>
    I am not trying to win an academic argument or start a movement. I
    am writing this so that when I am six months into implementation
    and tempted to take a shortcut that costs the user some attention
    they will not get back, I have something to read that reminds me
    why I started. I am publishing it because the commitment is more
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
