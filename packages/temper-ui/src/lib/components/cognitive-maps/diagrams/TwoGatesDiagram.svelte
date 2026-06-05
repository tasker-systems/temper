<script lang="ts">
  /**
   * The page-6 INLINE panel: the two gates easiest to conflate, side by side,
   * as two differently-treated arrows into the *same* map.
   *
   * Gate 1 — membership-based shape-readability: `cogmap_readable_by_profile`
   * ("are you a member of a team joined to this map") gates seeing the map's
   * SHAPE. Gate 2 — down-DAG grant-inheritance: `resources_visible_to`
   * ("has a resource been granted to a team you reach") gates reading
   * RESOURCES. Different questions, different functions, different answers —
   * the panel's whole job is to keep them from blurring. One is a solid arrow
   * (membership), the other dashed (grant-inheritance); both terminate at the
   * one map, but at different things inside it.
   *
   * `id` namespaces the marker defs so multiple instances don't clash.
   */
  let { id = 'two-gates' }: { id?: string } = $props();
</script>

<svg
  viewBox="0 0 660 320"
  xmlns="http://www.w3.org/2000/svg"
  role="img"
  aria-label="Two gates into the same map, kept apart. The first, cogmap_readable_by_profile, asks whether you are a member of a team joined to this map and gates seeing the map's shape. The second, resources_visible_to, asks whether a resource has been granted to a team you reach and gates reading the map's resources. Different questions, different functions, different answers."
>
  <defs>
    <!-- Gate 1 arrowhead — solid, membership -->
    <marker
      id="{id}-arrow-shape"
      viewBox="0 0 10 10"
      refX="8.5"
      refY="5"
      markerWidth="7"
      markerHeight="7"
      orient="auto"
    >
      <path d="M0,1 L9,5 L0,9 z" fill="#7eb8da" />
    </marker>
    <!-- Gate 2 arrowhead — dimmer, grant-inheritance -->
    <marker
      id="{id}-arrow-res"
      viewBox="0 0 10 10"
      refX="8.5"
      refY="5"
      markerWidth="7"
      markerHeight="7"
      orient="auto"
    >
      <path d="M0,1 L9,5 L0,9 z" fill="rgba(126,184,218,0.55)" />
    </marker>
  </defs>

  <!-- ════ The one map, on the right — what both gates point into ════════ -->
  <g>
    <rect x="438" y="74" width="184" height="172" rx="6" fill="rgba(126,184,218,0.05)" stroke="#7eb8da" stroke-width="1" />
    <text x="530" y="62" text-anchor="middle" font-family="var(--font-mono)" font-size="8.5" letter-spacing="2" fill="rgba(255,255,255,0.4)">ONE MAP</text>

    <!-- Inner target 1: SHAPE (top half) -->
    <rect x="460" y="98" width="140" height="52" rx="4" fill="rgba(126,184,218,0.10)" stroke="rgba(126,184,218,0.5)" stroke-width="0.85" />
    <text x="530" y="120" text-anchor="middle" font-family="var(--font-serif)" font-size="13" fill="#e8e4df">the shape</text>
    <text x="530" y="138" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.5" fill="rgba(255,255,255,0.42)">regions · salience · counts</text>

    <!-- Inner target 2: RESOURCES (bottom half) -->
    <rect x="460" y="168" width="140" height="56" rx="4" fill="rgba(126,184,218,0.05)" stroke="rgba(126,184,218,0.32)" stroke-width="0.85" />
    <text x="530" y="190" text-anchor="middle" font-family="var(--font-serif)" font-size="13" fill="#e8e4df">the resources</text>
    <text x="530" y="208" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.5" fill="rgba(255,255,255,0.42)">the material inside</text>
  </g>

  <!-- ════ GATE 1: membership → shape (solid) ════════════════════════════ -->
  <g>
    <text x="40" y="96" font-family="var(--font-mono)" font-size="8.5" letter-spacing="1.5" fill="#7eb8da">GATE 1 · SHAPE-READABILITY</text>
    <text x="40" y="118" font-family="var(--font-serif)" font-style="italic" font-size="12" fill="#e8e4df">“are you a member of a team</text>
    <text x="40" y="135" font-family="var(--font-serif)" font-style="italic" font-size="12" fill="#e8e4df">joined to this map?”</text>
    <text x="40" y="156" font-family="var(--font-mono)" font-size="9.5" fill="#7eb8da">cogmap_readable_by_profile()</text>

    <!-- Solid arrow into the SHAPE target -->
    <line x1="290" y1="124" x2="456" y2="124" stroke="#7eb8da" stroke-width="1.75" marker-end="url(#{id}-arrow-shape)" />
  </g>

  <!-- ════ GATE 2: grant-inheritance → resources (dashed) ════════════════ -->
  <g>
    <text x="40" y="214" font-family="var(--font-mono)" font-size="8.5" letter-spacing="1.5" fill="rgba(126,184,218,0.7)">GATE 2 · GRANT-INHERITANCE</text>
    <text x="40" y="236" font-family="var(--font-serif)" font-style="italic" font-size="12" fill="#e8e4df">“has a resource been granted</text>
    <text x="40" y="253" font-family="var(--font-serif)" font-style="italic" font-size="12" fill="#e8e4df">to a team you reach?”</text>
    <text x="40" y="274" font-family="var(--font-mono)" font-size="9.5" fill="rgba(126,184,218,0.85)">resources_visible_to()</text>

    <!-- Dashed arrow into the RESOURCES target -->
    <line x1="290" y1="196" x2="456" y2="196" stroke="rgba(126,184,218,0.55)" stroke-width="1.75" stroke-dasharray="6 4" marker-end="url(#{id}-arrow-res)" />
  </g>

  <!-- The point of the panel, stated -->
  <text
    x="330" y="308"
    text-anchor="middle"
    font-family="var(--font-serif)"
    font-style="italic"
    font-size="11.5"
    fill="rgba(255,255,255,0.45)"
  >different questions · different functions · different answers</text>
</svg>

<style>
  svg {
    display: block;
    width: 100%;
    height: auto;
  }
</style>
