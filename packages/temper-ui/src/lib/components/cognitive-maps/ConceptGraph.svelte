<script lang="ts">
  /**
   * The /cognitive-maps index, drawn as the thing it describes: a small curated
   * field of concepts where the page links ARE the vertices and their prose
   * cross-references are the edges. Two clusters keep the structural genre split
   * legible — the upper field is *shown from the schema*, the lower is the
   * *invitation* to operating Temper. Nodes are real <a> links (SSR/SEO/a11y);
   * the layout is hand-placed and deterministic, not force-directed.
   *
   * Coordinates are curated for legibility. Hrefs match the tier routes in
   * ./nav.ts; labels are short concept-handles ("name the thing").
   */

  type NodeKey =
    | 'what-is' | 'substrate' | 'lives' | 'grows' | 'relate' | 'visible'
    | 'operating' | 'deployment' | 'governance' | 'observability' | 'insights';

  interface GraphNode {
    href: string;
    label: string;
    cx: number;
    cy: number;
    hub?: boolean;
    /** The recommended entry point — drawn green with a "start here" tag so the
        eye lands here, not on the higher-density operating-Temper hub. */
    start?: boolean;
  }

  const NODES: Record<NodeKey, GraphNode> = {
    'what-is':       { href: '/cognitive-maps/what-a-cognitive-map-is',   label: 'what a map is',   cx: 150, cy: 108, start: true },
    substrate:       { href: '/cognitive-maps/the-substrate-beneath-it',  label: 'the substrate',   cx: 372, cy: 88 },
    lives:           { href: '/cognitive-maps/what-lives-in-a-map',       label: 'what lives in it', cx: 108, cy: 226 },
    grows:           { href: '/cognitive-maps/how-a-map-grows',           label: 'how it grows',    cx: 300, cy: 214 },
    relate:          { href: '/cognitive-maps/how-maps-relate',           label: 'how maps relate', cx: 540, cy: 156 },
    visible:         { href: '/cognitive-maps/whats-visible-from-here',   label: "what's visible",  cx: 486, cy: 272 },
    operating:       { href: '/cognitive-maps/operating-temper',          label: 'operating Temper', cx: 360, cy: 400, hub: true },
    deployment:      { href: '/operating/deployment',                 label: 'deployment',    cx: 132, cy: 504 },
    governance:      { href: '/operating/governance-and-administration', label: 'governance',  cx: 300, cy: 516 },
    observability:   { href: '/operating/observability-and-audit',     label: 'observability', cx: 470, cy: 516 },
    insights:        { href: '/operating/insights',                    label: 'insights',      cx: 612, cy: 504 },
  };

  // Edges = the conceptual cross-references the prose actually draws.
  // kind 'flow' is a within-arc link; 'bridge' crosses show → invite (dashed).
  const EDGES: { from: NodeKey; to: NodeKey; kind: 'flow' | 'bridge' }[] = [
    { from: 'what-is', to: 'substrate', kind: 'flow' },
    { from: 'what-is', to: 'lives', kind: 'flow' },
    { from: 'what-is', to: 'grows', kind: 'flow' },
    { from: 'what-is', to: 'relate', kind: 'flow' },
    { from: 'what-is', to: 'visible', kind: 'flow' },
    { from: 'substrate', to: 'grows', kind: 'flow' },
    { from: 'lives', to: 'grows', kind: 'flow' },
    { from: 'grows', to: 'relate', kind: 'flow' },
    { from: 'relate', to: 'visible', kind: 'flow' },
    { from: 'grows', to: 'operating', kind: 'bridge' },
    { from: 'visible', to: 'operating', kind: 'bridge' },
    { from: 'operating', to: 'deployment', kind: 'flow' },
    { from: 'operating', to: 'governance', kind: 'flow' },
    { from: 'operating', to: 'observability', kind: 'flow' },
    { from: 'operating', to: 'insights', kind: 'flow' },
    { from: 'governance', to: 'observability', kind: 'flow' },
    { from: 'observability', to: 'insights', kind: 'flow' },
  ];

  const showKeys: NodeKey[] = ['what-is', 'substrate', 'lives', 'grows', 'relate', 'visible'];
  const inviteKeys: NodeKey[] = ['operating', 'deployment', 'governance', 'observability', 'insights'];
</script>

<figure class="concept-graph">
  <svg
    viewBox="0 0 720 580"
    xmlns="http://www.w3.org/2000/svg"
    role="group"
    aria-label="The cognitive-maps page set as a concept graph: six pages shown from the schema, and an invitation arc of five pages on operating Temper"
  >
    <!-- Cluster band labels -->
    <text x="40" y="40" font-family="var(--font-mono)" font-size="10" letter-spacing="2.5" fill="rgba(255,255,255,0.4)">SHOWN FROM THE SCHEMA</text>
    <line x1="40" y1="332" x2="680" y2="332" stroke="var(--rule)" stroke-width="1" stroke-dasharray="2 6" />
    <text x="40" y="362" font-family="var(--font-mono)" font-size="10" letter-spacing="2.5" fill="rgba(126,184,218,0.6)">AN INVITATION · OPERATING TEMPER</text>

    <!-- Edges first, under the nodes -->
    <g>
      {#each EDGES as e (e.from + e.to)}
        <line
          x1={NODES[e.from].cx} y1={NODES[e.from].cy}
          x2={NODES[e.to].cx}   y2={NODES[e.to].cy}
          class="edge {e.kind}"
        />
      {/each}
    </g>

    <!-- Nodes as links -->
    {#each [...showKeys, ...inviteKeys] as key (key)}
      {@const n = NODES[key]}
      <a href={n.href} class="node" class:hub={n.hub} class:start={n.start} aria-label={n.label}>
        {#if n.start}
          <circle class="start-halo" cx={n.cx} cy={n.cy} r="15" />
          <text x={n.cx} y={n.cy - 22} text-anchor="middle" class="start-tag">start here</text>
        {/if}
        <circle cx={n.cx} cy={n.cy} r={n.hub ? 11 : 7} />
        <text
          x={n.cx}
          y={n.cy + (n.hub ? 30 : 24)}
          text-anchor="middle"
          class="node-label"
        >{n.label}</text>
      </a>
    {/each}
  </svg>
</figure>

<style>
  .concept-graph {
    max-width: 760px;
    margin: 1rem auto 0;
    padding: 1.5rem 1rem;
  }
  svg {
    display: block;
    width: 100%;
    height: auto;
    overflow: visible;
  }

  .edge {
    stroke: var(--temper-blue-border-dim);
    stroke-width: 1;
  }
  .edge.bridge {
    stroke-dasharray: 4 4;
    stroke: rgba(126, 184, 218, 0.4);
  }

  .node circle {
    fill: var(--obsidian);
    stroke: var(--temper-blue-border);
    stroke-width: 1.5;
    transition: fill 0.18s, stroke 0.18s, r 0.18s;
  }
  .node.hub circle {
    fill: var(--temper-blue-card);
    stroke: var(--temper-blue);
    stroke-width: 2;
  }
  .node-label {
    font-family: var(--font-serif);
    font-size: 13px;
    fill: var(--chalk);
    transition: fill 0.18s;
  }
  .node.hub .node-label {
    fill: var(--parchment);
    font-size: 14px;
  }

  /* The recommended entry point — a soft green node + halo so the eye starts
     here rather than on the denser operating-Temper hub. */
  .node.start circle {
    fill: rgba(130, 201, 154, 0.16);
    stroke: #82c99a;
    stroke-width: 2;
  }
  .node.start .node-label {
    fill: var(--parchment);
  }
  .start-halo {
    fill: none;
    stroke: #82c99a;
    stroke-width: 1;
    opacity: 0.4;
  }
  .start-tag {
    font-family: var(--font-mono);
    font-size: 8.5px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    fill: #82c99a;
  }

  .node { cursor: pointer; }
  .node:hover circle,
  .node:focus-visible circle {
    fill: var(--temper-blue);
    stroke: var(--temper-blue);
  }
  .node:hover .node-label,
  .node:focus-visible .node-label {
    fill: var(--parchment);
  }
  /* Keep the entry node green on hover rather than flipping to blue. */
  .node.start:hover circle,
  .node.start:focus-visible circle {
    fill: #82c99a;
    stroke: #86efac;
  }
  .node:focus-visible { outline: none; }
  .node:focus-visible circle { stroke-width: 3; }
</style>
