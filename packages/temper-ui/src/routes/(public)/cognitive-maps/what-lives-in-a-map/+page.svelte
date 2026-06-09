<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
  import VizFigure from '$lib/components/VizFigure.svelte';
  import ResourceBlockERD from '$lib/components/cognitive-maps/diagrams/ResourceBlockERD.svelte';
</script>

<svelte:head>
  <title>What lives in a map — temper</title>
  <meta
    name="description"
    content="A map's charter, questions, and regulation are built from two kernel primitives chosen for the capabilities each exposes — resources are atomic and graph-participating; content blocks are addressable and attributable. That difference is what makes provenance and freshness answerable."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/cognitive-maps/what-lives-in-a-map</div>
  <h1 class="t-hero-title">What <em>lives</em> in a map</h1>
  <p class="tagline t-tagline">
    Two kernel primitives, chosen for what each makes answerable.
  </p>
</section>

<div class="cognitive-maps-page">

<blockquote class="epigraph">
  The steward has just concluded something — <em>pair newcomers with a maintainer on the first
  PR</em> — and now has to put it somewhere. To be useful, the lesson has to become two things at
  once: a concept the map can point at and wire into its graph, and recorded text that
  carries where it came from. Those are two different capabilities, and they're the whole
  shape of what a map is built from.
</blockquote>

<Section label="When the steward learns something">
  <p>
    A map earns a lesson and has to find it a home. Watch what the onboarding steward needs the
    moment it concludes <em>pair on the first PR</em>, and the data model nearly designs itself.
  </p>
  <p>
    It needs the lesson to be a <strong>concept it can point at and relate</strong> — wired to the telos,
    referenced by other ideas, present in the graph as a thing in its own right. And it needs the
    lesson's <strong>text to be addressable and attributable</strong> — so the question it answers (<em>"where
    are the sharp edges that scar newcomers?"</em>) can record which engineer's week, which events,
    which systems shaped it, and so someone later can check whether it still holds.
  </p>
  <p>
    Two needs, two kinds of thing. The clearer way into the model isn't a catalogue of primitive
    types; it's to follow each need to the primitive that serves it. Two kernel primitives carry
    all of it — the <strong>resource</strong> and the <strong>content block</strong> — and they split along exactly those
    two needs: what stands on its own and anchors relationships, versus what is addressable and
    attributable inside something else.
  </p>
</Section>

<Section label="The charter is &quot;just&quot; a resource">
  <p>
    Structurally, a telos-charter is nothing special: a <code>kb_resources</code> row, the same kind of
    resource as any concept in the graph. That plainness <em>is</em> the capability. Because the
    charter is an ordinary resource, it <strong>fully participates</strong> — it has its own identity, it can
    be the source or target of edges, it can be a region member, and its history is a projection
    off the event ledger like everything else. A map's purpose isn't sealed in a special charter
    table; it's a first-class citizen of the same graph the map reasons over.
  </p>
  <p>
    This is what <strong>atomicity</strong> is for. A resource is an atomic, self-sufficient unit of
    reference: it can be pointed at from anywhere, found on its own, and it participates in the
    graph as a cohesive whole. You can build relationships <em>to</em> the charter precisely because
    the charter is a thing that stands by itself.
  </p>
</Section>

<Section label="The questions are content blocks">
  <p>
    The charter's guiding questions live <em>inside</em> that resource, as <strong>content blocks</strong> — and
    the change of primitive is deliberate, because a question needs capabilities the
    charter-as-a-whole doesn't. A question has to be <strong>uniquely addressable</strong> (you can point at <em>this</em> one),
    <strong>mutable</strong> (it gets re-stated, reinforced, folded as the map learns), and above all
    <strong>attributable</strong>: a block records which events, in what order, and from which systems shaped
    it into its current form.
  </p>
  <p>
    Attribution is the capability that earns the block as its own primitive. It's what lets a
    map answer <em>"where did this come from?"</em> and <em>"where do I look to check it's still fresh
    against the remote system that fed it?"</em> — a block's provenance is a chain back through the
    ledger to the integrations and acts that formed it. (Body text lives one level deeper still,
    in chunks under a block — the grain at which content is embedded and deduplicated.)
  </p>
  <p>
    The trade that makes this coherent: a content block is <strong>not</strong> atomic and <strong>not</strong>
    self-sufficient. It has a lifecycle of its own, but it does <strong>not</strong> participate in the graph
    independently. Nobody draws an edge to a single question-block from across the map; relations
    attach to the charter resource that contains it. Addressable interiority, not a free-standing
    node — and that's the whole line between the two primitives. Stands alone and anchors
    relationships → resource. Addressable and attributable <em>within</em> a host, its meaning part of
    that host → content block.
  </p>
</Section>

<Section label="Regulation: the tools a map makes for itself">
  <p>
    Regulation is the third thing a map holds, and it isn't a third primitive — it's resources
    again, used a particular way. A map's regulation is an open, <strong>agent-maintained</strong> set of
    concept-resources that exist to <em>express and effect</em> the grounding telos, written as the map
    learns what its purpose demands in practice. In the seed, <em>"pair on the first PR"</em> is a
    resource, homed in the map, marked <code>doc_type = cogmap_regulation</code>, reached from the telos by
    an <code>express</code> edge labelled <code>operationalized_by</code>.
  </p>
  <p>
    Two things give it its character. It's <strong>open</strong> — regulation accrues; it isn't a fixed field
    on the map. And it's <strong>the agent's instrument</strong> — regulation is how a map's steward turns
    <em>what we've learned</em> into <em>what we now do</em>, each piece a tool the map made for itself.
    <a href="/cognitive-maps/how-a-map-grows">How a map grows</a> is partly the story of how a new piece of regulation
    gets written — sometimes the hard way.
  </p>
</Section>

<VizFigure placement="HERO" fidelity="illustrative">
  {#snippet diagram()}
    <ResourceBlockERD id="cm-erd" />
  {/snippet}
  {#snippet shows()}
    the handful of tables that hold a map, populated with <em>the onboarding map's
    own rows</em>: the telos <code>kb_resources</code> row; its <code>kb_resource_homes</code> row anchoring it in the
    cogmap; its charter <code>kb_content_blocks</code> (the telos statement block + three question blocks)
    and their <code>kb_chunks</code>; the <code>kb_cogmaps</code> row pointing at the telos; the <code>express</code> <code>kb_edges</code>
    row to the regulation resource; the <code>doc_type</code> <code>kb_properties</code> rows. Boxes and the
    relationships between them, labelled with real values from the seed — enough to see <em>what
    data is a map</em>, not every column.
  {/snippet}
  {#snippet honestBasis()}
    the tables named above — <code>kb_resources</code>, <code>kb_content_blocks</code>,
    <code>kb_chunks</code>, <code>kb_cogmaps</code>, <code>kb_edges</code>,
    <code>kb_properties</code>; the reads — <code>resource_body_text</code> for the charter
    body and <code>resource_blocks</code> for its question blocks, both resolved through
    <code>cogmap_telos</code>, plus <code>cogmap_regulation</code> — demonstrated by scenario
    <strong>S4</strong>.
  {/snippet}
</VizFigure>

<Section label="How this map came to be">
  <p>
    The charter, the blocks, the cogmap row, the home, the <code>doc_type</code> — all of it is created in
    <strong>one transaction</strong>, by <code>cogmap_genesis</code>. The ordering carries the idea: <strong>resource-first</strong>.
    The telos resource and its blocks are written <em>before</em> the cogmap row, so the map's not-null
    pointer to its telos is satisfiable the moment the map row is inserted — no deferred
    constraint, no half-built map. The charter exists before the map that's organized around it,
    which is the right order for a thing whose whole identity is its purpose.
  </p>
</Section>

<Section label="An open seam">
  <p>
    The resource-or-block call is clean at the extremes and has a soft middle: a thing that's
    <em>mostly</em> interior but occasionally referenced on its own sits right on the line. The artifact
    takes a position — findable-and-graph-participating ⇒ resource — and the seed stays clear of
    the ambiguous cases on purpose. Where exactly the line wants to fall under real content is
    something the empirical load is meant to pressure-test, not something we've frozen.
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
