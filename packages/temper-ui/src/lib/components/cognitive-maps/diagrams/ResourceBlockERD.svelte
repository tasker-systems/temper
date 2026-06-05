<script lang="ts">
  /**
   * The page-3 HERO: the two kernel primitives a cognitive map is built from,
   * drawn faithfully to the schema artifact (01_schema.sql / 02_functions.sql).
   *
   * A RESOURCE (kb_resources) is the atomic, graph-participating primitive:
   * edges attach to it, properties attach to it, a cogmap points at it. The map's
   * charter IS a resource (kb_cogmaps.telos_resource_id → kb_resources); its
   * doc_type is a kb_properties row (doctype-as-property, not a column); its
   * regulation is the open set of concept-resources it `express`-edges to with
   * label 'operationalized_by' (cogmap_regulation).
   *
   * A CONTENT-BLOCK (kb_content_blocks) is the addressable interior: it lives
   * inside a resource (resource_id), is ordered by seq (block-0 = the telos
   * statement, seq>=1 = the guiding questions), and is a projection of its own
   * correlation-keyed event stream (genesis_event_id / last_event_id) — which is
   * what makes per-block provenance and freshness answerable.
   *
   * `id` namespaces the marker def so multiple instances don't clash.
   */
  let { id = 'resource-block-erd' }: { id?: string } = $props();
</script>

<svg
  viewBox="0 0 720 446"
  xmlns="http://www.w3.org/2000/svg"
  role="img"
  aria-label="An entity-relationship schematic of the two kernel primitives. A resource (kb_resources) is the atomic graph-participant: a cogmap points at the charter resource via telos_resource_id, a kb_properties row carries its doc_type, and a kb_edges express edge labelled operationalized_by reaches a regulation concept-resource. A content block (kb_content_blocks) is the addressable interior, living inside the charter via resource_id, ordered by seq, each a projection of its own event stream."
>
  <defs>
    <marker
      id="{id}-arrow"
      viewBox="0 0 10 10"
      refX="9"
      refY="5"
      markerWidth="7"
      markerHeight="7"
      orient="auto-start-reverse"
    >
      <path d="M0,0 L10,5 L0,10 z" fill="rgba(126,184,218,0.6)" />
    </marker>
  </defs>

  <!-- ════ Band labels: the two primitives ═══════════════════════════════ -->
  <text x="30" y="28" font-family="var(--font-mono)" font-size="9" letter-spacing="2" fill="var(--temper-blue)">RESOURCE · atomic, graph-participating</text>
  <text x="30" y="270" font-family="var(--font-mono)" font-size="9" letter-spacing="2" fill="rgba(255,255,255,0.4)">CONTENT-BLOCK · addressable interior</text>

  <!-- ── Relationship lines (under the boxes) ─────────────────────────────── -->
  <g fill="none" stroke="rgba(126,184,218,0.45)" stroke-width="1" marker-end="url(#{id}-arrow)">
    <!-- cogmap → charter resource (telos_resource_id) -->
    <path d="M 206 84 L 286 84" />
    <!-- charter → regulation concept-resource (express edge) -->
    <path d="M 520 84 L 560 84" />
    <!-- charter ⊃ content blocks (resource_id, contains) -->
    <path d="M 372 156 L 372 286" />
  </g>
  <!-- charter → doc_type property (a property row, dashed = projection) -->
  <path d="M 470 156 L 540 196" fill="none" stroke="var(--rule-2)" stroke-width="1" stroke-dasharray="2 3" />

  <!-- relationship labels -->
  <g font-family="var(--font-mono)" font-size="8.5" letter-spacing="0.4" fill="var(--graphite)">
    <text x="246" y="76" text-anchor="middle">telos_resource_id</text>
    <text x="540" y="76" text-anchor="middle" fill="var(--temper-blue)">express</text>
    <g transform="translate(382,228)">
      <text x="0" y="0" fill="var(--temper-blue)">resource_id</text>
      <text x="0" y="12" fill="var(--graphite-2)">contains · 1 → N</text>
    </g>
  </g>

  <!-- ── kb_cogmaps ───────────────────────────────────────────────────────── -->
  <g>
    <rect x="30" y="52" width="176" height="62" rx="3" fill="var(--obsidian-3)" stroke="var(--rule-2)" stroke-width="1" />
    <text x="42" y="72" font-family="var(--font-mono)" font-size="11" fill="var(--chalk)">kb_cogmaps</text>
    <line x1="30" y1="82" x2="206" y2="82" stroke="var(--rule)" stroke-width="1" />
    <text x="42" y="99" font-family="var(--font-mono)" font-size="8.5" fill="var(--graphite)">name = onboarding</text>
    <text x="42" y="110" font-family="var(--font-mono)" font-size="8.5" fill="var(--graphite-2)">telos_resource_id ↘</text>
  </g>

  <!-- ── kb_resources : the telos CHARTER (the atomic primitive) ──────────── -->
  <g>
    <rect x="286" y="44" width="234" height="112" rx="3" fill="var(--temper-blue-card)" stroke="var(--temper-blue-border)" stroke-width="1.25" />
    <text x="300" y="66" font-family="var(--font-mono)" font-size="11" fill="var(--temper-blue)">kb_resources</text>
    <text x="506" y="66" text-anchor="end" font-family="var(--font-serif)" font-style="italic" font-size="10" fill="var(--parchment)">the charter</text>
    <line x1="286" y1="76" x2="520" y2="76" stroke="var(--temper-blue-border-dim)" stroke-width="1" />
    <text x="300" y="93" font-family="var(--font-mono)" font-size="8.5" fill="var(--graphite)">title · origin_uri</text>
    <text x="300" y="106" font-family="var(--font-mono)" font-size="8.5" fill="var(--graphite)">body_hash  (merkle over its blocks)</text>
    <line x1="286" y1="116" x2="520" y2="116" stroke="var(--rule)" stroke-width="1" />
    <text x="300" y="133" font-family="var(--font-serif)" font-style="italic" font-size="10" fill="var(--chalk)">stands alone · anchors edges &amp; properties</text>
    <text x="300" y="148" font-family="var(--font-serif)" font-style="italic" font-size="10" fill="var(--chalk)">homed in the map</text>
  </g>

  <!-- ── kb_resources : a REGULATION concept (reached by an express edge) ─── -->
  <g>
    <rect x="560" y="50" width="140" height="84" rx="3" fill="var(--obsidian-3)" stroke="var(--temper-blue-border-dim)" stroke-width="1" />
    <text x="572" y="70" font-family="var(--font-mono)" font-size="9.5" fill="var(--temper-blue)">kb_resources</text>
    <line x1="560" y1="80" x2="700" y2="80" stroke="var(--rule)" stroke-width="1" />
    <text x="572" y="96" font-family="var(--font-mono)" font-size="8" fill="var(--graphite-2)">label =</text>
    <text x="572" y="107" font-family="var(--font-mono)" font-size="8" fill="var(--graphite-2)">operationalized_by</text>
    <text x="572" y="125" font-family="var(--font-serif)" font-style="italic" font-size="9.5" fill="var(--chalk)">“pair on the first PR”</text>
  </g>

  <!-- ── kb_properties : doctype-as-property on the resource ──────────────── -->
  <g>
    <rect x="540" y="196" width="160" height="52" rx="3" fill="var(--obsidian-3)" stroke="var(--rule)" stroke-width="1" />
    <text x="552" y="214" font-family="var(--font-mono)" font-size="9.5" fill="var(--graphite)">kb_properties</text>
    <line x1="540" y1="223" x2="700" y2="223" stroke="var(--rule)" stroke-width="1" />
    <text x="552" y="239" font-family="var(--font-mono)" font-size="8" fill="var(--graphite-2)">doc_type = "cogmap_charter"</text>
  </g>

  <!-- ── kb_content_blocks : the addressable, attributable INTERIOR ───────── -->
  <g>
    <rect x="256" y="286" width="234" height="142" rx="3" fill="var(--obsidian-3)" stroke="var(--rule-2)" stroke-width="1.25" />
    <text x="270" y="308" font-family="var(--font-mono)" font-size="11" fill="var(--chalk)">kb_content_blocks</text>
    <text x="476" y="308" text-anchor="end" font-family="var(--font-serif)" font-style="italic" font-size="10" fill="var(--parchment)">the questions</text>
    <line x1="256" y1="318" x2="490" y2="318" stroke="var(--rule)" stroke-width="1" />
    <text x="270" y="335" font-family="var(--font-mono)" font-size="8.5" fill="var(--graphite)">resource_id → charter · seq · is_folded</text>
    <text x="270" y="349" font-family="var(--font-mono)" font-size="8.5" fill="var(--graphite)">genesis_event_id · last_event_id</text>
    <line x1="256" y1="359" x2="490" y2="359" stroke="var(--rule)" stroke-width="1" />
    <text x="270" y="376" font-family="var(--font-serif)" font-style="italic" font-size="9.5" fill="var(--graphite-2)">seq 0 = the telos statement</text>
    <text x="270" y="390" font-family="var(--font-serif)" font-style="italic" font-size="9.5" fill="var(--graphite-2)">seq ≥ 1 = the guiding questions</text>
    <line x1="256" y1="400" x2="490" y2="400" stroke="var(--rule)" stroke-width="1" />
    <text x="270" y="418" font-family="var(--font-serif)" font-style="italic" font-size="10" fill="var(--chalk)">each a projection of its own event stream</text>
  </g>

  <!-- ── The split, named ─────────────────────────────────────────────────── -->
  <text x="566" y="320" font-family="var(--font-mono)" font-size="9" letter-spacing="1.5" fill="var(--temper-blue)">WHY TWO?</text>
  <text x="566" y="340" font-family="var(--font-serif)" font-style="italic" font-size="10.5" fill="var(--chalk)">a resource</text>
  <text x="566" y="355" font-family="var(--font-serif)" font-style="italic" font-size="10.5" fill="var(--chalk)">stands in the graph;</text>
  <text x="566" y="375" font-family="var(--font-serif)" font-style="italic" font-size="10.5" fill="var(--chalk)">a block is addressable</text>
  <text x="566" y="390" font-family="var(--font-serif)" font-style="italic" font-size="10.5" fill="var(--chalk)">inside one, and</text>
  <text x="566" y="405" font-family="var(--font-serif)" font-style="italic" font-size="10.5" fill="var(--chalk)">attributable on its own.</text>
</svg>

<style>
  svg {
    display: block;
    width: 100%;
    height: auto;
  }
</style>
