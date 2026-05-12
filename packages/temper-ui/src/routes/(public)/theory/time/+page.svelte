<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
</script>

<svelte:head>
  <title>Time — temper</title>
  <meta
    name="description"
    content="Time is co-equal with position. Every element of the model is temporally extended, and bidirectional coupling forces events as the substrate."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/theory/time</div>
  <h1 class="t-hero-title"><em>Time</em></h1>
  <p class="tagline t-tagline">
    Time is not an afterthought; it is co-equal with position.
  </p>
</section>

<div class="theory-page">

<Section label="Temporal extension">
  <h2>Every element of the model is <em>temporally extended</em></h2>
  <ul>
    <li>A field's spatial profile drifts as the work it represents shifts.</li>
    <li>A field's weight rises and falls as concerns gain and lose currency.</li>
    <li>A stream is constitutively temporal — there is no stream without time.</li>
    <li>A projection has a <em>temporal lens</em>: it can be taken at the present moment, at a prior moment, or integrated over a window.</li>
    <li>Resources have temporal validity — they are true with respect to a field configuration that may have since changed.</li>
    <li>Perspectives are themselves trajectories rather than points (<a href="/theory/perspectives">perspectives</a> returns to this).</li>
  </ul>
</Section>

<Section label="Events as substrate">
  <h2>Why this forces <em>events as the substrate</em></h2>
  <p>
    Bidirectional coupling forces a substantive substrate commitment. If
    the manifold's geometry is a function of stream history, then stream
    history is the primary data and the manifold is derived.
    <strong>Event-sourcing is not one substrate option among many; it is the only substrate consistent with the model.</strong>
    Distributed, ledger-flavored, or single-node implementations all
    remain viable, but events-as-primary is load-bearing for the
    semantics. Geometry is computed against history rather than
    maintained as a separate snapshot.
  </p>
  <p>
    Two consequences follow that the model commits to. First,
    entity-state is itself derivable from events — role-changes,
    position-updates, membership-changes are all emissions of
    state-change topics, append-only like everything else. The substrate
    doesn't store entity-state; it stores the events from which
    entity-state is computed. Second, point-in-time truth is
    cross-cutting: for any event at time T, the state of every entity
    associated with that event — emitter, on-behalf-of scopes,
    observers — is projectable at T from their trajectories. The
    substrate carries the events; everything else, including the
    temporal truth of who-was-where-when, is computed.
  </p>
  <p>
    Specific implementations remain downstream of the model. Caching,
    indexing, and materialization tradeoffs — when derived structure
    must be maintained rather than computed, and what reconciliation
    looks like when it is — are <a href="/theory/open-questions#schema">open</a>.
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
