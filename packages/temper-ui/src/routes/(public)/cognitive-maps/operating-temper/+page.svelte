<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
</script>

<svelte:head>
  <title>Operating Temper — temper</title>
  <meta
    name="description"
    content="The closing turn of the cognitive-maps journey: the architecture is only half of a running system, and the map had to be stood up somewhere. A good problem — walked through in full, for the cold evaluator, under /operating."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/cognitive-maps/operating-temper</div>
  <h1 class="t-hero-title">Operating <em>Temper</em></h1>
  <p class="tagline t-tagline">
    The architecture is only half of a running system — and that's a good problem.
  </p>
</section>

<div class="cognitive-maps-page">

<blockquote class="epigraph">
  Everything so far showed an architecture whose shape is proven. Operating it is
  a different kind of question, because the architecture is only half of a running
  system. The other half is the shape a particular organization gives it — its
  topology, its tenancy, its agents, its rules — and that shape varies between
  organizations and shifts over time. The public deployment you may be reading
  this on is one such shape, and a deliberately small one.
</blockquote>

<Section label="Someone had to stand it up">
  <p>
    Go back to the onboarding map one last time and ask the question the show
    pages stepped over: how did it come to exist <em>at all</em>?
  </p>
  <p>
    Something had to be running first. A Postgres instance. Temper itself, serving
    its CLI, API, and MCP. Dave provisioned, org-common created, a webhook wired,
    and a threshold that woke <code>onboarding-agent#1</code> for the first time.
    None of that appears in the story the show pages told, and all of it had to
    happen for the story to be possible. And "something had to be running" hides a
    choice: <em>running how, and where?</em>
  </p>
  <p>
    That's the turn outward — from a map whose shape is proven to the good problem
    of standing one up and keeping it running. It's a different reader's question,
    too: not the journeyer who came to understand a cognitive map, but the
    evaluator who arrives cold to ask what they'd run, what they get out of the
    box, and what stays theirs to decide.
  </p>
</Section>

<Section label="Walked through under /operating">
  <p>
    So the operating story has its own home now — told from that cold start rather
    than carried as the tail of this journey. <strong><a href="/operating">Operating
    Temper</a></strong> sorts it against a three-tier confidence ledger — what the
    architecture fixes, what's extensible with the edges still being found, and
    what's genuinely yours — and splits four ways:
  </p>
  <ul>
    <li><strong><a href="/operating/deployment">Deployment</a></strong> — topology, tenancy, per-tenant integration, and where agents run.</li>
    <li><strong><a href="/operating/governance-and-administration">Governance &amp; administration</a></strong> — authoring versus reshaping the access graph, and how guarded that must be.</li>
    <li><strong><a href="/operating/observability-and-audit">Observability &amp; audit</a></strong> — is it healthy, and what did it know and why did it change.</li>
    <li><strong><a href="/operating/insights">Insights</a></strong> — the queryable provenance of how a shared understanding formed. The forward-looking close.</li>
  </ul>
  <p>
    That's the invitation, in the end: not only to help build a system that grows
    understanding, but to help run it where the operating shape is yours to choose.
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
