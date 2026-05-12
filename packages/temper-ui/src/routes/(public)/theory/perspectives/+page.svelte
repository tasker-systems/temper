<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
</script>

<svelte:head>
  <title>Perspectives — temper</title>
  <meta
    name="description"
    content="Perspectives are themselves on the manifold — trajectories with positions, characterizations, and visibility constraints that parallel the data side."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/theory/perspectives</div>
  <h1 class="t-hero-title"><em>Perspectives</em></h1>
  <p class="tagline t-tagline">
    The model's account of <em>who is asking</em> mirrors its account of
    <em>what is asked about</em>. Perspectives are themselves on the
    manifold; they have positions, trajectories, characteristics, and
    visibility constraints that parallel the data side.
  </p>
</section>

<div class="theory-page">

<Section label="On the manifold">
  <h2>Perspectives on the <em>manifold</em></h2>
  <p>
    A perspective-point is not external to the topology — it has a
    position. It is a point capable of emitting intention-vectors, but
    it is in the same space as everything else. The same mechanics that
    govern resources govern perspectives: they have positions, they are
    affected by fields, they decay if not reinforced, they can be
    deformed.
  </p>
  <p>
    Resources and perspectives are both on the manifold: both have
    positions, trajectories, and decay; both can be deformed strongly
    or weakly; both have spatial profiles for their effects.
  </p>
</Section>

<Section label="Trajectories">
  <h2>Perspectives are trajectories, not <em>points</em></h2>
  <p>
    A given individual's perspective changes as they engage with the
    manifold. The perspective on a region today is the integral of past
    engagement with it; it is not a fixed identity but a moving locus.
    Perspective has the same temporal substrate as everything else —
    derived from event history, computed at projection time. Two
    queries from "the same person" at different times come from
    slightly different perspective-points.
  </p>
  <p>
    Because perspective is derived from trajectory rather than stored,
    the schema commits to a corresponding property of the substrate:
    any event's emitter-perspective at the time of emission is
    projectable from the emitter's accumulated trajectory at that time.
    The substrate does not denormalize perspective onto events; it
    preserves the trajectory and computes perspective when asked.
  </p>
</Section>

<Section label="Characterization">
  <h2>Perspective <em>characterization</em></h2>
  <p>A perspective-point has at least:</p>
  <ul>
    <li><strong>Identity</strong> — who or what this perspective is.</li>
    <li>
      <strong>Reliability profile</strong> — the prior on intentions
      from this perspective producing accurate information.
    </li>
    <li>
      <strong>Characteristic intention-vectors</strong> — what kinds of
      intentions this perspective typically emits, with what
      magnitudes.
    </li>
    <li>
      <strong>Domain-specificity</strong> — reliability is not uniform;
      it has a <em>spatial profile</em> over the manifold, with
      high-confidence regions and low-confidence ones.
    </li>
  </ul>
  <p>
    Domain-specificity matters most. A single perspective has different
    reliability in different regions of the manifold. An expert in
    distributed systems is a high-magnitude perspective in that region
    and a low-magnitude one in cell biology. Perspective itself has
    spatial structure — much like fields do.
  </p>
</Section>

<Section label="Role vs individual">
  <h2>Role-perspective and <em>individual-perspective</em></h2>
  <p>
    There are two related but distinct kinds of perspective the model
    represents.
  </p>
  <p>
    A <strong>role-perspective</strong> is a characterizable
    perspective-class: a kind of perspective characterized by its
    concerns, KPIs, accountabilities, characteristic intention-vectors,
    and characteristic field-set. Examples: "ops department head,"
    "product manager," "technical lead," "cloud-agent session working a
    ticket." A role is a specification of expected patterns.
  </p>
  <p>
    An <strong>individual-perspective</strong> is a specific
    perspective-trajectory that may instantiate one or more
    role-classes at different times. A particular person inhabits a
    role; an agent session occupies a role; over time, both develop
    their own trajectory within and across roles.
  </p>
  <p>
    The two relate the way priors and likelihoods relate in Bayesian
    inference. The role-persona is a <em>prior</em> on the kinds of
    intention-vectors this perspective is likely to emit. The
    individual's trajectory provides <em>likelihoods</em> — actual
    evidence of what queries this particular instantiator makes. The
    working model of a perspective at any given time is the
    <em>posterior</em>: the role-prior updated by accumulated
    individual evidence. New individuals in a role inherit the prior
    (low cold-start cost); over time, their actual trajectory updates
    the model for them specifically — without ever fully discarding
    the role-persona that anchors what kind of perspective this is at
    all.
  </p>
  <p>
    Role-changes are <em>strong deformations</em> of the
    perspective-point's position — discrete, intentional, performed
    with awareness, substantially shifting the perspective's
    characteristic intention-vectors. Individual evolution within a
    role is <em>trajectory accumulation</em> — continuous, integrated
    against decay rates. The two operate at different temporal scales
    but with the same primitives that govern the rest of the manifold
    (<a href="/theory/deformation">deformation</a>).
  </p>
  <p>
    Accumulated individual trajectory may itself become signal for
    role-change: drift toward a different role's prior, persistent
    atypicality of the codified role, sophistication accumulation
    suggesting expertise rather than role transition. The model does
    not resolve these signals automatically — that would presume a
    univocal frame. But it can <em>surface</em> them. Role-evolution
    is itself a translation problem.
  </p>
</Section>

<Section label="Aggregates">
  <h2>Aggregate-<em>perspectives</em></h2>
  <p>
    There is a third kind of perspective the model represents. A team,
    a squad, an affinity group, a leadership committee — these are
    aggregate-perspectives, with their own class-priors and their own
    trajectories. An EPD squad's characteristic intentions are not the
    sum of its members'; a leadership team's position in
    perspective-space is not reducible to the individuals in it.
  </p>
  <p>
    The model commits to a specific asymmetry. Aggregate-perspectives
    have positions and accumulate trajectories, but they do not
    <em>emit</em>. Emission is always discrete-non-aggregate — a
    particular individual sends the document, even when the document
    is from the team. What the aggregate provides is <em>scope</em>:
    the context within which the discrete emission happens. The schema
    names this scope as <strong>on-behalf-of</strong> — the relation
    between an emitting individual and the aggregates whose context
    their emission occupies.
  </p>
  <p>
    The aggregate's trajectory is computed from the events emitted
    on-behalf-of it — never from emissions the aggregate itself
    performed, because it never performs any. This keeps
    events-as-primary intact while letting aggregate-perspectives have
    positions and histories.
  </p>
</Section>

<Section label="Personas">
  <h2>Personas as <em>field-sets</em></h2>
  <p>
    A persona is a useful name for the complete role-perspective
    specification: position on the manifold, characteristic field-set,
    typical resolution profile, characteristic intention-vector
    patterns. The "jobs to be done" frame names <em>what
    intention-vectors are typically active for this role</em> — the
    bundle of fields usually active when this perspective is at work,
    with rough relative weights. Codifying personas-and-jobs-to-be-done
    is not reductive of individual variation; it is the system
    pre-paying the cost of common queries in service of
    attention-economy.
  </p>
</Section>

<Section label="Visibility">
  <h2>Visibility: access and <em>expertise</em></h2>
  <p>
    A perspective's available manifold is bounded by two distinct
    mechanics.
  </p>
  <p>
    <strong>Access</strong> is hard, mechanical, topological. Certain
    regions of the manifold are not present from a given perspective —
    folded out of reach by RBAC, encryption, organizational
    compartmentalization. Access produces perspective-specific
    topology in the weak observer-relativity sense: each perspective
    has its own <em>visible manifold</em>, a sub-region of the
    underlying substrate. Access can be enforced mechanically and
    reasoned about formally.
  </p>
  <p>
    <strong>Expertise</strong> is soft and resolutional. Data may be
    present and accessible, but the perspective lacks the
    <em>resolution apparatus</em> to read it as information. A novice
    and an expert see the same data and produce different information
    from it. Expertise connects directly to domain-specificity: it is
    the perspective's resolving power over different regions of the
    manifold, varying spatially.
  </p>
  <p>
    Expertise is itself a function of perspective-trajectory. It
    accumulates through engagement; it can in principle be computed
    from event history by integrating past engagement against decay
    rates. Expertise that isn't reinforced fades. This gives a
    non-magical account of why expertise has the shape it has at any
    given time.
  </p>
  <p>
    Access creates <em>what is in the manifold for this perspective</em>.
    Expertise creates <em>what can be made into information from what
    is in the manifold</em>. Both bound knowledge production, but at
    different layers.
  </p>
</Section>

<Section label="Observer-relativity">
  <h2>Observer-relativity (weak <em>version</em>)</h2>
  <p>
    The model adopts the <em>weak</em> version of observer-relativity.
    A strong version — that each observer has a fully private manifold
    and there is no global geometry — may be true in some deep
    information-theoretic sense, but is not actionable. Designing as
    if it were true makes collaboration incoherent.
  </p>
  <p>
    The weak version: there is a shared substrate of events, but each
    observer's projection includes their own accumulated deformation
    history. Two observers who have engaged with the same regions in
    similar ways will converge on similar projections; observers with
    divergent histories will project differently even given identical
    substrate. <strong>Shared understanding is therefore an emergent
    property of convergent projection histories, not something
    guaranteed by shared substrate.</strong>
  </p>
  <p>
    The mechanics of how perspectives bring their information shapes
    into productive contact — and what fails when they cannot — live
    on <a href="/theory/translation">the translation page</a>.
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
