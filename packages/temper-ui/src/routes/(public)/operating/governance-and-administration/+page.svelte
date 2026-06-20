<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
  import VizFigure from '$lib/components/VizFigure.svelte';
  import AdminFirewallDiagram from '$lib/components/cognitive-maps/diagrams/AdminFirewallDiagram.svelte';
</script>

<svelte:head>
  <title>Governance & administration — temper</title>
  <meta
    name="description"
    content="Authoring a map and reshaping the access graph beneath every map are different powers. Authoring is built; the administrative surface is org-shaped — how guarded it must be varies by organization. Administration is event-sourced (auditable by construction), with two deliberate boundaries."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/operating/governance-and-administration</div>
  <h1 class="t-hero-title">Governance &amp; <em>administration</em></h1>
  <p class="tagline t-tagline">
    Authoring is built; the admin surface is org-shaped.
  </p>
</section>

<div class="operating-page">

<blockquote class="epigraph">
  Someone maintains one team; someone else owns another. Making those things
  true — adding a person to a team, creating the team, joining it into the right
  place — isn't the same act as <em>authoring a map</em>, and how guarded it
  needs to be is one of the most organization-shaped decisions here. What
  <em>is</em> settled: every one of those administrative acts is an event, on the
  ledger, auditable by construction.
</blockquote>

<Section label="Two different powers">
  <p>
    Look closely at what it takes to set an organization up, and two distinct
    powers come apart.
  </p>
  <p>
    One is <strong>authoring</strong> — bringing a telos and its map into being.
    That's built and invariant: <code>cogmap_genesis</code>, reachable over MCP,
    is the act that brings a map into being. Authoring is creative and
    relatively safe; the worst a bad map does is exist until it's folded.
  </p>
  <p>
    The other is <strong>administration</strong> — adding a person to a team,
    creating a team, joining a team to a map, disabling a profile.
    These reshape <em>who can see what</em> across the whole system. They
    shouldn't share a surface with authoring: the power to create a map and the
    power to rewrite the access graph beneath every map differ in kind, and the
    second wants a different, more deliberate door.
  </p>
</Section>

<Section label="How guarded is your call">
  <p>
    Authoring writes <em>inside</em> the boundaries that already exist.
    Administration <em>moves the boundaries</em>. A mistaken map is local and
    recoverable; a mistaken grant — a team joined to a map it shouldn't reach, a
    profile enabled that shouldn't be — changes what everyone in its shadow can
    read. So the administrative surface wants to be guarded — and <em>how</em>
    guarded is where the organization decides.
  </p>
  <p>
    A small, trusted team running its own deployment may be fine with a thin
    admin surface where the operators are the administrators. A regulated
    enterprise wants the opposite: a separated, heavily-audited plane, with
    approvals, with enterprise identity behind it. temperkb.io sits at the
    minimal end — single-tenant, no separate administrative plane to speak of,
    the operators <em>are</em> the admins. Your deployment chooses where on that
    spectrum it needs to be, and moves as it grows.
  </p>
  <p>
    That choice rides an <strong>authentication fork</strong>. Temper already
    integrates OAuth (Auth0 / Okta) for who-you-are. Administration raises
    whether your organization needs <strong>SAML over and above</strong> that —
    enterprise identity, group mapping, the assurances a security team asks for
    before it will put real org structure into a system. Some deployments need
    it on day one; others never do.
  </p>
  <p>
    What the administrative surface must <em>do</em> is steady across all of
    that: provision profiles (human and agent alike), create and disable teams,
    place teams in the DAG, join and remove teams from maps. What it looks like —
    how separated, how audited, how authenticated — is yours.
  </p>
</Section>

<Section label="What administration is, on the ledger">
  <p>
    Here's the part that's settled rather than open. Administrative acts are
    <strong>events</strong> — creating a team, granting a team to a map, each
    with an emitter and a producing anchor, exactly like every other change in
    the system. So governance is auditable <em>by construction</em>: every "who
    granted whom access to what, and when" is already on the ledger, no separate
    audit log to bolt on.
  </p>
  <p>Two boundaries make this precise, and they're deliberate:</p>
  <ul>
    <li>
      <strong>Governance is traceable, but it isn't knowledge.</strong>
      Administrative events are privacy- and auth-bound records, kept for
      <strong>compliance</strong>. By design they do <strong>not</strong>
      participate in cognitive maps, subscriptions, or resource relationships —
      a grant is not a concept, and the agents growing maps never see the
      governance stream as material to reason over. The two live on the same
      ledger, firewalled by intent.
    </li>
    <li>
      <strong>The ledger stops at the persistence layer.</strong> A command
      issued straight to Postgres can bypass the event stream entirely. That's
      not a hole in the audit — it's a <strong>system-responsibility
      boundary</strong>: below the application, you're in the domain of database
      controls and infrastructure policy, not Temper's ledger. (The same line is
      drawn from the other side in
      <a href="/operating/observability-and-audit">observability &amp; audit</a>.)
    </li>
  </ul>
</Section>

<VizFigure placement="INLINE" fidelity="conceptual">
  {#snippet diagram()}
    <AdminFirewallDiagram id="cm-admin" />
  {/snippet}
  {#snippet shows()}
    two surfaces over one ledger. <strong>Authoring</strong> (left): an MCP call
    into <code>cogmap_genesis</code> producing a new map <em>inside</em> existing
    boundaries — built / solid. <strong>The administrative surface</strong>
    (right): operations on the access graph — add profile to team, create /
    disable team, place team in the DAG, join / remove team↔map — drawn as
    organization-shaped (a dial from a thin operator surface to a separated,
    audited, SAML-backed plane). Both write <strong>events</strong> to the
    ledger, but the administrative events flow into a <strong>firewalled
    compliance-audit stream</strong> (drawn as a separate channel that does
    <em>not</em> feed the cognitive maps / subscriptions). A dashed line at the
    bottom marks the <strong>Postgres responsibility boundary</strong>, below
    which commands can bypass the ledger.
  {/snippet}
  {#snippet honestBasis()}
    authoring is real (<code>cogmap_genesis</code>); the graph it administers is
    real (<code>kb_profiles</code>, <code>kb_teams</code>,
    <code>kb_teams_parents</code>, <code>kb_team_members</code>,
    <code>kb_team_cogmaps</code>); the event ledger and producing-anchor shape
    that admin events would use are real (<code>kb_events</code>). The
    <strong>administrative surface itself is unbuilt</strong> and its
    <em>shape</em> is organization-specific — drawn as a proposed dial.
  {/snippet}
</VizFigure>

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
