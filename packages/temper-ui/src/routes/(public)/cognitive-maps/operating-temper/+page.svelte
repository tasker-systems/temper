<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
</script>

<svelte:head>
  <title>Operating Temper — temper</title>
  <meta
    name="description"
    content="The architecture is only half of a running system; the other half is the shape an organization gives it, which varies between organizations and evolves over time. What temper-next fixes vs. what a deployment shapes, temperkb.io as one near-minimal point on the range, and the decisions a private deployment comes to own."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/cognitive-maps/operating-temper</div>
  <h1 class="t-hero-title">Operating <em>Temper</em></h1>
  <p class="tagline t-tagline">
    The architecture is only half of a running system.
  </p>
</section>

<div class="cognitive-maps-page">

<blockquote class="epigraph">
  Everything so far showed an architecture whose shape is proven — help us finish
  it. Operating it is a different kind of question, because the architecture is
  only half of a running system. The other half is the shape a particular
  organization gives it — its topology, its tenancy, its agents, its rules — and
  that shape varies between organizations and shifts over time. The public
  deployment you may be reading this on is one such shape, and a deliberately
  small one.
</blockquote>

<Section label="Someone had to stand it up">
  <p>
    We go back to the onboarding map one more time and ask the question the show
    pages stepped over: how did it come to exist <em>at all</em>?
  </p>
  <p>
    Something had to be running first. A Postgres instance. Temper itself,
    serving its CLI, API, and MCP. Dave provisioned, org-common created, a
    webhook wired, and a threshold that woke <code>onboarding-agent#1</code> for
    the first time. None of that appears in the story the show pages told, and
    all of it had to happen for the story to be possible.
  </p>
  <p>
    But "something had to be running" hides a choice: <em>running how, and
    where?</em> On the public deployment that's Vercel functions over a Neon
    database; in your organization it might be containers in a cluster, a
    database you operate, agents on a platform you pick. The cognitive map looks
    the same from inside either way. Underneath, the operating shape is a
    decision — and mostly distinct by the needs of where it is being deployed and
    for whom.
  </p>
</Section>

<Section label="What's fixed, and what's yours">
  <p>
    It helps to pull apart two layers the word "Temper" runs together.
  </p>
  <p>
    The <strong>architecture</strong> — what these pages have described as
    temper-next — fixes a specific set of things, and they don't vary by
    deployment: events are primary, the kernel is convention-agnostic, access is
    teams-RBAC over homed boundaries, actors are entities, and every writer
    (agent or integration) meets the same event shape. Adopt Temper and you
    adopt those.
  </p>
  <p>
    The <strong>operating shape</strong> is everything else, and it's a
    <em>range</em>, not a point. The architecture has a minimum viable form —
    small, single-tenant, serverless — and a much larger one — multi-tenant,
    per-tenant integrations, dedicated agent infrastructure, deep observability.
    Every real deployment sits somewhere on that range and moves along it as the
    organization grows. So the pages here <em>invite</em> in two senses: help us,
    the project, refine the architecture and the range it admits — and, when you
    run Temper privately, own these operational decisions yourself, revisiting
    them as your needs change. The questions are real and the mechanisms mostly
    known; the answers are shaped by how your organization needs to run.
  </p>
</Section>

<Section label="temperkb.io is one point on the range">
  <p>
    A concrete example, since it's probably in front of you. <code>temperkb.io</code>,
    the public deployment, is one shape — and a near-minimal one. It runs on
    Vercel serverless functions over a Neon database, routed edge functions
    rather than containers in a cluster. It's single-tenant: it isn't set up
    today for the multitenant choices, or the per-tenant webhook subscriptions,
    that a private organizational deployment would want. Its agents run on the
    mechanisms Vercel offers, which are not the same as a dedicated managed-agent
    platform. (It's also the <em>current</em> public version, while temper-next —
    the architecture you've been reading — is the destination; even its own shape
    is one option among the range temper-next opens.) None of that is
    specifically a flaw. It's a <em>choice of shape</em>, near the small end of
    what the architecture allows, and a useful picture of what a minimum looks
    like. Your deployment gets to choose differently, and to change its mind
    later.
  </p>
</Section>

<Section label="Four dimensions you'll shape">
  <p>
    The operating story splits four ways, each a dimension a deployment shapes —
    and each with its own texture:
  </p>
  <ul>
    <li>
      <strong><a href="/cognitive-maps/operating-temper/deployment">Deployment</a></strong>
      — topology, tenancy, and how new maps, integrations, and agents come
      online. The dimension where one organization's Temper diverges most from
      another's.
    </li>
    <li>
      <strong><a href="/cognitive-maps/operating-temper/governance-and-administration">Governance &amp; administration</a></strong>
      — who may create a map, and who may reshape the teams and grants beneath
      it. How guarded that second power must be depends on the organization.
    </li>
    <li>
      <strong><a href="/cognitive-maps/operating-temper/observability-and-audit">Observability &amp; audit</a></strong>
      — how an operator sees the system is healthy, plus the audit the ledger
      gives for free. Which metrics matter is an organizational call.
    </li>
    <li>
      <strong><a href="/cognitive-maps/operating-temper/insights">Insights</a></strong>
      — what becomes <em>possible</em> once agents leave correlated, reasoned
      traces. There's a payoff hiding in the operating layer — the exhaust from
      running Temper is one of the more interesting things it produces — and what
      you'd ask of it varies with what you're running. The forward-looking close.
    </li>
  </ul>
</Section>

<Section label="The decisions, and who owns them">
  <p>
    Three decisions cut across those dimensions — two still open, and one we've
    settled and would rather state plainly than leave you to guess. Each has a
    part the architecture fixes and a part a deployment shapes:
  </p>
  <ol>
    <li>
      <strong>The event-shape data contract</strong> <em>(open).</em> <em>The
      architecture's part:</em> every external writer is an entity and every
      event meets a shared shape — fixed. <em>Your part:</em> which integrations
      you wire in, what their events carry, how much raw signal you admit before
      an agent makes sense of it. This is the boundary where Temper becomes
      infrastructure your other systems emit into. (Deployment goes deeper.)
    </li>
    <li>
      <strong>Administration is event-sourced</strong> <em>(settled, with
      boundaries).</em> Creating a team, granting a team to a map — these are
      <em>events</em>, with an emitter and a producing anchor, so governance is
      auditable by construction. Two deliberate limits, though. They're privacy-
      and auth-bound records kept for <strong>compliance</strong>, and by design
      they do <strong>not</strong> participate in cognitive maps, subscriptions,
      or resource relationships — governance is traceable, but it isn't
      knowledge. And they stop at the persistence layer: a command issued
      straight to Postgres can bypass the ledger, which is a
      system-responsibility boundary, not a gap. (Governance and audit carry
      this.)
    </li>
    <li>
      <strong>What wakes an agent</strong> <em>(open, and mostly yours).</em>
      Event volume, a cadence, salience crossing a floor — the rhythm of waking
      and sweeping a map (the <em>temper-system dreaming</em> we keep naming)
      depends on your traffic and your tolerances, and you'll re-tune it over
      time. (Deployment, again.)
    </li>
  </ol>
</Section>

<Section label="One thing we're not pretending">
  <p>
    A straight answer for a security-minded reader, so trust isn't lost later:
    <strong>v1 assumes good-faith actors.</strong> We name that as a bracket
    rather than hide it, because it's real — the features that make a knowledge
    substrate good at <em>discoverability</em> (surfacing the right concept to
    the right reader at the right moment) are close to isomorphic with the
    features that make it good at <em>reconnaissance</em>. The RBAC and
    homed-boundary work genuinely gates access; what v1 does not yet model is an
    actor working <em>against</em> the system from inside its good-faith
    assumptions. How much that matters is itself partly a deployment question — a
    trusted internal team is a different threat surface than an open one — but if
    your context makes that adversary real, it's a conversation we want early,
    not a surprise you find later.
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
