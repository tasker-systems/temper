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
  <h2>Forgetting is what makes memory <em>meaningful</em></h2>
  <p>
    Memory in an information system is only as effective as its
    mechanics for forgetting. Without decay (time-and-distance
    fading), active deformation (the revision and topological change
    that follows from new decisions or new data), and scarification
    (the trace of past mistakes carried into future engagement),
    signal washes out in noise — and attention can no longer land on
    what matters now.
  </p>
  <p>
    A system that lets everything stay equally findable, equally
    true-sounding, equally clean has not committed to attention; it
    has only committed to retention.
  </p>
</Section>

<Section label="Commitment 2">
  <h2>The system must be capable of <em>deforming itself</em></h2>
  <p>
    An information system must be able to change its own shape —
    actively, when decisions are codified into the substrate, and in
    aggregate, when thresholds are crossed or patterns arrived at in
    practice. The information landscape is not static. Decisions
    supersede earlier ones; concepts merge or split; tolerances
    shift; what was once peripheral becomes central.
  </p>
  <p>
    A system that requires every change to come from an explicit
    author act has externalized work the system itself should do.
  </p>
</Section>

<Section label="Commitment 3">
  <h2>Data is only information when it has a <em>frame of reference</em></h2>
  <p>
    Data is not information. Data becomes information when it is
    engaged from a particular perspective, at a particular time, at
    a particular resolution, for a particular purpose. What matters,
    to whom, at what level of granularity, on what time horizon —
    these are not secondary concerns to be layered on top of "the
    data." They are what makes data legible as anything at all.
  </p>
</Section>

<Section label="Commitment 4">
  <h2>Projections and summaries are <em>different acts</em></h2>
  <p>
    A projection is what happens when a perspective's intention
    queries the substrate and renders what is relevant from a
    position in perspective-space. It is lossy by necessity,
    perspective-laden by construction, but it does not claim to be
    a new resource.
  </p>
  <p>
    A summary is something else: a deliberately crafted artifact
    that selects what to include, weights what to emphasize, and
    manages the boundary of what gets disclosed to inform a
    decision. Summaries carry intention and implicit analysis; they
    are not value-neutral views of the substrate.
  </p>
  <p>
    Both projections and summaries are necessary. But if the system
    cannot distinguish them, every summary becomes an unmarked
    reweighting of how the underlying information will be read — and
    decision-making compounds on top of views that look neutral but
    are not.
  </p>
</Section>

<Section label="Commitment 5">
  <h2>Events and observers weave through <em>every kind of work</em></h2>
  <p>
    Customer discovery, research, requests, analytical insights,
    recommendations, prototypes, tickets, pull requests — every kind
    of work artifact carries topics, scopes, and relationships that
    make it more or less useful to different consumers at different
    times. The operational pattern connecting them is not a separate
    workflow layer bolted on top; it is the events-and-observers
    structure of the substrate itself.
  </p>
  <p>
    Resources are not free-floating. They are situated, scoped,
    related — and only when those properties are first-class can the
    system route attention to what matters now, for whom, and at
    what resolution.
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
    One commitment in the model runs through everything that follows:
    the system stores data and traces of past intentional acts —
    recorded questions, notes, decisions, which themselves become further data.
    It does not store <em>knowledge</em>. Knowledge is produced in the generative
    space defined by a perspective-as-intention and the information-from-data that
    this perspective describes, transformed into a cognitive or operational
    artifact - a tool, in effect, that can be used.
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
    There is an interesting symmetry that emerges - the
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
    This document is not an academic argument or some kind of declaration of
    principles that are meant to cast aspersions on existing systems. I
    am writing this so that when I am six months into implementation,
    I remember what *matters* - this document itself is a form of respecting
    my own limited attention. I am publishing it because the commitment is more
    honest if it is shared, and because whether this project is successful or not,
    the principles and commitments are, I believe, worth engaging with for others
    who are likewise working in an information management space.
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
