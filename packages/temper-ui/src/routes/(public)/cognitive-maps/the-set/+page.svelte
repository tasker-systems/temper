<script lang="ts">
  import type { Component } from 'svelte';
  import RegionFieldDiagram from '$lib/components/cognitive-maps/diagrams/RegionFieldDiagram.svelte';
  import LedgerSpineDiagram from '$lib/components/cognitive-maps/diagrams/LedgerSpineDiagram.svelte';
  import LedgerProjectionSpiral from '$lib/components/cognitive-maps/diagrams/LedgerProjectionSpiral.svelte';
  import ResourceBlockERD from '$lib/components/cognitive-maps/diagrams/ResourceBlockERD.svelte';
  import LearningActsDiagram from '$lib/components/cognitive-maps/diagrams/LearningActsDiagram.svelte';
  import ShapeBoundaryDiagram from '$lib/components/cognitive-maps/diagrams/ShapeBoundaryDiagram.svelte';
  import PromotionDiagram from '$lib/components/cognitive-maps/diagrams/PromotionDiagram.svelte';
  import SeedDagDiagram from '$lib/components/cognitive-maps/diagrams/SeedDagDiagram.svelte';
  import TwoGatesDiagram from '$lib/components/cognitive-maps/diagrams/TwoGatesDiagram.svelte';
  import BootstrapRangeDiagram from '$lib/components/cognitive-maps/diagrams/BootstrapRangeDiagram.svelte';
  import AdminFirewallDiagram from '$lib/components/cognitive-maps/diagrams/AdminFirewallDiagram.svelte';
  import AuditHomesDiagram from '$lib/components/cognitive-maps/diagrams/AuditHomesDiagram.svelte';
  import ProvenanceChainDiagram from '$lib/components/cognitive-maps/diagrams/ProvenanceChainDiagram.svelte';

  interface Plate {
    id: string;
    component: Component<{ id?: string }>;
    href: string;
    page: string;
    line: string;
  }

  // The whole argument, one image per page (a couple of pages earn two). Each
  // plate reuses the page's own diagram; the cell links to where it lives.
  const SHOW: Plate[] = [
    {
      id: 'region', component: RegionFieldDiagram,
      href: '/cognitive-maps/what-a-cognitive-map-is', page: 'What a cognitive map is',
      line: "A map's honest shape — regions with weight and population, members fogged.",
    },
    {
      id: 'ledger', component: LedgerSpineDiagram,
      href: '/cognitive-maps/the-substrate-beneath-it', page: 'The substrate beneath it',
      line: 'Events are primary; every surface is a projection over the ledger.',
    },
    {
      id: 'erd', component: ResourceBlockERD,
      href: '/cognitive-maps/what-lives-in-a-map', page: 'What lives in a map',
      line: 'Two primitives — a resource stands in the graph; a block is its addressable interior.',
    },
    {
      id: 'acts', component: LearningActsDiagram,
      href: '/cognitive-maps/how-a-map-grows', page: 'How a map grows',
      line: 'Five learning-acts — form, modify, decay, fold, and the scar that stays.',
    },
    {
      id: 'shape', component: ShapeBoundaryDiagram,
      href: '/cognitive-maps/how-maps-relate', page: 'How maps relate',
      line: 'Shape crosses a boundary; the material interior stays home.',
    },
    {
      id: 'promo', component: PromotionDiagram,
      href: '/cognitive-maps/how-maps-relate', page: 'How maps relate',
      line: 'Promotion — a curated concept sent forward across scopes (proposed).',
    },
    {
      id: 'dag', component: SeedDagDiagram,
      href: '/cognitive-maps/whats-visible-from-here', page: "What's visible from here",
      line: 'The teams DAG, and visibility = permission × precedence.',
    },
    {
      id: 'gates', component: TwoGatesDiagram,
      href: '/cognitive-maps/whats-visible-from-here', page: "What's visible from here",
      line: 'Two gates, kept apart — reading a shape vs reading resources.',
    },
  ];

  const INVITE: Plate[] = [
    {
      id: 'bootstrap', component: BootstrapRangeDiagram,
      href: '/operating/deployment', page: 'Deployment',
      line: '0→1 is the invariant seed; topology, tenancy, and agents are the org-shaped range.',
    },
    {
      id: 'admin', component: AdminFirewallDiagram,
      href: '/operating/governance-and-administration', page: 'Governance & administration',
      line: 'Administration is event-sourced — and firewalled from cognition.',
    },
    {
      id: 'audit', component: AuditHomesDiagram,
      href: '/operating/observability-and-audit', page: 'Observability & audit',
      line: 'Two audits, two homes — operational outside, epistemic in the ledger.',
    },
    {
      id: 'provenance', component: ProvenanceChainDiagram,
      href: '/operating/insights', page: 'Insights',
      line: 'A causal chain from a merge to a shift in a map’s understanding.',
    },
  ];
</script>

<svelte:head>
  <title>The set, at a glance — temper</title>
  <meta
    name="description"
    content="Every page in the cognitive-maps set earns one image, drawn from the data model — the whole argument seen at once, before you walk it. Shown from the schema, then the invitation to operating Temper."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/cognitive-maps/the-set</div>
  <h1 class="t-hero-title">The set, <em>at a glance</em></h1>
  <p class="tagline t-tagline">
    Every page earns one picture, drawn from the data model — the whole argument
    seen at once, before you walk it.
  </p>
</section>

<div class="set-page">
  <section class="lead">
    <div class="lead-head">
      <span class="lead-label t-label">Start here · the whole argument in motion</span>
      <p class="lead-note">
        One image carries the value proposition; the rest of the set elaborates its
        parts. The append-only event ledger sits at the base, and the cognitive map
        materializes out of it in event order — scrub it, or replay. Regions forming,
        typed edges, lineage risers, the late flip to <em>stale</em>: everything the
        plates below show precisely is already here, derived from the events and
        nothing else. Start with this; then walk the readings.
      </p>
    </div>
    <div class="lead-canvas">
      <LedgerProjectionSpiral id="set-lead-spiral" />
    </div>
  </section>

  <section class="band">
    <div class="band-head">
      <span class="band-label t-label">Shown from the schema</span>
      <p class="band-note">
        Six pages that argue from the data model outward — the visuals are the
        evidence.
      </p>
    </div>
    <div class="plates">
      {#each SHOW as p (p.id)}
        {@const Diagram = p.component}
        <a class="plate" href={p.href}>
          <div class="plate-canvas"><Diagram id={`set-${p.id}`} /></div>
          <div class="plate-meta">
            <span class="plate-page">{p.page}</span>
            <p class="plate-line">{p.line}</p>
          </div>
        </a>
      {/each}
    </div>
  </section>

  <section class="band">
    <div class="band-head">
      <span class="band-label band-label--invite t-label">An invitation · operating Temper</span>
      <p class="band-note">
        What a deployment shapes, argued toward decisions not yet made.
      </p>
    </div>
    <div class="plates">
      {#each INVITE as p (p.id)}
        {@const Diagram = p.component}
        <a class="plate" href={p.href}>
          <div class="plate-canvas"><Diagram id={`set-${p.id}`} /></div>
          <div class="plate-meta">
            <span class="plate-page">{p.page}</span>
            <p class="plate-line">{p.line}</p>
          </div>
        </a>
      {/each}
    </div>
  </section>
</div>

<style>
  .hero {
    min-height: 44vh;
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    text-align: center;
    padding: 4.5rem 2.5rem 1.5rem;
  }
  .hero-label { margin-bottom: 1.5rem; }
  .hero h1 { margin-bottom: 1.5rem; }
  .tagline { max-width: 38em; }

  .set-page {
    max-width: 1040px;
    margin: 0 auto;
    padding: 1rem 2.5rem 0;
  }

  .lead { margin-bottom: 3.75rem; }
  .lead-head {
    border-top: 1px solid var(--rule);
    padding-top: 1.25rem;
    margin-bottom: 1.5rem;
  }
  .lead-label {
    display: block;
    margin-bottom: 0.75rem;
    color: var(--temper-blue);
  }
  .lead-note {
    margin: 0;
    font-family: var(--font-serif);
    font-size: 1.05rem;
    line-height: 1.62;
    color: var(--chalk);
    max-width: 44em;
  }
  .lead-note em {
    font-style: italic;
    color: var(--parchment);
  }
  .lead-canvas {
    margin: 1.75rem auto 0;
    max-width: 760px;
    padding: 1.5rem 1.5rem 1.1rem;
    background: var(--obsidian-3);
    border: 1px solid var(--rule);
  }

  .band { margin-bottom: 3.5rem; }
  .band-head {
    border-top: 1px solid var(--rule);
    padding-top: 1.25rem;
    margin-bottom: 1.5rem;
  }
  .band-label { display: block; margin-bottom: 0.5rem; }
  .band-label--invite { color: var(--temper-blue); }
  .band-note {
    margin: 0;
    font-family: var(--font-serif);
    font-style: italic;
    font-size: 0.95rem;
    color: var(--graphite);
    max-width: 40em;
  }

  .plates {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 1.5rem;
  }

  .plate {
    display: flex;
    flex-direction: column;
    text-decoration: none;
    background: var(--obsidian-3);
    border: 1px solid var(--rule);
    transition: border-color 0.18s, transform 0.18s;
  }
  .plate:hover {
    border-color: var(--temper-blue-border);
    transform: translateY(-2px);
  }
  .plate-canvas {
    padding: 1.25rem 1.25rem 0.5rem;
  }
  .plate-meta {
    padding: 0.75rem 1.25rem 1.1rem;
    border-top: 1px solid var(--rule);
    margin-top: auto;
  }
  .plate-page {
    font-family: var(--font-mono);
    font-size: 0.62rem;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: var(--temper-blue);
  }
  .plate-line {
    margin: 0.4rem 0 0;
    font-family: var(--font-serif);
    font-size: 0.9rem;
    line-height: 1.55;
    color: var(--chalk);
  }
  .plate:hover .plate-line { color: var(--parchment); }

  @media (max-width: 720px) {
    .plates { grid-template-columns: 1fr; }
    .set-page { padding: 1rem 1.5rem 0; }
  }
</style>
