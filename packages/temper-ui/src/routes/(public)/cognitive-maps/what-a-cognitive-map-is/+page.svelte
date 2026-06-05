<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
  import VizFigure from '$lib/components/VizFigure.svelte';
  import RegionFieldDiagram from '$lib/components/cognitive-maps/diagrams/RegionFieldDiagram.svelte';
</script>

<svelte:head>
  <title>What a cognitive map is — temper</title>
  <meta
    name="description"
    content="A new engineer's first week, and the tended region of understanding that exists to get her there. What a cognitive map does — hold a purpose, keep asking, accrete what's learned, show its shape without dumping its contents — and the name we give to that."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/cognitive-maps/what-a-cognitive-map-is</div>
  <h1 class="t-hero-title">What a cognitive map <em>is</em></h1>
  <p class="tagline t-tagline">
    A telos-seeded incubation home — no inside, no outside.
  </p>
</section>

<div class="cognitive-maps-page">

<blockquote class="epigraph">
  A new engineer starts Monday; by Friday the aim is a merge she trusts.
  Something has to hold the question of <em>how she gets there</em> and keep
  working at it as it learns. That tended understanding-toward-a-purpose is what
  we'll come to call a cognitive map — and what it <em>does</em> is the better
  way in than what it <em>is</em>.
</blockquote>

<Section label="A week-one problem">
  <p>
    A new engineer joins epd-team-a. The goal that matters first is small and
    concrete: a pull request she actually trusts, inside the first week. Getting
    her there isn't a document anyone can hand over — it's a live question, with
    a few more underneath it. What does she already know that carries over?
    What's the smallest real change that builds confidence? Where are the sharp
    edges that scar newcomers?
  </p>
  <p>
    Those aren't rhetorical. In the running scenario they're written down — the
    standing questions of a part of the system that exists for exactly this
    purpose, seeded with the goal <em>"help a new EPD engineer reach first-merge
    confidence in week one."</em>
  </p>
</Section>

<Section label="The thing doing the work">
  <p>
    What makes it more than a folder of onboarding notes is that something
    <em>tends</em> it. <code>onboarding-agent#1</code> — an agent working as a
    steward — holds those questions, watches what actually happens as engineers
    come through, and records what it learns. When pairing a newcomer with a
    maintainer turns out to head off the worst scars, that doesn't stay an
    observation; it becomes something the map now <em>does</em>: a small standing
    rule, <em>pair on the first PR.</em>
  </p>
  <p>
    A purpose, the questions it keeps asking, the understanding gathering around
    it, and an agent keeping the whole thing current — that working apparatus is
    what we call a <strong>cognitive map</strong>. The name is a handle for the
    thing we've just watched do its job, not a category it belongs to.
  </p>
</Section>

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

<Section label="What you reach for it to do">
  <p>
    The thing you actually use a map for is to <em>see where understanding has
    gotten</em> without reading everything inside it. A map offers a
    <strong>shape</strong>: from wherever you stand, you can see the regions it
    has formed, how much each one matters under its purpose, and roughly how
    populated each is — while the material those regions are made of stays
    something you reach for piece by piece, and can be refused.
  </p>
  <p>
    So it isn't a container with an interior you're admitted to. It's a shape it
    shows openly and a set of materials you ask for individually — no membrane,
    no act of <em>entering</em>, nothing bored through. That split —
    <strong>shape you can see, materials you reach for</strong> — is what later
    makes maps legible to
    <a href="/cognitive-maps/how-maps-relate">one another</a> and gives
    <a href="/cognitive-maps/whats-visible-from-here">visibility</a> something to
    work on.
  </p>
</Section>

<VizFigure placement="HERO" fidelity="conceptual">
  {#snippet diagram()}
    <RegionFieldDiagram id="cm-region-field" />
  {/snippet}
  {#snippet shows()}
    an emergent field of concept-points that has settled into one or two
    <strong>density regions</strong>. Each region carries a <strong>label</strong>
    and a <strong>salience weight</strong> (how much it matters under the map's
    telos), and a <code>member_count</code> blur — you can see <em>that</em>
    roughly N things cluster here, but the individual members are deliberately
    not drawn. The honest picture of a cognitive map: a shape with weight and
    population, <strong>not</strong> a walled garden with a gate.
  {/snippet}
  {#snippet honestBasis()}
    <code>kb_cogmap_regions</code> (<code>centroid</code>, <code>salience</code>,
    <code>label</code>, <code>member_count</code>); the surface read
    <code>cogmap_shape()</code>, which returns exactly <em>salience / label /
    member_count</em> and <strong>never</strong> member identities. Scenario
    <strong>S6</strong> demonstrates the surface returning the blur with
    identities withheld.
  {/snippet}
</VizFigure>

<Section label="Why a shape and not a dump">
  <p>
    Why reach for the shape and not the contents? Because handing over everything
    a map holds would spend the exact attention the system exists to conserve —
    so instead it shows where the weight is and lets you spend attention
    deliberately. For the new engineer's steward, that means meeting <em>"pair on
    the first PR"</em> and the sharp-edges question first, not wading through
    every note the map has ever absorbed. The map is economical with attention by
    construction — the thesis already visible in the smallest unit.
  </p>
</Section>

<Section label="An open seam">
  <p>
    One thing this leaves unsettled: <strong>when</strong> a map's shape gets
    re-drawn. The regions are <em>materialized</em> — computed at a moment and
    then held — so a shape can sit slightly behind the events that have since
    touched the map. The system treats that as normal and reports it honestly
    rather than blocking on freshness (the staleness signal is real and
    on-read). <em>What should wake an agent to re-materialize a shape</em> —
    event volume? a cadence? salience crossing a floor? — is genuinely open. We
    mark it as a seam and return to it in
    <a href="/cognitive-maps/how-a-map-grows">how a map grows</a> and the
    <a href="/cognitive-maps/operating-temper/deployment">deployment</a> forks.
    Naming a seam <em>as</em> a seam, instead of papering over it, is the
    discipline the system applies to superseded thinking — and the one these
    pages try to keep.
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

  .epigraph {
    max-width: 800px;
    margin: 0 auto 1rem;
    padding: 0.5rem 2.5rem 0.5rem 3.75rem;
  }
  :global(.cognitive-maps-page .epigraph) {
    border-left: 2px solid var(--temper-blue-border);
    font-family: var(--font-serif);
    font-style: italic;
    color: var(--parchment);
    line-height: 1.7;
  }
</style>
