<script lang="ts">
  /**
   * The page-2 HERO: the append-only event ledger (kb_events) drawn as a single
   * horizontal spine running left→right (an arrow of time), labelled as the one
   * source of truth. Everything else fans ABOVE it as projections — resources,
   * content blocks, edges, properties, cogmap-regions — each tied back down to
   * the spine by a thin derivation arrow pointing FROM the ledger UP TO the
   * projection (asserted_by / last_event lineage). The kernel band sits between
   * spine and projections as convention-agnostic: the same ledger + entities +
   * resources underlie both Temper's workflow / KB patterns and the cognitive
   * map, with neither baked into the schema.
   *
   * Honest basis: kb_events (emitter_entity_id / event_type_id /
   * producing_anchor_* / correlation_id / metadata) and the lineage columns
   * (asserted_by_event_id / last_event_id / genesis_event_id) on the projection
   * tables.
   *
   * `id` namespaces the marker/gradient defs so multiple instances don't clash.
   */
  let { id = 'ledger-spine' }: { id?: string } = $props();
</script>

<svg
  viewBox="0 0 660 420"
  xmlns="http://www.w3.org/2000/svg"
  role="img"
  aria-label="An append-only event ledger kb_events drawn as a horizontal time-spine running left to right; above it a convention-agnostic kernel band; above that, five projection surfaces — resources, content blocks, edges, properties, and cogmap-regions — each connected back down to the spine by a thin derivation arrow"
>
  <defs>
    <!-- Derivation arrow tip: points UP, from ledger toward projection -->
    <marker
      id="{id}-up"
      viewBox="0 0 10 10"
      refX="5" refY="2"
      markerWidth="6" markerHeight="6"
      orient="auto-start-reverse"
    >
      <path d="M0,8 L5,1 L10,8" fill="none" stroke="#7eb8da" stroke-width="1.4" />
    </marker>
    <!-- Soft glow under the projection band -->
    <radialGradient id="{id}-band" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(126,184,218,0.07)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0)" />
    </radialGradient>
  </defs>

  <!-- Atmospheric wash behind the projections -->
  <ellipse cx="330" cy="120" rx="340" ry="120" fill="url(#{id}-band)" />

  <!-- ── Projection surfaces, fanning above the spine ─────────────────── -->
  <!-- Each is a faint card with a serif name + a mono lineage column.
       They are derived/replayable, so drawn lighter than the spine. -->
  {#each [
    { x: 40,  name: 'resources',      table: 'kb_resources',       lin: 'genesis_event_id' },
    { x: 168, name: 'content blocks', table: 'kb_content_blocks',  lin: 'last_event_id' },
    { x: 296, name: 'edges',          table: 'kb_edges',           lin: 'asserted_by_event_id' },
    { x: 424, name: 'properties',     table: 'kb_properties',      lin: 'last_event_id' },
    { x: 540, name: 'cogmap-regions', table: 'kb_cogmap_regions',  lin: 'genesis_event_id' },
  ] as p}
    <g transform="translate({p.x},44)">
      <rect
        x="0" y="0" width="80" height="56" rx="3"
        fill="var(--temper-blue-card)"
        stroke="var(--temper-blue-border-dim)"
        stroke-width="1"
      />
      <text
        x="40" y="22"
        text-anchor="middle"
        font-family="var(--font-serif)"
        font-size="11.5"
        fill="#e8e4df"
      >{p.name}</text>
      <text
        x="40" y="38"
        text-anchor="middle"
        font-family="var(--font-mono)"
        font-size="7"
        letter-spacing="0.3"
        fill="rgba(126,184,218,0.85)"
      >{p.table}</text>
      <text
        x="40" y="49"
        text-anchor="middle"
        font-family="var(--font-mono)"
        font-size="6"
        letter-spacing="0.2"
        fill="rgba(255,255,255,0.38)"
      >{p.lin}</text>
    </g>
  {/each}

  <!-- "projections — derived / replayable" caption above the cards -->
  <text
    x="330" y="28"
    text-anchor="middle"
    font-family="var(--font-mono)"
    font-size="8.5"
    letter-spacing="2"
    fill="rgba(255,255,255,0.4)"
  >PROJECTIONS · DERIVED, REPLAYABLE</text>

  <!-- ── The convention-agnostic kernel band ──────────────────────────── -->
  <!-- A neutral core between projections and spine: it knows events,
       entities, resources — and nothing about what they're for. -->
  <g>
    <rect
      x="40" y="148" width="580" height="40" rx="3"
      fill="rgba(255,255,255,0.025)"
      stroke="var(--rule)"
      stroke-width="1"
      stroke-dasharray="4 3"
    />
    <text
      x="56" y="166"
      font-family="var(--font-mono)"
      font-size="8.5"
      letter-spacing="2"
      fill="rgba(255,255,255,0.42)"
    >KERNEL · CONVENTION-AGNOSTIC</text>
    <text
      x="56" y="180"
      font-family="var(--font-serif)"
      font-size="10.5"
      font-style="italic"
      fill="var(--graphite)"
    >events · entities · resources · edges · properties · blocks</text>
    <text
      x="604" y="173"
      text-anchor="end"
      font-family="var(--font-mono)"
      font-size="7.5"
      letter-spacing="0.5"
      fill="rgba(255,255,255,0.3)"
    >workflow / KB · cognitive map — neither baked in</text>
  </g>

  <!-- ── Derivation arrows: FROM the spine UP TO each projection ───────── -->
  <!-- They cross the kernel band; thin and faint — the direction of
       derivation, not a load path. -->
  <g stroke="#7eb8da" stroke-width="1" opacity="0.5" fill="none">
    <line x1="80"  y1="316" x2="80"  y2="104" marker-end="url(#{id}-up)" />
    <line x1="208" y1="316" x2="208" y2="104" marker-end="url(#{id}-up)" />
    <line x1="336" y1="316" x2="336" y2="104" marker-end="url(#{id}-up)" />
    <line x1="464" y1="316" x2="464" y2="104" marker-end="url(#{id}-up)" />
    <line x1="580" y1="316" x2="580" y2="104" marker-end="url(#{id}-up)" />
  </g>

  <!-- ── The spine: kb_events, an append-only timeline left→right ──────── -->
  <g>
    <!-- The axis line -->
    <line x1="40" y1="316" x2="612" y2="316" stroke="#7eb8da" stroke-width="2" />
    <!-- Arrowhead at the right — the arrow of time, always growing -->
    <polygon points="612,316 600,311 600,321" fill="#7eb8da" />

    <!-- Event ticks: discrete, append-only, time-ordered -->
    {#each [80, 152, 224, 296, 368, 440, 512, 580] as tx}
      <line x1={tx} y1="308" x2={tx} y2="324" stroke="#7eb8da" stroke-width="2" />
      <circle cx={tx} cy="316" r="3.5" fill="var(--obsidian)" stroke="#7eb8da" stroke-width="1.5" />
    {/each}

    <!-- Spine identity label, under the axis -->
    <text
      x="40" y="352"
      font-family="var(--font-mono)"
      font-size="11"
      letter-spacing="0.5"
      fill="#7eb8da"
    >kb_events</text>
    <text
      x="40" y="367"
      font-family="var(--font-serif)"
      font-size="11"
      font-style="italic"
      fill="var(--parchment)"
    >the source of truth — append-only</text>

    <!-- Time arrow caption at the right end -->
    <text
      x="612" y="338"
      text-anchor="end"
      font-family="var(--font-mono)"
      font-size="8"
      letter-spacing="1.5"
      fill="rgba(255,255,255,0.42)"
    >time →</text>

    <!-- The four things an event carries — schema columns, mono, faint -->
    <text
      x="40" y="392"
      font-family="var(--font-mono)"
      font-size="7.5"
      letter-spacing="0.3"
      fill="rgba(255,255,255,0.38)"
    >emitter_entity_id · event_type_id · producing_anchor_* · correlation_id</text>
  </g>
</svg>

<style>
  svg {
    display: block;
    width: 100%;
    height: auto;
  }
</style>
