<script lang="ts">
  /**
   * The time-aware deep cut beside LedgerSpineDiagram: the append-only event
   * ledger (kb_events) drawn as a tilted plane at the base, with the cognitive
   * map rising OUT of it as a projection materialized in occurred_at order. A
   * scrubber (and a Replay button) walk the ledger event-by-event so you watch
   * resource-nodes, typed edges, and region glows appear — the honest
   * demonstration of "the ledger is primary; every higher surface is a
   * projection materialized at read time."
   *
   * Honest basis — every visual claim traces to schema/functions in the
   * canonical baseline (migrations/ + temper-next's tests/fixtures/):
   *   base plane, append-only, left→right  → kb_events; occurred_at;
   *                                          kb_events_append_only() trigger
   *   row labels                           → the kb_event_types registry (canonical_seed.sql)
   *   riser to a resource / cogmap node    → genesis_event_id (stamped by
   *                                          _project_blocks / _project_cogmap_seeded)
   *   riser to an edge                     → asserted_by_event_id (_project_relationship_asserted)
   *   region watermark riser               → shape_materialized_event_id (_project_region_materialized)
   *   dash pattern = edge_kind             → enum {express, contains, leads_to, near} (01_schema.sql)
   *   opacity = recency                    → last_event_id age — newer drawn harder
   *   region glow with fogged members      → kb_cogmap_regions; cogmap_shape() returns
   *                                          salience/label/count and never member identities
   *   α converged & tight                  → bound by genuine near + express content affinity (03_seed α cast)
   *   β "deployment" looser                → bound by a shared facet at weight 1.5, not content (β cast)
   *   solo-retro isolate                   → no facet, no edge; cosine WOULD merge, declared does NOT (S6d)
   *   ghost + STALE flip                   → final relationship_reweighted advances last_event_id past
   *                                          shape_materialized_event_id → cogmap_staleness() reports stale
   *
   * The cast (telos charter, α/β concepts, regulation, solo isolate, the late
   * reweight touch) is lifted from the worked scenario in 03_seed.sql — names
   * kept aligned so the picture and the seed stay mutually checkable.
   *
   * `id` namespaces the marker/gradient/filter defs so multiple instances on a
   * page don't clash.
   */
  let { id = 'ledger-projection-spiral' }: { id?: string } = $props();

  type Ev = {
    t: string;
    lin: string;
    show?: string[];
    riser?: string;
    weather?: boolean;
    regionA?: boolean;
    regionB?: boolean;
    supersede?: boolean;
  };

  // The worked scenario from 03_seed.sql, in occurred_at order. Never mutated,
  // so a module-const rather than $state (per the port plan).
  const EVENTS: Ev[] = [
    { t: 'cogmap_seeded', lin: 'genesis_event_id', show: ['node-telos'], riser: 'node-telos' },
    { t: 'resource_created', lin: 'genesis_event_id', show: ['node-a1'], riser: 'node-a1' },
    { t: 'resource_created', lin: 'genesis_event_id', show: ['node-a2'], riser: 'node-a2' },
    { t: 'relationship_asserted', lin: 'asserted_by_event_id · near', show: ['edge-e1'] },
    { t: 'relationship_asserted', lin: 'asserted_by_event_id · express', show: ['edge-e2', 'edge-e5'] },
    { t: 'resource_created', lin: 'genesis_event_id', show: ['node-reg-ghost'], riser: 'node-reg-ghost' },
    { t: 'resource_created', lin: 'genesis_event_id', show: ['node-b1'], riser: 'node-b1', weather: true },
    { t: 'resource_created', lin: 'genesis_event_id', show: ['node-b2', 'node-b3'], riser: 'node-b2' },
    { t: 'relationship_asserted', lin: 'asserted_by_event_id · leads_to', show: ['edge-e3', 'edge-e4'] },
    { t: 'resource_created', lin: 'genesis_event_id · isolate', show: ['node-solo'], riser: 'node-solo' },
    { t: 'region_materialized', lin: 'shape_materialized_event_id', regionA: true },
    { t: 'region_materialized', lin: 'shape_materialized_event_id', regionB: true },
    { t: 'relationship_reweighted', lin: 'last_event_id → STALE', supersede: true },
  ];
  const N = EVENTS.length;

  // The ledger plane geometry — rows fan down-and-right off the base plane.
  // Carried verbatim from the reference artifact.
  const PLANE_Y0 = 398;
  const PLANE_DY = 10.6;
  function rowGeo(i: number) {
    const y = PLANE_Y0 + i * PLANE_DY;
    const x = 60 + i * 4;
    const w = 150 + i * 26;
    return { x, y, w, cx: x + w / 2, cy: y - 2 };
  }

  // Bottom-of-circle anchor for every node a riser can land on. Mirrors the
  // cx/cy/r drawn in the template below (the reference read these off the DOM;
  // declarative Svelte names them once so risers and circles can't drift).
  const ANCHORS: Record<string, { x: number; y: number }> = {
    'node-telos': { x: 318, y: 339 },
    'node-a1': { x: 232, y: 221 },
    'node-a2': { x: 276, y: 212.5 },
    'node-b1': { x: 442, y: 244.5 },
    'node-b2': { x: 494, y: 258 },
    'node-b3': { x: 470, y: 290 },
    'node-reg-ghost': { x: 372, y: 306.5 },
    'node-reg': { x: 360, y: 319 },
    'node-solo': { x: 582, y: 336 },
  };

  // The single index events that flip a whole sub-picture on.
  const idxA = EVENTS.findIndex((e) => e.regionA);
  const idxB = EVENTS.findIndex((e) => e.regionB);
  const wIdx = EVENTS.findIndex((e) => e.weather);
  const supIdx = EVENTS.findIndex((e) => e.supersede);

  // Static once-per-event ledger ticks on the time axis.
  const ticks = EVENTS.map((_, i) => 56 + i * 42);

  // ── Interaction state ────────────────────────────────────────────────
  // Resting frame is step = N - 1: the fully converged, healthy map BEFORE the
  // late reweight makes it stale. The scrubber reaches N (the STALE flip) — the
  // reference capped its <input> one short of that, leaving the payoff
  // scrubbable only via Replay; here it is directly reachable.
  let step = $state(N - 1);
  let playing = $state(false);

  // An element is visible iff its event index < step; opacity scales with
  // recency. node-reg-ghost / node-reg are driven separately (below) because
  // the late supersede overrides their plain show-rule.
  const elementOpacity = $derived.by(() => {
    const m: Record<string, number> = {};
    EVENTS.forEach((e, i) => {
      const on = i < step;
      const recency = (i + 1) / N;
      for (const sid of e.show ?? []) m[sid] = on ? 0.4 + 0.6 * recency : 0;
    });
    return m;
  });

  const regAon = $derived(idxA >= 0 && idxA < step);
  const regBon = $derived(idxB >= 0 && idxB < step);
  const weatherOn = $derived(wIdx >= 0 && wIdx < step);
  const superseded = $derived(supIdx >= 0 && supIdx < step);
  // Ghost appears once the regulation node is created (event 5), at 0.25; the
  // final reweight folds it, deepening it to 0.6 as the live node takes over.
  const ghostOpacity = $derived(step > 5 ? (superseded ? 0.6 : 0.25) : 0);

  const rows = $derived.by(() =>
    EVENTS.map((e, i) => {
      const g = rowGeo(i);
      const on = i < step;
      const recency = (i + 1) / N;
      return { e, g, opacity: on ? 0.5 + 0.5 * recency : 0 };
    }),
  );

  const visibleRisers = $derived.by(() => {
    const out: { x1: number; y1: number; x2: number; y2: number; opacity: number }[] = [];
    for (let i = 0; i < step && i < N; i++) {
      const e = EVENTS[i];
      if (!e.riser) continue;
      const a = ANCHORS[e.riser];
      if (!a) continue;
      const g = rowGeo(i);
      out.push({ x1: g.cx, y1: g.cy, x2: a.x, y2: a.y, opacity: 0.12 + 0.3 * ((i + 1) / N) });
    }
    return out;
  });

  const evLabel = $derived(
    step === 0
      ? 'empty ledger'
      : step >= N
        ? `all ${N} events · stale`
        : `${step} · ${EVENTS[step - 1].lin}`,
  );

  function togglePlay() {
    if (playing) {
      playing = false;
      return;
    }
    if (step >= N) step = 0;
    playing = true;
  }

  // Drive the replay while `playing`; tear the interval down on pause or unmount.
  $effect(() => {
    if (!playing) return;
    const timer = setInterval(() => {
      step += 1;
      if (step >= N) playing = false;
    }, 680);
    return () => clearInterval(timer);
  });
</script>

<div class="controls">
  <button type="button" onclick={togglePlay} aria-label="Replay projection">
    {playing ? '❚❚ Pause' : '▶ Replay'}
  </button>
  <input
    type="range"
    min="0"
    max={N}
    step="1"
    bind:value={step}
    oninput={() => {
      if (playing) playing = false;
    }}
    aria-label="Ledger time"
  />
  <span class="evlabel">{evLabel}</span>
</div>

<svg
  viewBox="0 0 680 640"
  xmlns="http://www.w3.org/2000/svg"
  role="img"
  aria-label="An append-only event ledger at the base; events rise off it into a graph of resource-nodes and typed edges that gain opacity with recency; two region glows converge while an isolate stays separate and a superseded node ghosts behind"
>
  <title>The event ledger rising into a cognitive map</title>
  <desc
    >An append-only ledger at the base; events rise off it into a graph of resource-nodes
    and typed edges that gain opacity with recency; two region glows converge while an isolate stays
    separate and a superseded node ghosts behind.</desc
  >
  <defs>
    <marker
      id="{id}-up"
      viewBox="0 0 10 10"
      refX="5"
      refY="2"
      markerWidth="5.5"
      markerHeight="5.5"
      orient="auto-start-reverse"
    >
      <path d="M0,8 L5,1 L10,8" fill="none" stroke="var(--temper-blue)" stroke-width="1.3" />
    </marker>
    <radialGradient id="{id}-glowA" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(126,184,218,0.36)" />
      <stop offset="55%" stop-color="rgba(126,184,218,0.13)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0)" />
    </radialGradient>
    <radialGradient id="{id}-glowB" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(130,201,154,0.26)" />
      <stop offset="60%" stop-color="rgba(130,201,154,0.08)" />
      <stop offset="100%" stop-color="rgba(130,201,154,0)" />
    </radialGradient>
    <radialGradient id="{id}-weather" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(126,184,218,0.05)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0)" />
    </radialGradient>
    <filter id="{id}-fog" x="-60%" y="-60%" width="220%" height="220%">
      <feGaussianBlur stdDeviation="3.6" />
    </filter>
  </defs>

  <text
    x="40"
    y="30"
    font-family="var(--font-mono)"
    font-size="10.5"
    letter-spacing="2"
    fill="var(--graphite)">PROJECTION · MATERIALIZED AT READ TIME · THE GRAPH RISES AS THE LEDGER GROWS</text
  >

  <!-- Atmospheric wash that arrives with the β cast -->
  <g opacity={weatherOn ? 1 : 0}>
    <ellipse cx="340" cy="230" rx="300" ry="170" fill="url(#{id}-weather)" />
  </g>

  <!-- ── Region glows: salience/label/count, members fogged ──────────────── -->
  <g opacity={regAon ? 1 : 0}>
    <ellipse cx="250" cy="232" rx="132" ry="104" fill="url(#{id}-glowA)" />
    <text x="250" y="132" text-anchor="middle" font-family="var(--font-serif)" font-size="14" fill="var(--parchment)"
      >“first-week confidence”</text
    >
    <text
      x="250"
      y="149"
      text-anchor="middle"
      font-family="var(--font-mono)"
      font-size="8.5"
      letter-spacing="0.5"
      fill="var(--graphite)">converged · salience high · cohesion tight · ≈3 fogged</text
    >
  </g>
  <g opacity={regBon ? 0.85 : 0}>
    <ellipse cx="468" cy="250" rx="116" ry="94" fill="url(#{id}-glowB)" />
    <text x="468" y="160" text-anchor="middle" font-family="var(--font-serif)" font-size="13" fill="rgba(232,228,223,0.9)"
      >“deployment”</text
    >
    <text
      x="468"
      y="176"
      text-anchor="middle"
      font-family="var(--font-mono)"
      font-size="8"
      letter-spacing="0.5"
      fill="var(--graphite)">forming · bound by facet, not content</text
    >
  </g>

  <!-- Fogged member dots — present but never identified (cogmap_shape withholds ids) -->
  <g opacity={regAon ? 0.5 : 0} filter="url(#{id}-fog)" fill="var(--temper-blue)">
    <circle cx="214" cy="220" r="5.5" opacity="0.85" />
    <circle cx="276" cy="206" r="5" opacity="0.8" />
    <circle cx="252" cy="256" r="5.5" opacity="0.82" />
  </g>
  <g opacity={regBon ? 0.45 : 0} filter="url(#{id}-fog)" fill="var(--graph-session)">
    <circle cx="442" cy="238" r="4.5" opacity="0.7" />
    <circle cx="494" cy="252" r="4.5" opacity="0.68" />
    <circle cx="466" cy="282" r="4" opacity="0.62" />
    <circle cx="470" cy="222" r="4" opacity="0.6" />
  </g>

  <!-- ── The graph: typed edges (dash = edge_kind) then resource-nodes ────── -->
  <g>
    <g opacity={elementOpacity['edge-e1'] ?? 0}
      ><line x1="232" y1="214" x2="276" y2="206" stroke="var(--temper-blue)" stroke-width="1.4" stroke-dasharray="2 4" /></g
    >
    <g opacity={elementOpacity['edge-e2'] ?? 0}
      ><line x1="252" y1="256" x2="232" y2="214" stroke="var(--temper-blue)" stroke-width="1.4" /></g
    >
    <g opacity={elementOpacity['edge-e3'] ?? 0}
      ><line x1="442" y1="238" x2="494" y2="252" stroke="var(--graph-session)" stroke-width="1.3" stroke-dasharray="7 4" /></g
    >
    <g opacity={elementOpacity['edge-e4'] ?? 0}
      ><line x1="494" y1="252" x2="470" y2="284" stroke="var(--graph-session)" stroke-width="1.3" stroke-dasharray="7 4" /></g
    >
    <g opacity={elementOpacity['edge-e5'] ?? 0}
      ><line x1="318" y1="330" x2="252" y2="256" stroke="var(--graph-concept)" stroke-width="1.3" stroke-dasharray="1 4" /></g
    >

    <g opacity={elementOpacity['node-telos'] ?? 0}>
      <circle cx="318" cy="330" r="9" fill="var(--obsidian)" stroke="var(--temper-blue)" stroke-width="2.2" />
      <text x="318" y="352" text-anchor="middle" fill="var(--parchment)" font-family="var(--font-serif)" font-size="11"
        >telos charter</text
      >
      <text x="318" y="313" text-anchor="middle" fill="rgba(126,184,218,0.85)" font-family="var(--font-mono)" font-size="8"
        >genesis_event_id</text
      >
    </g>
    <g opacity={elementOpacity['node-a1'] ?? 0}>
      <circle cx="232" cy="214" r="7" fill="var(--obsidian)" stroke="var(--temper-blue)" stroke-width="1.7" />
      <text x="232" y="197" text-anchor="middle" fill="var(--parchment)" font-family="var(--font-serif)" font-size="10"
        >pair-on-first-PR</text
      >
    </g>
    <g opacity={elementOpacity['node-a2'] ?? 0}>
      <circle cx="276" cy="206" r="6.5" fill="var(--obsidian)" stroke="var(--temper-blue)" stroke-width="1.5" />
    </g>
    <g opacity={elementOpacity['node-b1'] ?? 0}>
      <circle cx="442" cy="238" r="6.5" fill="var(--obsidian)" stroke="var(--graph-session)" stroke-width="1.5" />
      <text x="442" y="221" text-anchor="middle" fill="rgba(232,228,223,0.85)" font-family="var(--font-serif)" font-size="9.5"
        >staging</text
      >
    </g>
    <g opacity={elementOpacity['node-b2'] ?? 0}>
      <circle cx="494" cy="252" r="6" fill="var(--obsidian)" stroke="var(--graph-session)" stroke-width="1.4" />
    </g>
    <g opacity={elementOpacity['node-b3'] ?? 0}>
      <circle cx="470" cy="284" r="6" fill="var(--obsidian)" stroke="var(--graph-session)" stroke-width="1.4" />
    </g>

    <!-- Regulation: a ghost of the folded node sits behind the live one -->
    <g opacity={ghostOpacity}>
      <circle cx="372" cy="300" r="6.5" fill="none" stroke="rgba(126,184,218,0.3)" stroke-width="1.2" stroke-dasharray="2 2" />
    </g>
    <g opacity={superseded ? 1 : 0}>
      <circle cx="360" cy="312" r="7" fill="var(--obsidian)" stroke="var(--temper-blue)" stroke-width="1.6" />
      <text x="360" y="333" text-anchor="middle" fill="var(--parchment)" font-family="var(--font-serif)" font-size="9.5"
        >regulation</text
      >
    </g>

    <!-- The solo isolate: cosine WOULD merge it, the declared graph does NOT -->
    <g opacity={elementOpacity['node-solo'] ?? 0}>
      <circle cx="582" cy="330" r="6" fill="var(--obsidian)" stroke="rgba(255,255,255,0.35)" stroke-width="1.3" />
      <text x="582" y="312" text-anchor="middle" fill="var(--chalk)" font-family="var(--font-serif)" font-size="9.5"
        >solo-retro</text
      >
      <text x="582" y="349" text-anchor="middle" fill="var(--graphite)" font-family="var(--font-mono)" font-size="7.5"
        >no facet · no edge</text
      >
    </g>
  </g>

  <!-- ── Risers: lineage columns the projection-half stamps ──────────────── -->
  <g fill="none">
    {#each visibleRisers as r}
      <line
        x1={r.x1}
        y1={r.y1}
        x2={r.x2}
        y2={r.y2}
        stroke="var(--temper-blue)"
        stroke-width="0.7"
        opacity={r.opacity}
        marker-end="url(#{id}-up)"
      />
    {/each}
  </g>

  <!-- ── The base plane: kb_events, append-only, occurred_at left→right ──── -->
  <g>
    <polygon
      points="118,388 562,388 632,520 48,520"
      fill="rgba(126,184,218,0.045)"
      stroke="rgba(126,184,218,0.22)"
      stroke-width="1"
    />
  </g>

  <!-- Ledger rows, one per event, fanning down the plane -->
  <g font-family="var(--font-mono)" font-size="8">
    {#each rows as row}
      <g opacity={row.opacity}>
        <rect
          x={row.g.x}
          y={row.g.y - 8}
          width={row.g.w}
          height="11"
          rx="2"
          fill="var(--temper-blue-card)"
          stroke="var(--temper-blue-border-dim)"
          stroke-width="0.7"
        />
        <text x={row.g.x + 6} y={row.g.y + 1} fill="rgba(126,184,218,0.92)" font-size="8">{row.e.t}</text>
      </g>
    {/each}
  </g>

  <!-- ── The time axis, with one tick per event ──────────────────────────── -->
  <g>
    <line x1="48" y1="536" x2="600" y2="536" stroke="var(--temper-blue)" stroke-width="1.5" />
    <polygon points="612,536 598,531 598,541" fill="var(--temper-blue)" />
    {#each ticks as tx}
      <line x1={tx} y1="530" x2={tx} y2="542" stroke="var(--temper-blue)" stroke-width="1.4" />
    {/each}
    <text x="612" y="554" text-anchor="end" font-family="var(--font-mono)" font-size="8" letter-spacing="1.5" fill="var(--graphite)"
      >time →</text
    >
    <text x="48" y="572" font-family="var(--font-mono)" font-size="11" letter-spacing="0.5" fill="var(--temper-blue)">kb_events</text>
    <text x="48" y="587" font-family="var(--font-serif)" font-size="11" font-style="italic" fill="var(--parchment)"
      >the source of truth — append-only</text
    >
    <text x="48" y="606" font-family="var(--font-mono)" font-size="7.5" fill="var(--graphite)"
      >emitter_entity_id · event_type_id · payload · producing_anchor_* · occurred_at</text
    >
  </g>

  <!-- ── Legend: dash = edge_kind, plus the fold + recency keys ──────────── -->
  <g font-family="var(--font-mono)" font-size="8" opacity="0.92">
    <line x1="486" y1="600" x2="506" y2="600" stroke="var(--temper-blue)" stroke-width="1.4" stroke-dasharray="1 4" />
    <text x="512" y="603" fill="var(--graphite)">express</text>
    <line x1="486" y1="612" x2="506" y2="612" stroke="var(--temper-blue)" stroke-width="1.4" />
    <text x="512" y="615" fill="var(--graphite)">contains</text>
    <line x1="560" y1="600" x2="580" y2="600" stroke="var(--temper-blue)" stroke-width="1.4" stroke-dasharray="7 4" />
    <text x="586" y="603" fill="var(--graphite)">leads_to</text>
    <line x1="560" y1="612" x2="580" y2="612" stroke="var(--temper-blue)" stroke-width="1.4" stroke-dasharray="2 4" />
    <text x="586" y="615" fill="var(--graphite)">near</text>
    <circle cx="491" cy="624" r="4" fill="none" stroke="rgba(126,184,218,0.3)" stroke-width="1.1" stroke-dasharray="2 2" />
    <text x="512" y="627" fill="var(--graphite)">folded / superseded</text>
    <text x="586" y="589" fill="var(--graphite)">opacity = recency</text>
  </g>
</svg>

<style>
  .controls {
    display: flex;
    align-items: center;
    gap: 14px;
    flex-wrap: wrap;
    margin: 0 0 12px;
    font-size: 13px;
    color: var(--chalk);
  }

  button {
    font-family: var(--font-mono);
    background: var(--temper-blue-card);
    color: var(--temper-blue);
    border: 1px solid var(--temper-blue-border-dim);
    border-radius: 6px;
    padding: 6px 12px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  button:hover {
    background: var(--temper-blue-glow);
  }

  input[type='range'] {
    flex: 1;
    min-width: 150px;
    accent-color: var(--temper-blue);
  }

  .evlabel {
    min-width: 150px;
    text-align: right;
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--temper-blue);
  }

  svg {
    display: block;
    width: 100%;
    height: auto;
  }
</style>
