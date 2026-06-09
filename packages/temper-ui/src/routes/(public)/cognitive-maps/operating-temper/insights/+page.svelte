<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
  import VizFigure from '$lib/components/VizFigure.svelte';
  import ProvenanceChainDiagram from '$lib/components/cognitive-maps/diagrams/ProvenanceChainDiagram.svelte';
</script>

<svelte:head>
  <title>Insights — temper</title>
  <meta
    name="description"
    content="A PR merged on one team; minutes later a map had new regulation — and the causal chain between them is recorded and queryable, across system boundaries. Analytics is the how; the insight is the why-it-matters — a provenance of how understanding formed. What you'd ask of it is organization-shaped; the forward-exciting close to the set."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/cognitive-maps/operating-temper/insights</div>
  <h1 class="t-hero-title">Insights</h1>
  <p class="tagline t-tagline">
    The exhaust from running it is one of the more interesting things it makes.
  </p>
</section>

<div class="cognitive-maps-page">

<blockquote class="epigraph">
  A PR merged on team-a. Minutes later, the onboarding map had a new piece of
  regulation. Those two facts are connected — and the connection is
  <em>recorded</em>. You can follow it: the merge woke a triage agent, the agent
  reasoned about it, the reasoning changed a concept, the change reinforced a
  charter question. The whole causal chain, across system boundaries, is
  queryable. This is where running Temper stops being a cost and starts being a
  payoff.
</blockquote>

<Section label="Start with a question you usually can't ask">
  <p>
    In most systems, <em>"why did our shared understanding of onboarding shift
    this week?"</em> has no answer you can compute. The change is somewhere in a
    chat log, a person's memory, a commit nobody connected to it. Temper's
    exhaust makes the question answerable.
  </p>
  <p>
    Here's the chain, concretely. A webhook event arrives — <em>PR #123
    merged</em> — carrying a correlation id. A triage agent watching that topic
    wakes. It reads the onboarding charter, decides the merge bears on
    <em>"where are the sharp edges that scar newcomers,"</em> and emits a
    mutation event — <strong>with its reasoning in the payload</strong> — that
    writes a new regulation and reinforces that question. Every one of those
    events shares the correlation thread back to the original merge.
  </p>
  <p>
    So the trace exists end to end: <strong>PR merged → triage agent woke →
    concept mutated, with this reasoning → charter question reinforced</strong>
    — one correlated causal chain, crossing from a remote system into the
    cognitive substrate. Not a log you assemble afterward. A graph the system
    already holds.
  </p>
</Section>

<Section label="Analytics, and the insight beneath it">
  <p>
    Two layers sit here, and they aren't the same kind of thing.
    <strong>Analytics</strong> is the <em>how</em> — the metrics any running
    system can produce. <strong>Insight</strong> is the <em>why it matters</em>
    — what those traces let you understand about your own thinking.
  </p>
  <p>
    The analytics are what you'd assume: resource and event lifecycle metrics,
    how maps grow, which concepts churn, where attention concentrates. Useful,
    ordinary, and good to have — but not the reason this page closes the set.
  </p>
  <p>
    The insight is the chain above — the <strong>provenance graph of how
    understanding formed.</strong> Because agents leave their reasoning in the
    events they emit, and because correlation ties those events back across
    integration boundaries to the remote acts that triggered them, you can query
    not only <em>what</em> the system believes but <em>how</em> it came to
    believe it, step by reasoned step, all the way out to a merge in someone's
    repository. The provenance of a thought, made queryable.
  </p>
  <p>
    One thing this graph is <em>not</em> about: governance. The
    reasoning-provenance is a <strong>cognitive</strong> trail — how a shared
    understanding formed. The record of who was granted what access lives on the
    same ledger but in a separate, firewalled stream (that separation is drawn
    in
    <a href="/cognitive-maps/operating-temper/governance-and-administration"
      >governance &amp; administration</a
    >). The insight here is about thought, not administration; the two don't
    bleed into each other.
  </p>
</Section>

<VizFigure placement="HERO" fidelity="conceptual / illustrative">
  {#snippet diagram()}
    <ProvenanceChainDiagram id="cm-provenance" />
  {/snippet}
  {#snippet shows()}
    a single causal chain drawn left-to-right across a <strong>system
    boundary</strong>. On the far left, <em>outside</em> Temper: a remote act —
    <strong>PR #123 merged</strong> on GitHub. It crosses the boundary as a
    webhook <strong>event carrying a correlation id</strong>. Inside: the triage
    <strong>agent wakes</strong>, reads the charter, and emits a
    <strong>mutation event with its reasoning in the payload</strong>, which
    <strong>writes a regulation</strong> and <strong>reinforces a charter
    question</strong>. Every node shares the same correlation thread, drawn as a
    connecting spine. The reader should see one unbroken, queryable line from a
    merge in a repo to a shift in a map's understanding.
  {/snippet}
  {#snippet honestBasis()}
    the threading is real: <code>kb_events.correlation_id</code>,
    <code>emitter_entity_id</code>, and the open <code>metadata jsonb</code> that
    can carry an agent's reasoning; the reinforcement effect is the provenance
    accretion behind <code>resource_blocks</code>' <code>reinforce_count</code>.
    The <strong>cross-system query and the assembled provenance graph are
    proposed</strong> — the columns exist to support them; the analytics layer
    that reads them does not yet. Draw the chain as real, the dashboard around it
    as proposed.
  {/snippet}
</VizFigure>

<Section label="What's yours to ask">
  <p>
    The capability is the architecture's; the questions are yours. What to
    trace, which provenance chains repay the attention, what a dashboard over
    this should even show — those depend on what your organization runs and what
    it's trying to learn about itself. temperkb.io doesn't build this layer
    today; a deployment that cares about how its understanding evolves would. The
    substrate holds the threads either way; pulling them is a choice each
    organization makes for itself.
  </p>
</Section>

<Section label="Why this is the closing note">
  <p>
    This is where the whole design pays off in a single direction. Event-primary
    made every change answerable. Homed boundaries kept the answers honest.
    Agents-as-entities let outside systems write the same ledger. Correlation
    threaded it all together. Not one of those choices was made <em>for</em>
    insight — and yet together they produce something most systems can't: a
    queryable, reasoned account of how a shared understanding came to be what it
    is.
  </p>
  <p>
    That's the invitation, in the end. Not only to help build a system that
    grows understanding — but to help build one that can <em>show its work</em>,
    and then to run it where that work is yours to read.
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
