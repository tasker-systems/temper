<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
</script>

<svelte:head>
  <title>Deformation — temper</title>
  <meta
    name="description"
    content="Deformations are the events that shape the manifold's geometry. Forming and forgetting, strong and weak, scarification, self-cohesion and relaxation."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/theory/deformation</div>
  <h1 class="t-hero-title">Deformation: forming and <em>forgetting</em></h1>
  <p class="tagline t-tagline">
    Deformations are the events that shape the manifold's geometry.
  </p>
</section>

<div class="theory-page">

<Section label="Forming">
  <h2>Forming deformations</h2>
  <p>
    Deformations are the events that shape the manifold's geometry. They
    come in two broad classes — <em>forming</em> and <em>forgetting</em> —
    with several mechanics within each. Two structural primitives —
    <em>self-cohesion</em> and <em>background relaxation</em> — operate
    beneath both classes.
  </p>
  <p>
    A deformation that adds to or alters the manifold's geometry is a
    forming deformation. Forming deformations exist on a continuum of
    strength.
  </p>
  <p>
    <strong>Strong / authoritative deformations</strong> are decisions,
    supersessions, declared topology changes. They are discrete,
    deliberately performed, often with awareness of their effects. They
    overcome the manifold's self-cohesion and substantially bend its
    geometry.
  </p>
  <p>
    <strong>Weak / cumulative deformations</strong> are attention,
    questioning, repeated investigation, sustained focus. They are
    continuous, mostly performed without awareness, and individually
    subtle. They do not overcome self-cohesion on their own, but they
    exert ongoing pressure that, when reinforced, can crystallize into
    something the system represents.
  </p>
  <p>
    The boundary between weak and strong is the <strong>recording
    threshold</strong>: the point at which sub-recording attention
    crystallizes into a tracked artifact. The act of recording — writing
    down the question, capturing the note, opening the ticket — is itself
    a strong-enough deformation to track. Below the threshold, weak
    deformations exist and have effects, but for design purposes their
    effects are <em>integrated into</em> the recorded artifacts that
    subsequently emerge from sustained attention. The model does not
    pretend sub-recording attention is invisible; it accepts that the
    recording threshold is the natural and principled design boundary.
  </p>
</Section>

<Section label="By topic-class">
  <h2>Deformation by <em>topic-class</em></h2>
  <p>
    Different kinds of events deform the manifold in different modes.
    Three are worth naming.
  </p>
  <p>
    <strong>Emission-adds-geometry.</strong> Resource-lifecycle events
    (publications, contributions) and declaration events (decisions,
    role-grants) introduce new positions to the manifold. Each emission
    adds structure that wasn't there before.
  </p>
  <p>
    <strong>Observation-reinforces-geometry.</strong> Observation-topic
    events do not add new positions; they reinforce the position-strength
    of the events they engage. Sustained observation of a region
    strengthens that region's findability; absence of observation lets
    the region decay.
  </p>
  <p>
    <strong>Correction-scars-geometry.</strong> Supersession and
    correction events do both: they deform the topology (the superseded
    region routes through less easily) and they carry the scar-property,
    marking the region as having been wrong before.
  </p>
  <p>
    Bidirectional coupling resolves differently depending on the topic
    the event carries: emissions, observations, and corrections all flow
    through the manifold and shape it, but in distinct ways. The
    <a href="/theory/schema">schema's topic taxonomy</a> names which
    topics fall into which mode.
  </p>
</Section>

<Section label="Forgetting">
  <h2>Forgetting <em>mechanics</em></h2>
  <p>
    Forgetting is not deletion or annotation; it is geometric. There are
    three distinct mechanics, plus a property that can attach to certain
    deformations.
  </p>
  <p>
    <strong>Decay.</strong> A resource that no longer participates in any
    active field drifts away from the active region of the manifold. Its
    findability falls off as a function of distance and
    time-since-reinforcement. Decay is passive, continuous, and
    reversible: re-engagement reinforces position. Decay answers
    <em>no longer relevant</em>.
  </p>
  <p>
    <strong>Deformation (in the forgetting sense).</strong> A decision,
    supersession, or correction can change the manifold's topology
    itself. The superseded region becomes locally less reachable — not
    because it has been labeled, but because the geometry no longer
    routes through it. Deformation is active, discrete, and is performed
    by an agent with the authority to alter topology. Deformation answers
    <em>no longer true</em>.
  </p>
  <p>
    <strong>Folding.</strong> Some resources must be preserved for audit,
    reversibility, or historical understanding, but should not surface in
    default projection. These are folded onto a different <em>sheet</em>
    of the manifold: accessible by deliberate time-travel queries,
    invisible to default projection. Folding answers <em>preserved but
    not present</em>.
  </p>
</Section>

<Section label="Scarification">
  <h2>Scarification</h2>
  <p>
    When a deformation is performed because something was <em>wrong</em>
    — whether through human error, agent hallucination, latent ambiguity
    hardening into false certainty, or any other source — it carries an
    additional property called a <strong>scar</strong>. The scar is not
    the corrected information; it is the <em>audit of the correction</em>:
    a structured memory of what was wrong, what was assumed because of
    it, what had to be reworked.
  </p>
  <p>
    Future engagement with the region gets the corrected state plus the
    awareness that this region has been wrong before — appropriately
    raising scrutiny and lowering confidence in adjacent claims that may
    have rested on the original error.
  </p>
  <p>
    Scarification is not a fourth forgetting mechanic; it is a property
    certain deformations carry. The model is deliberately source-agnostic:
    hallucination, misremembering, conflation, and false certainty all
    leave the same kind of scar. Policy of how scars <em>inform</em>
    engagement — an agent's hallucination region may warrant different
    scrutiny than a human's misremembering region — is a system-design
    question that builds atop the model.
  </p>
  <p>
    The model needs to support not just <em>what is true now</em> but the
    <em>epistemic history of how regions came to be considered true</em>.
    This falls out naturally from events-as-primary
    (<a href="/theory/time">time</a>).
  </p>
</Section>

<Section label="Self-cohesion">
  <h2>Self-cohesion and <em>background relaxation</em></h2>
  <p>
    Two structural primitives operate beneath the deformation mechanics.
  </p>
  <p>
    <strong>Self-cohesion</strong> is the manifold's structural resistance
    to bending under weak forces <em>at any given moment</em>. A weak
    deformation might not bend the manifold meaningfully even immediately,
    because the structural integrity of accumulated recorded geometry
    resists it. Self-cohesion is what makes the recording threshold
    meaningful: weak forces alone don't crystallize; they require either
    accumulation or escalation to a strong-enough event.
  </p>
  <p>
    <strong>Background relaxation</strong> is the manifold's tendency to
    return toward an unweighted ambient state <em>over time</em> when
    deformations aren't reinforced. Strong deformations relax slowly or
    not at all; weak deformations relax quickly. Relaxation is what keeps
    the manifold from accumulating into an unreadable noise floor of
    every question ever asked.
  </p>
  <p>
    Self-cohesion and relaxation answer different questions.
    Self-cohesion: <em>does this deformation register at all?</em>
    Relaxation: <em>does it stick?</em> Both are needed.
  </p>
  <p>
    How scars themselves decay — whether catastrophic past errors resist
    decay differently from incidental ones — is
    <a href="/theory/open-questions#model">an open question</a>.
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
