<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
  import VizFigure from '$lib/components/VizFigure.svelte';
  import LedgerSpineDiagram from '$lib/components/cognitive-maps/diagrams/LedgerSpineDiagram.svelte';
</script>

<svelte:head>
  <title>The substrate beneath it — temper</title>
  <meta
    name="description"
    content="Pull any thread in a map and it ends at an event. What the substrate does — make every part of a map answerable to how it came to be — and how the present is materialized from an append-only ledger, with the kernel kept convention-agnostic."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/cognitive-maps/the-substrate-beneath-it</div>
  <h1 class="t-hero-title">The substrate <em>beneath</em> it</h1>
  <p class="tagline t-tagline">
    Pull any thread, and it ends at an event.
  </p>
</section>

<div class="cognitive-maps-page">

<blockquote class="epigraph">
  A week later, another engineer hits the same wall — and <em>"pair on the first PR"</em> is
  already there to meet them. Lean on a rule like that and a fair question follows: where did it
  come from, and is it still what we think? Pull the thread and you never reach a typed-in row.
  You reach an event. That's what the substrate does: it makes every part of a map answerable to
  <em>how it came to be.</em>
</blockquote>

<Section label="Where a rule comes from">
  <p>
    A week into the onboarding map's life, a second engineer hits the same sharp edge the first
    one did, and the steward's rule — <em>pair on the first PR</em> — is already there to meet
    them. Anyone leaning on a rule like that is owed two answers: <em>where did this come from,</em>
    and <em>can I trust it's still current?</em>
  </p>
  <p>
    Pull the thread in Temper and you don't land on a row someone typed and could quietly have
    edited. You land on an <strong>event</strong>: the moment the lesson was recorded, by which
    actor, under which map. Pull <em>any</em> thread in a cognitive map — a region, an edge, a
    charter question — and it ends the same way, at an event on an append-only ledger. That
    answerability is the whole job of the substrate.
  </p>
</Section>

<Section label="Events are the truth; everything else is derived">
  <p>
    So the single decision everything else in Temper leans on: <strong>events are the source of
    truth, and everything else is derived.</strong> A resource, an edge, a region, a charter block
    — each is a <em>projection</em>: a materialized current-state view the system maintains, and
    could always rebuild, from the events that produced it. The projection is what you query; the
    ledger is what's true.
  </p>
  <p>
    Everything downstream — supersession that leaves a mark, folding that preserves rather than
    deletes, provenance you can follow — falls out of that one choice.
  </p>
</Section>

<Section label="What an event carries">
  <p>
    The ledger is <code>kb_events</code>. Every change lands there as an event, and an event
    carries four things that matter:
  </p>
  <ul>
    <li>
      an <strong>emitter</strong> — <em>who</em> acted, modelled as an <em>entity</em>, never a
      bare person (the reason surfaces in
      <a href="/cognitive-maps/how-a-map-grows">how a map grows</a>);
    </li>
    <li>
      a <strong>type</strong> — what kind of act it was (<code>cogmap_seeded</code>,
      <code>relationship_asserted</code>, <code>block_mutated</code>, and so on);
    </li>
    <li>
      an optional <strong>producing anchor</strong> — the cogmap or context the act happened in,
      kept as <em>provenance</em>, not as the access gate;
    </li>
    <li>
      a <strong>correlation id</strong> — the thread that ties a multi-event act together.
    </li>
  </ul>
  <p>
    It only ever grows. The present is a fold over the past, not a row someone overwrote — which
    is what let the second engineer's question have a real answer.
  </p>
</Section>

<Section label="The present is materialized, not replayed">
  <p>
    The things you actually read — resources, content blocks, edges, properties, regions — are
    projection tables. Each carries its lineage back to the ledger: an
    <code>asserted_by_event_id</code> (or <code>genesis_event_id</code>) for where it came from,
    and a <code>last_event_id</code> for the event that last changed it. They're maintained, not
    recomputed on every read — kept materialized so a query doesn't replay history each time.
  </p>
  <p>
    That materialization is where the <em>current-state machinery</em> lives, and it earns its
    place. The full-text index, the vector embeddings used for similarity, the weights on edges,
    the salience on regions — all of it is computed onto the projections, not the ledger. Regions
    in particular are recomputed on a <strong>cadence</strong> rather than continuously: a region's
    shape is a snapshot taken at a moment, which is exactly why the system can tell you, on read,
    when that snapshot has gone stale. The ledger holds what happened; the materialized present
    holds what's currently searchable, rankable, and salient.
  </p>
  <p>
    This is also where forgetting becomes mechanical. When something is folded, it gets an
    <code>is_folded</code> flag — the projection stops surfacing it, the indexes stop ranking it,
    and the event that folded it stays on the ledger. Nothing was destroyed; what changed is what
    the current state carries forward. Decay, fold, and scar — the moves a map makes as it grows —
    all happen on the materialized present, against a ledger that forgets nothing.
  </p>
</Section>

<Section label="The onboarding map, traced">
  <p>
    Take the map the engineers are leaning on and follow it down. It didn't spring into being —
    it was <em>emitted</em>:
  </p>
  <ul>
    <li>a single <code>cogmap_seeded</code> event is the genesis correlation root;</li>
    <li>
      the <strong>telos resource</strong> and its <strong>charter blocks</strong> (the statement,
      then the three questions) were written under that event;
    </li>
    <li>
      the <strong>regulation</strong> — <em>"pair on the first PR"</em> — hangs off the telos by
      an <code>express</code> edge, itself asserted by an event;
    </li>
    <li>
      the <strong>materialized region</strong> the steward reads cites the event it was
      materialized under.
    </li>
  </ul>
  <p>
    Every part of the map traces to the ledger. The map is what those events currently add up to.
  </p>
</Section>

<VizFigure placement="HERO" fidelity="conceptual">
  {#snippet diagram()}
    <LedgerSpineDiagram id="cm-ledger" />
  {/snippet}
  {#snippet shows()}
    the event ledger (<code>kb_events</code>) drawn as a single <strong>append-only spine</strong>
    down the page, with <strong>projections</strong> branching off it: resources, content blocks,
    edges, properties, and cogmap-regions, each connected back to the spine by its
    <code>asserted_by</code> / <code>last_event</code> lineage. Above the spine, the kernel shown as
    <strong>convention-agnostic</strong>: the same ledger + entities + resources underlie
    <em>both</em> Temper's workflow / knowledge-base patterns <em>and</em> the cognitive map, with
    neither baked into the schema. The reader should come away with: one source of truth at the
    bottom, everything else hanging off it.
  {/snippet}
  {#snippet honestBasis()}
    <code>kb_events</code> (<code>emitter_entity_id</code>, <code>event_type_id</code>,
    <code>producing_anchor_*</code>, <code>correlation_id</code>); the lineage columns
    (<code>asserted_by_event_id</code> / <code>last_event_id</code> / <code>genesis_event_id</code>)
    on <code>kb_resources</code>, <code>kb_edges</code>, <code>kb_properties</code>,
    <code>kb_cogmap_regions</code>, <code>kb_content_blocks</code>.
  {/snippet}
</VizFigure>

<Section label="The kernel doesn't pick sides">
  <p>
    The kernel underneath is deliberately <strong>convention-agnostic</strong>. It knows about
    events, entities, resources, edges, properties, and blocks — and nothing about what they're
    <em>for</em>. The workflow and knowledge-base mechanics Temper ships — documents, contexts,
    sync — are <strong>conventions</strong> expressed over that kernel, not structure carved into
    it. The cognitive map is another such convention: a charter is a resource carrying a particular
    <code>doc_type</code> property; a regulation is a resource reached by a particular edge. Neither
    is new machinery beneath the kernel — both are <em>agreements about how to read it</em>.
  </p>
  <p>
    That separation is what keeps the model open. Because the schema commits to the kernel and not
    to today's use cases, it can express patterns we haven't reached for yet, while convention
    steers the common ones toward shared shapes — intention expressed, not enforced by the table
    layout. The cognitive map gets to be event-sourced without paying for it twice: it isn't bolted
    onto the substrate, it's a reading of it.
  </p>
</Section>

<Section label="What this buys">
  <p>
    An append-only ledger costs something to maintain, and what it buys is everything that comes
    later. Because the trace of <em>how</em> a thing came to be is never thrown away, supersession
    can leave a <a href="/cognitive-maps/how-a-map-grows">scar</a>, a fold can preserve instead of
    delete, and the question the second engineer asked — <em>why does the map say this?</em> —
    always has an answer the system can compute, across system boundaries even, which the
    <a href="/cognitive-maps/operating-temper/insights">insights</a> story returns to. The honesty
    about its own history that <code>/theory</code> asked for isn't a feature added on top; it's
    what's left when nothing is overwritten.
  </p>
</Section>

<Section label="An open seam">
  <p>
    One modelling call we made here and are watching: the event's producing anchor is kept as
    <strong>provenance, and left nullable</strong>. Two reasons sit behind that. First, every homed
    object (an edge, a region) already carries its own access anchor, so the event doesn't re-decide
    access — it records <em>where the act happened</em> and lets the object's own home gate the
    read. Second, not every event is born inside a map. An integration like a webhook emits a
    <strong>pure data event</strong> with no cogmap to anchor to yet — raw signal, until a triage
    agent picks it up and works it for salience within some map's telos. A nullable anchor is what
    lets unowned, external events exist on the ledger before any map has claimed them, which is
    exactly the shape the integration story (later in this set) leans on. Whether the same modelling
    holds for every event family is the kind of thing the empirical load of the artifact exists to
    surface.
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
