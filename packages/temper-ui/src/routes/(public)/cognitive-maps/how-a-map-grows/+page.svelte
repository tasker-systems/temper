<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
  import VizFigure from '$lib/components/VizFigure.svelte';
  import LearningActsDiagram from '$lib/components/cognitive-maps/diagrams/LearningActsDiagram.svelte';
</script>

<svelte:head>
  <title>How a map grows — temper</title>
  <meta
    name="description"
    content="Five learning-acts — form, modify, decay, fold, scar — each mapped to a real mechanism, not a metaphor. The agent that performs them is a persona; the actor that emits is an entity."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/cognitive-maps/how-a-map-grows</div>
  <h1 class="t-hero-title">How a map <em>grows</em></h1>
  <p class="tagline t-tagline">
    Five learning-acts — each a mechanism, not a metaphor.
  </p>
</section>

<div class="cognitive-maps-page">

<blockquote class="epigraph">
  Watch the onboarding map over a few weeks. A question keeps getting leaned on
  and grows sturdier. An old assumption quietly stops being used — then one week
  it backfires, and the steward folds it <em>and</em> writes down what it cost.
  Each of those is one of five things that can happen to an idea in a map, and
  not one is a metaphor.
</blockquote>

<VizFigure placement="HERO" fidelity="conceptual">
  {#snippet diagram()}
    <LearningActsDiagram id="cm-acts" />
  {/snippet}
  {#snippet shows()}
    what an agent does when an event arrives, as a flow: <strong>inbound
    event</strong> → <strong>load the charter</strong> (telos + questions)
    <strong>and regulation</strong> → <strong>relevance call against <em>this
    map's</em> telos</strong> → <strong>one of the five learning-acts</strong> →
    <strong>emit provenance-with-stance</strong> back to the ledger. The
    <strong>scar path</strong> drawn explicitly as its own branch: <em>fold the
    question</em> <strong>and</strong> <em>write a regulation resource</em>, with
    the provenance link back to the folded block. The reader should see that
    growth is a loop, and that scar is the branch that feeds regulation.
  {/snippet}
  {#snippet honestBasis()}
    <code>kb_block_provenance</code> (<code>accretion_seq</code>,
    <code>is_corrected</code>, <code>source_kind</code>) and the
    <code>is_folded</code> gates on <code>kb_content_blocks</code> /
    <code>kb_edges</code>; the <code>cogmap_charter</code> /
    <code>cogmap_questions</code> / <code>cogmap_regulation</code> reads the agent
    loads. The five learning-acts are canonical from the
    <em>2026-05-29 resolution-contract</em>.
  {/snippet}
</VizFigure>

<Section label="A few weeks in the life of a map">
  <p>
    Give the onboarding map a little time, and watch what happens to its ideas.
  </p>
  <p>
    One of its questions — <em>"what's the smallest real change that builds
    confidence?"</em> — keeps getting leaned on. Every engineer who comes through
    references it, and the map notices: the question grows sturdier, not because
    anyone bumped a number, but because the traffic past it is real. That's
    <strong>reinforcement</strong>.
  </p>
  <p>
    An assumption it once held — <em>new folks should start by reading the whole
    architecture doc</em> — quietly stops getting used. Nobody argues it down; it
    just stops drawing references, and its standing falls on its own. That's
    <strong>decay</strong>.
  </p>
  <p>
    Then one week the assumption backfires: a newcomer sinks three days into that
    doc and comes out <em>less</em> confident. The steward doesn't simply drop it.
    It <strong>folds</strong> the assumption — sets it aside as superseded,
    without erasing that it was ever held — <em>and</em> writes the lesson that
    replaces it into the map's regulation: <em>pair on the first PR.</em> Fold,
    plus a lesson written forward: that's a <strong>scar</strong>.
  </p>
  <p>
    Reinforcement, decay, fold, scar — plus the plain <strong>forming</strong> of
    something new — are the five things that can happen to an idea in a map. Each
    is a real mechanism in the kernel, not a mood, and the rest of this page is
    what each one actually is.
  </p>
</Section>

<Section label="The five, named">
  <ul>
    <li><strong>form</strong> — a new concept or relation appears;</li>
    <li>
      <strong>modify</strong> — something already there is re-stated or
      re-weighted (reinforcement is the quiet case);
    </li>
    <li>
      <strong>decay</strong> — something fades because nothing keeps reinforcing
      it (restraint, not deletion);
    </li>
    <li>
      <strong>fold</strong> — something is deliberately set aside as superseded —
      <em>preserved, not wrong</em>;
    </li>
    <li>
      <strong>scar</strong> — the hardest one: a thing is folded <em>and</em> the
      lesson from folding it is written into regulation.
    </li>
  </ul>
  <p>
    Each maps to a mechanism already in the kernel. None is a special "learning"
    table.
  </p>
</Section>

<Section label="Reinforcement is derived, not stored">
  <p>
    The act that surprised us most is <strong>modify</strong>, specifically
    reinforcement — because there's nothing to bump. There is no weight column on
    a question that an agent increments when the question proves useful. A
    question's standing is <em>read from the reference stream</em>: the count and
    recency of provenance accretions into its block. <code>cogmap_questions</code>
    returns a <code>reinforce_count</code>, and that number is a
    <code>count(...)</code> over <code>kb_block_provenance</code>, not a stored
    field.
  </p>
  <p>
    The reason this is the right shape and not just a clever one: the substrate
    exposes the raw reference stream honestly, and any narrowing of "referenced"
    down to "confirmed" is a tuning decision an agent makes out in the open, not a
    number baked into storage where nobody can see how it was set.
  </p>
</Section>

<Section label="Decay and fold">
  <p>
    <strong>Fold</strong> is a visibility act, and it's orthogonal to whether
    something is current. Setting <code>is_folded</code> on a block (or an edge)
    stops the projection from surfacing it; the event that folded it stays on the
    ledger; the content is <em>preserved, not deleted</em>. "Wrong" is not the
    claim — "superseded" is. Anyone re-engaging the history can still see what was
    set aside and when.
  </p>
  <p>
    <strong>Decay</strong> is the softer cousin of fold: nothing is actively set
    aside, a thing simply stops being reinforced and its standing falls on its
    own. Restraint rather than action. The map gets lighter without anyone
    deciding to delete.
  </p>
</Section>

<Section label="Scar">
  <p>
    <strong>Scar</strong> is fold with a memory. A question (or concept) is
    folded, <em>and</em> a lesson is written into the map's regulation, linked
    back to the folded block through <code>kb_block_provenance</code> — a
    provenance row that says, in effect, <em>"this lesson came from something
    scarring."</em> It's the act that turns a painful supersession into durable
    guidance instead of a silently-dropped row.
  </p>
  <p>
    That's the scar the page opened with, now in mechanism: the architecture-doc
    assumption folded, <em>"pair on the first PR"</em> written into regulation,
    and a <code>kb_block_provenance</code> row tying the new lesson back to what it
    replaced. The charter's third question — <em>"where are the sharp edges that
    scar newcomers?"</em> — is the map asking, in advance, for exactly this.
  </p>
</Section>

<Section label="The actor is an entity">
  <p>
    A word on <em>who</em> does all this, because the modelling is deliberate. An
    agent is a <strong>persona</strong> — a telos-bearing behaviour, a way of
    attending to a map. But the thing that actually emits an event is an
    <strong>entity</strong>, and <code>emitter_entity_id</code> is
    <code>NOT NULL</code> on every event. Persona is behaviour; entity is the
    actor of record.
  </p>
  <p>
    <code>onboarding-agent#1</code> is an entity. Its launch details —
    <code>model: claude-opus-4-8</code>, <code>persona: steward</code>,
    <code>bound_cogmap: onboarding-cogmap</code> — live in an <strong>open
    <code>metadata jsonb</code></strong>, not in a frozen <code>entity_kind</code>
    enum. That openness is the hinge into
    <a href="/cognitive-maps/operating-temper">operating Temper</a>: a GitHub
    webhook or a Notion integration is <em>the same kind of entity</em>, writing
    <em>the same ledger</em>, with its own launch metadata in the same open field.
    The system doesn't privilege "an agent" over "an integration" — both are
    entities that emit, and that's what lets external systems become first-class
    writers later.
  </p>
</Section>

<Section label="An open seam">
  <p>
    What we've described is the agent's loop <em>once it's awake</em>. What
    <strong>wakes it</strong> is genuinely unsettled — event volume, a time
    cadence, salience crossing a floor. It's the same seam the opening raised
    about re-materializing shape, and it matters enough to be one of the explicit
    "help us decide" forks in
    <a href="/cognitive-maps/operating-temper/deployment">deployment</a> (the
    <em>"temper-system dreaming"</em> question). We mark it here rather than
    pretend the cadence is solved.
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
