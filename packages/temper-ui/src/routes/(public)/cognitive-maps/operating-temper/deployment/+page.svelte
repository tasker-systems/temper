<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
  import VizFigure from '$lib/components/VizFigure.svelte';
  import BootstrapRangeDiagram from '$lib/components/cognitive-maps/diagrams/BootstrapRangeDiagram.svelte';
</script>

<svelte:head>
  <title>Deployment — temper</title>
  <meta
    name="description"
    content="The 0→1 bootstrap is the seed file itself and doesn't vary; everything after — topology, tenancy, per-tenant integration, where agents run — is where one organization's deployment diverges most from another's. temperkb.io is one near-minimal shape; a private deployment chooses its own."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/cognitive-maps/operating-temper/deployment</div>
  <h1 class="t-hero-title">Deployment</h1>
  <p class="tagline t-tagline">
    The seed doesn't vary; everything after is yours to shape.
  </p>
</section>

<div class="cognitive-maps-page">

<blockquote class="epigraph">
  The very first thing that exists is the seed file — <code>temper-system</code> and a
  <code>system-default</code> map, the floor everything stands on. That part is the same
  everywhere. After it, getting to one map and then many is a path every deployment walks —
  but <em>how</em> it walks it, and on what infrastructure, is where one organization's Temper
  diverges most sharply from another's.
</blockquote>

<Section label="Zero to one">
  <p>
    Standing up the first Temper is a short list with one elegant property at the end.
  </p>
  <p>
    A Postgres instance, with the schema loaded. Temper itself, serving its three surfaces — the
    CLI, the API, and the MCP server that agents and integrations speak to. And then the first
    telos, seeded.
  </p>
  <p>
    The elegant property: that first seed isn't a special bootstrap script. It's the same
    <code>temper-system</code> root team and <code>system-default</code> map the seed file already
    creates — the public floor every later team descends from and every enabled profile joins. The
    0→1 picture is literally the seed you've been reading; the system's first act is to describe
    itself in its own terms. That much is invariant — it looks identical on every deployment.
  </p>
  <p>
    What's <em>already</em> a choice is the substrate it loads into. The public deployment runs
    "Postgres + Temper serving its surfaces" as Vercel functions over a Neon database; a private
    deployment might run it as containers over a database it operates. The seed is the same; the
    ground it lands on is yours to pick.
  </p>
</Section>

<Section label="One to many">
  <p>
    Growth from there has three motions, visible already in the cast:
  </p>
  <ul>
    <li>
      <strong>New maps, by authoring.</strong> A telos and its charter come into being through
      <code>cogmap_genesis</code>, reachable over MCP — exactly how the onboarding map was born. A
      solved, callable act.
    </li>
    <li>
      <strong>Raw events, by integration.</strong> A GitHub webhook writes events into the ledger
      as they happen — a PR merged, an issue closed — as a pure data stream, no map attached yet.
      The integration is an entity, the same kind of actor as an agent.
    </li>
    <li>
      <strong>Attention, by agents.</strong> Agents wake on some signal, read the maps they're
      bound to, and do the growing — triage, regulation, the five learning-acts.
    </li>
  </ul>
  <p>
    The first motion is invariant — it's a function you call. The second and third are where the
    shape of <em>your</em> deployment starts to matter.
  </p>
</Section>

<Section label="Where deployments diverge">
  <p>
    This is the dimension that varies most between organizations, so here are the axes concretely —
    using the public deployment as the near-minimal reference point:
  </p>
  <ul>
    <li>
      <strong>Topology.</strong> Serverless edge functions (temperkb.io: Vercel) versus containers
      in a cluster. The architecture runs on either; the operational properties — scaling, cold
      starts, where long-running agents live — differ, and so does what your organization already
      knows how to operate.
    </li>
    <li>
      <strong>Tenancy.</strong> temperkb.io is effectively single-tenant. A private deployment
      usually asks the opposite question: one organization, yes, but often many internal
      sub-tenants — divisions, customers, environments — with the isolation and per-tenant data
      boundaries that single-tenant temperkb.io doesn't draw today.
    </li>
    <li>
      <strong>Per-tenant integration.</strong> Webhook subscription <em>by tenant</em> — so each
      tenant's GitHub or Notion feeds only its own maps — is something a multi-tenant private
      deployment needs and the public one doesn't currently do. The event-shape contract makes it
      possible; wiring it per tenant is a deployment's own work.
    </li>
    <li>
      <strong>Agent infrastructure.</strong> Where the waking agents actually run. Vercel offers
      agent mechanisms; a dedicated managed-agent platform (Anthropic's, for instance) is not
      identical, and a private deployment may choose differently again, or run its own. The agent
      is an entity either way; <em>where it executes</em> is yours.
    </li>
  </ul>
  <p>
    None of these has a single right answer the project could publish for you. They're the shape
    you choose on the way in, and re-choose as you grow.
  </p>
</Section>

<VizFigure placement="HERO" fidelity="conceptual / illustrative">
  {#snippet diagram()}
    <BootstrapRangeDiagram id="cm-bootstrap" />
  {/snippet}
  {#snippet shows()}
    two layers. (1) The <strong>invariant path</strong>: a phased timeline — empty Postgres →
    schema loaded → Temper serving CLI / API / MCP → first seed (labelled <em>"this is the seed
    file"</em>) → 1→N (a new map via authoring, a webhook stream arriving, an agent waking). (2)
    Around it, the <strong>deployment-shape range</strong>: the same path drawn at two points — a
    <em>near-minimal</em> shape (serverless / Neon / single-tenant / platform agents, tagged
    "temperkb.io") and a <em>fuller</em> shape (cluster / operated DB / multi-tenant + per-tenant
    webhooks / dedicated agent infra). The reader should see that the bootstrap is identical
    everywhere, while the topology, tenancy, integration, and agent-execution choices slide along a
    range.
  {/snippet}
  {#snippet honestBasis()}
    the 0→1 floor is real: <code>temper-system</code> + <code>system-default</code> seed;
    authoring is <code>cogmap_genesis</code>; integrations-as-entities is
    <code>kb_entities</code> + <code>emitter_entity_id NOT NULL</code> + <code>correlation_id</code>;
    <code>kb_topics</code> seeds topic bounds. The <strong>topology / tenancy /
    per-tenant-subscription / agent-platform choices are operational, not in the artifact</strong> —
    drawn as a range, with the temperkb.io point annotated from its actual stack (Vercel functions,
    Neon, single-tenant). The contract and trigger mechanisms are designed, not yet built — drawn as
    proposed.
  {/snippet}
</VizFigure>

<Section label="The standing machinery">
  <p>
    For the integration and agent motions to run continuously, a few things have to be in place —
    and each is where a tenancy or platform choice lands:
  </p>
  <ul>
    <li>
      <strong>Integrations as ledger writers.</strong> An outside system is an entity with
      permission to append events, and its events satisfy a shared shape — an emitter, a type, a
      correlation thread — so a triage agent can pick them up later. That shape is the
      <strong>event-shape data contract</strong> (below); <em>which</em> systems write, and
      <em>for which tenant</em>, is the per-tenant integration choice above.
    </li>
    <li>
      <strong>Triggers for agent sessions.</strong> Something decides when an agent wakes — the
      <strong>trigger-threshold</strong> fork.
    </li>
    <li>
      <strong>Topic bounds for subscription.</strong> An agent watching for events needs a formal
      way to say <em>which</em> events it cares about — a topic scope it subscribes to — so a busy
      ledger doesn't wake everything for everything. In a multi-tenant deployment, that scope is
      also how a tenant's agents stay bound to a tenant's events.
    </li>
  </ul>
</Section>

<Section label="The forks here">
  <p>
    <strong>The event-shape data contract.</strong> This is the boundary where Temper becomes
    infrastructure your other systems emit into. The architecture fixes its floor — every external
    writer is an entity, <code>emitter_entity_id</code> is never null, so a GitHub source is a row
    of the same kind as <code>onboarding-agent#1</code>. What a <em>deployment</em> settles is the
    rest: what an event must carry to be admissible, how raw a webhook may be before a triage agent
    has to make sense of it, and how the whole thing is partitioned per tenant. It sets how far
    Temper reaches into the tools your organization already runs.
  </p>
  <p>
    <strong>What wakes an agent.</strong> A triage session is triggered by <em>something</em> —
    event volume past a threshold, a time cadence, salience accumulating past a floor — and the
    broader rhythm of sweeping a map to keep it coherent (the <em>temper-system dreaming</em> we
    keep naming) is undecided on purpose. Too eager and the system thrashes; too lazy and maps go
    stale. The right cadence depends on the map, the traffic, and the organization's tolerances —
    which is why it's a dial you set and re-set, not a constant the project ships.
  </p>
</Section>

<Section label="What's invariant, what's yours">
  <p>
    The 0→1 path is invariant and solid — it's the seed, loadable today, identical everywhere.
    Authoring new maps is invariant — a function you call. Everything that gives a running Temper
    its <em>operational</em> character — topology, tenancy, per-tenant integration, where agents
    execute, the waking cadence — is yours, varies by organization, and moves over time.
    Answerable, in other words. Answered differently by each deployment that asks.
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
