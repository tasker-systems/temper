<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
</script>

<svelte:head>
  <title>The manifold — temper</title>
  <meta
    name="description"
    content="The model's geometric vocabulary for aboutness: positions, streams, fields, coupling, projection, and context."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/theory/manifold</div>
  <h1 class="t-hero-title">The <em>manifold</em></h1>
  <p class="tagline t-tagline">
    The model's geometric vocabulary for <em>aboutness</em>.
  </p>
</section>

<div class="theory-page">

<Section label="Positions">
  <h2>Positions on the manifold</h2>
  <p>
    All resources — documents, decisions, concepts, sessions,
    observations, anything that can be referred to — occupy positions in
    a high-dimensional space. Position is determined by <em>aboutness</em>,
    and aboutness is multi-axial: a single resource is generally about
    several things at once (a goal, a surface area, a moment in some
    state machine, a constraint, an open question).
  </p>
  <p>
    The manifold is not a tree. Hierarchical organization is at best a
    particular <em>projection</em> of position, and any given projection
    loses information. The manifold is also not flat: it has curvature,
    induced by the fields and deformations described below. Its geometry
    at any moment is the integrated result of all strong deformations
    recorded into the substrate up to that moment, with relaxation rates
    and decay distances applied. There is no "current state" of the
    manifold over and above the integrated event history.
  </p>
</Section>

<Section label="Streams">
  <h2>Streams and <em>particles</em></h2>
  <p>
    Intention in motion is a <em>stream</em> flowing through the
    manifold. Each stream carries information — a query, an
    attention-state, a current focus — and its trajectory is shaped by
    the fields it traverses. Streams are not stored; they are
    <em>witnessed</em>.
  </p>
  <p>
    Stream and particle are two ways of seeing the same phenomenon.
    Stream foregrounds continuity, flow, accumulation. Particle
    foregrounds discreteness, event, witness. The substrate is one
    thing; both vocabulary registers are useful depending on what is
    being described. A session is best described as a stream; a single
    query is best described as a particle within that stream.
  </p>
  <p>
    At the level of the model, a stream is intention in motion. At the
    level of the schema, that motion resolves into events with topics.
    Emissions add new positions to the manifold; observations reinforce
    existing ones; corrections deform and scar. The model's "stream" and
    the schema's topic-classes describe the same phenomenon at different
    resolutions. The <a href="/theory/schema">schema</a> is where the
    resolution lives in full.
  </p>
</Section>

<Section label="Fields">
  <h2>Fields</h2>
  <p>
    Concerns and intentions — active goals, decisions in force, ongoing
    sessions, standing constraints, ambient tolerances, open
    investigations — are <em>fields</em> over the manifold. A field is
    not a position; a field is a force that makes positions matter.
  </p>
  <p>Each field has:</p>
  <ul>
    <li>
      <strong>Spatial profile</strong> — which regions it influences,
      with what intensity at each point.
    </li>
    <li>
      <strong>Weight</strong> — how much it matters at the current
      moment.
    </li>
    <li>
      <strong>Temporal character</strong> — how its profile and weight
      evolve over time.
    </li>
    <li>
      <strong>Characteristic decay distance</strong> — a finite range of
      influence that falls off with distance.
    </li>
  </ul>
  <p>
    Fields interact. Where multiple high-weight fields constructively
    interfere, the manifold's local topology bends toward that region.
    That region is what becomes salient.
  </p>
</Section>

<Section label="Coupling">
  <h2>Bidirectional <em>coupling</em></h2>
  <p>
    The relationship between streams and the manifold is bidirectional.
    Streams flow through the manifold, shaped by its fields. Streams
    also <em>deform the manifold they flow through</em>, including the
    fields that shaped them. The manifold is not a thing the streams
    encounter from outside — the manifold is <em>constituted by</em>
    the history of streams that have flowed through it.
  </p>
  <p>
    This is the model's most consequential commitment. Asking a
    question, choosing what to attend to, deciding what to investigate
    — these shape the field before any artifact is created or any code
    changes. Attention is not neutral observation; attention is force,
    and force deforms.
  </p>
  <p>
    The mechanics of how that deformation operates — what counts as a
    strong deformation, what counts as weak, when sub-recording
    attention crystallizes into something the system tracks — live on
    <a href="/theory/deformation">the deformation page</a>.
  </p>
</Section>

<Section label="Projection">
  <h2>Projection</h2>
  <p>
    What an observer experiences as "the current context" is a
    <em>projection</em> of the stream activity against the
    currently-active field configuration, computed at projection time
    from the integrated event history. Projections are lossy by
    necessity — they collapse a high-dimensional, time-extended reality
    into a finite, addressable surface (a search result, a vault
    listing, a graph view, an editor pane).
  </p>
  <p>
    Different projections serve different purposes; none is the
    territory. Any system that exposes only one projection is hiding
    most of the model from its users.
  </p>
</Section>

<Section label="Context">
  <h2>Context is a <em>verb-state</em></h2>
  <p>
    In conventional knowledge-management vocabulary, <em>context</em>
    is a noun: a container resources live in. In this model, context is
    a verb-state: the currently-active configuration of fields,
    weighted by their current intensities, projected through a temporal
    lens, from the position of a particular perspective.
  </p>
  <p>
    A persistent, low-friction field that happens to be on most of the
    time may <em>behave</em> like a containment-context — it can be
    addressed, named, used as a folder. But it is not ontologically a
    container. It is a long-running field that has settled into
    stationarity.
  </p>
  <p>
    Any addressing scheme that treats context as a noun is taking a
    single low-dimensional projection of position and discarding the
    rest. This may be a useful fiction for stable addressing, but the
    internal representation must be richer than the addressable
    surface, or the model collapses to the noun-version it was supposed
    to replace.
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
