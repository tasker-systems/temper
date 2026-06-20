<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
</script>

<svelte:head>
  <title>Operating Temper — temper</title>
  <meta
    name="description"
    content="What do I run, what do I get out of the box, and what's still mine to decide? Operating Temper answers the cold evaluator's question against a three-tier confidence ledger: what the architecture fixes (proven, not promised), what's extensible by design with the edges still being found, and what's genuinely yours."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/operating</div>
  <h1 class="t-hero-title">Operating <em>Temper</em></h1>
  <p class="tagline t-tagline">
    What the architecture fixes, what a deployment shapes, and what stays yours.
  </p>
</section>

<div class="operating-page">

<blockquote class="epigraph">
  Running Temper raises a practical question — what do I run, what do I get out
  of the box, and what's still mine to decide? This page answers it from a cold
  start: no conceptual walkthrough assumed, with links back where a term needs
  its grounding. The short version is one sentence: the 0→1 is invariant, and
  everything after it is shaped.
</blockquote>

<Section label="Three tiers, kept separate">
  <p>
    Rather than a flat "it's all handled," everything below sorts into three
    tiers kept visibly apart: what the architecture <strong>fixes</strong>, what
    it makes <strong>extensible</strong> (with the edges still being found), and
    what is <strong>genuinely yours</strong>. An evaluator is better served by
    that separation than by a claim that blurs them — and keeping them apart is
    exactly what makes the rest of this trustworthy. The same separation runs
    through the four dimension pages this hub leads into.
  </p>
  <p>
    The vocabulary — telos, cognitive map, an event-sourced substrate — is
    grounded in <a href="/cognitive-maps">cognitive maps</a> (the concrete on-ramp)
    and the commitments underneath in <a href="/theory">theory</a> (the why). This
    page assumes them and points back rather than retelling them.
  </p>
</Section>

<Section label="Tier 1 — Fixed by the architecture">
  <p>
    Invariant across every deployment, proven in the artifact and
    replay-verified. This is the strongest tier: <strong>proven, not
    promised.</strong> Adopt Temper and you adopt all of it.
  </p>
  <ul>
    <li>The <strong>event-primary ledger</strong> — every change is an event, and the current state is a projection of them.</li>
    <li>The <strong>convention-agnostic kernel</strong> — it never branches on a deployment's conventions.</li>
    <li><strong>Teams-RBAC over homed boundaries</strong> — access is scoped through team grants over owned regions.</li>
    <li><strong>Actors as entities</strong> — every writer, agent or integration, is an entity meeting the same event shape.</li>
    <li><strong>Administration is event-sourced</strong> — governance acts are events, so the audit exists by construction.</li>
    <li>The <strong>invocation envelope</strong> — an accountability-grain run with a telos/scope binding and a terminal outcome.</li>
    <li>The <strong>delegation launch-gate</strong> — a delegated run is authorized before it can act, never after.</li>
    <li><strong>Authorship invisible to affinity</strong> — who wrote a thing never tilts how concepts relate.</li>
  </ul>
</Section>

<Section label="Tier 2 — Extensible by design, edges still being found">
  <p>
    The runtime choice is localized to a thin contract the substrate never reads:
    the <code>temper-agents</code> crate depends on the substrate, never the
    reverse, and the kernel never branches on which runtime woke an agent. The
    <em>shape</em> of the delegation problem has held across the platforms modeled
    so far — Vercel's Eve durable agents and Claude Managed Agents are both
    first-class runtime bindings, and runtime and residency stay orthogonal axes
    rather than tangling together.
  </p>
  <p>
    So the claim is <strong>extensible shape with partial neutrality</strong>, not
    a flat "runtime-neutral." The seam, named plainly: those runtimes are enum
    variants in the contract, so adding one today is a <em>patch to the contract
    crate</em>, not a line of configuration — a deliberate refusal to abstract too
    early. What becomes pure configuration versus what the substrate must model is
    still being settled by running on real runtimes, and that line is expected to
    move as we learn. A live research front, not a permanent limitation — walked
    through under <a href="/operating/deployment">deployment</a>, and the same
    honest seam is the forward-exciting note <a href="/operating/insights">insights</a>
    closes on.
  </p>
</Section>

<Section label="Tier 3 — Genuinely yours">
  <p>
    The deployment chooses, and this page does not pretend otherwise. None of
    these has a single right answer the project could publish for you:
  </p>
  <ul>
    <li>Which <strong>runtime</strong> your agents run on, and the <strong>residency</strong> of where their tools execute.</li>
    <li>The <strong>token budget</strong> a delegated run is paced against.</li>
    <li>The <strong>tenancy model</strong> — single-tenant, or many internal sub-tenants with per-tenant boundaries.</li>
    <li><strong>Per-tenant integration</strong> — which systems write the ledger, for which tenant.</li>
    <li>The <strong>trigger cadence</strong> that wakes an agent — event volume, a clock, salience past a floor.</li>
    <li>The <strong>observability scope</strong> — which metrics matter, what you alert on and retain.</li>
    <li>How <strong>guarded</strong> the administrative surface must be — from a thin operator surface to a separated, audited, SAML-backed plane.</li>
  </ul>
</Section>

<Section label="Reading the tiers honestly">
  <p>
    Where this page can only point at a finding and a contract rather than a
    shipped fact, it says so — tier 2 is named as tier 2, never quietly promoted
    to tier 1. The unblurred version is the one worth holding to: <em>this is
    invariant; this is what we happen to run, and you can swap it; this is
    genuinely yours.</em>
  </p>
</Section>

<Section label="The four dimensions you'll shape">
  <p>
    The operating story splits four ways. Each is a dimension a deployment shapes,
    and each has a part the architecture fixes and a part that's yours:
  </p>
  <ul>
    <li>
      <strong><a href="/operating/deployment">Deployment</a></strong> — topology,
      tenancy, per-tenant integration, and where agents run. The dimension where
      one organization's Temper diverges most from another's, with the runtime
      seam stated as a seam.
    </li>
    <li>
      <strong><a href="/operating/governance-and-administration">Governance &amp; administration</a></strong>
      — who may author a map versus who may reshape the access graph beneath every
      map, how guarded that second power must be, and why administration is
      auditable by construction.
    </li>
    <li>
      <strong><a href="/operating/observability-and-audit">Observability &amp; audit</a></strong>
      — is the system healthy, and what did it know and why did it change: two
      questions, two homes, with the responsibility boundary named.
    </li>
    <li>
      <strong><a href="/operating/insights">Insights</a></strong> — what becomes
      possible once agents leave correlated, reasoned traces: a queryable
      provenance of how a shared understanding formed. The forward-looking close.
    </li>
  </ul>
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
  :global(.operating-page .epigraph) {
    border-left: 2px solid var(--temper-blue-border);
    font-family: var(--font-serif);
    font-style: italic;
    color: var(--parchment);
    line-height: 1.7;
  }
</style>
