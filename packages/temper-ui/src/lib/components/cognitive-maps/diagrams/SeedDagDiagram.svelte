<script lang="ts">
  /**
   * The page-6 HERO: the seed teams DAG read two ways. On the left, the six
   * teams as a directed acyclic graph — `temper-system` (root) above
   * {org-common, epd-department, directors}, with epd-team-a and epd-team-b
   * each descending from *both* epd-department and org-common. Edges point
   * down-DAG (a child "descends-from" its parents).
   *
   * Alongside it, the two orthogonal axes the page is about: visibility is
   * their product. The PERMISSION axis (consumer): grants inherit *down* the
   * DAG, a person reading what reaches them. The PRECEDENCE axis (producer /
   * salience): within what's permitted, what ranks. The two are drawn at right
   * angles to make their independence visible — one is structural reach, the
   * other graded ranking.
   *
   * `id` namespaces the marker/gradient defs so multiple instances don't clash.
   */
  let { id = 'seed-dag' }: { id?: string } = $props();
</script>

<svg
  viewBox="0 0 660 400"
  xmlns="http://www.w3.org/2000/svg"
  role="img"
  aria-label="The seed teams as a directed acyclic graph — temper-system root above org-common, epd-department and directors, with epd-team-a and epd-team-b each descending from epd-department and org-common — shown beside two orthogonal axes: permission (grants inheriting down the DAG) and precedence (graded ranking within what's permitted). Visibility is their product."
>
  <defs>
    <!-- Down-DAG edge arrowhead (descends-from) -->
    <marker
      id="{id}-arrow"
      viewBox="0 0 10 10"
      refX="8.5"
      refY="5"
      markerWidth="6"
      markerHeight="6"
      orient="auto-start-reverse"
    >
      <path d="M0,1 L9,5 L0,9 z" fill="rgba(126,184,218,0.6)" />
    </marker>
    <!-- Axis arrowhead -->
    <marker
      id="{id}-axis-arrow"
      viewBox="0 0 10 10"
      refX="8.5"
      refY="5"
      markerWidth="7"
      markerHeight="7"
      orient="auto"
    >
      <path d="M0,1 L9,5 L0,9 z" fill="#7eb8da" />
    </marker>
    <!-- Salience graded fill for the precedence axis -->
    <linearGradient id="{id}-rank" x1="0" y1="1" x2="0" y2="0">
      <stop offset="0%" stop-color="rgba(126,184,218,0.15)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0.85)" />
    </linearGradient>
  </defs>

  <!-- ════ LEFT: the teams DAG ═══════════════════════════════════════════ -->
  <g>
    <text
      x="190" y="28"
      text-anchor="middle"
      font-family="var(--font-mono)"
      font-size="8.5"
      letter-spacing="2.5"
      fill="rgba(255,255,255,0.4)"
    >THE SEED · SIX TEAMS</text>

    <!-- Edges (drawn first, behind nodes) · child → parent, arrow points up the DAG to the parent it descends from -->
    <g stroke="rgba(126,184,218,0.4)" stroke-width="1.25" fill="none" marker-end="url(#{id}-arrow)">
      <!-- root tier ← second tier -->
      <line x1="100" y1="120" x2="172" y2="76" />
      <line x1="190" y1="118" x2="190" y2="74" />
      <line x1="282" y1="120" x2="210" y2="76" />
      <!-- epd-team-a descends from epd-department AND org-common -->
      <line x1="120" y1="316" x2="92" y2="138" />
      <line x1="138" y1="316" x2="186" y2="138" />
      <!-- epd-team-b descends from epd-department AND org-common -->
      <line x1="252" y1="316" x2="200" y2="138" />
      <line x1="266" y1="316" x2="106" y2="138" />
    </g>

    <!-- Root -->
    <g>
      <rect x="138" y="48" width="104" height="26" rx="4" fill="rgba(126,184,218,0.10)" stroke="#7eb8da" stroke-width="1" />
      <text x="190" y="65" text-anchor="middle" font-family="var(--font-mono)" font-size="9.5" fill="#e8e4df">temper-system</text>
    </g>

    <!-- Second tier -->
    <g>
      <rect x="48" y="120" width="92" height="24" rx="4" fill="rgba(126,184,218,0.06)" stroke="rgba(126,184,218,0.45)" stroke-width="0.85" />
      <text x="94" y="136" text-anchor="middle" font-family="var(--font-mono)" font-size="8.5" fill="#e8e4df">org-common</text>

      <rect x="152" y="120" width="100" height="24" rx="4" fill="rgba(126,184,218,0.06)" stroke="rgba(126,184,218,0.45)" stroke-width="0.85" />
      <text x="202" y="136" text-anchor="middle" font-family="var(--font-mono)" font-size="8.5" fill="#e8e4df">epd-department</text>

      <rect x="264" y="120" width="78" height="24" rx="4" fill="rgba(126,184,218,0.06)" stroke="rgba(126,184,218,0.45)" stroke-width="0.85" />
      <text x="303" y="136" text-anchor="middle" font-family="var(--font-mono)" font-size="8.5" fill="#e8e4df">directors</text>
    </g>

    <!-- Leaf tier: the two teams that descend from two parents each -->
    <g>
      <rect x="70" y="320" width="86" height="24" rx="4" fill="rgba(126,184,218,0.06)" stroke="rgba(126,184,218,0.45)" stroke-width="0.85" />
      <text x="113" y="336" text-anchor="middle" font-family="var(--font-mono)" font-size="8.5" fill="#e8e4df">epd-team-a</text>

      <rect x="218" y="320" width="86" height="24" rx="4" fill="rgba(126,184,218,0.06)" stroke="rgba(126,184,218,0.45)" stroke-width="0.85" />
      <text x="261" y="336" text-anchor="middle" font-family="var(--font-mono)" font-size="8.5" fill="#e8e4df">epd-team-b</text>
    </g>

    <text
      x="190" y="372"
      text-anchor="middle"
      font-family="var(--font-serif)"
      font-style="italic"
      font-size="11"
      fill="rgba(255,255,255,0.45)"
    >every team descends from the root</text>
  </g>

  <!-- Divider between the structure and the two readings of it -->
  <line x1="396" y1="52" x2="396" y2="356" stroke="rgba(255,255,255,0.12)" stroke-width="0.75" />

  <!-- ════ RIGHT: the two orthogonal axes ════════════════════════════════ -->
  <g transform="translate(440,0)">
    <text
      x="100" y="28"
      text-anchor="middle"
      font-family="var(--font-mono)"
      font-size="8.5"
      letter-spacing="2.5"
      fill="rgba(255,255,255,0.4)"
    >VISIBILITY = PERMISSION × PRECEDENCE</text>

    <!-- Origin of the two axes -->
    <!-- PERMISSION axis: horizontal — grants reach down the DAG (structural yes/no) -->
    <line x1="36" y1="252" x2="196" y2="252" stroke="#7eb8da" stroke-width="1.5" marker-end="url(#{id}-axis-arrow)" />
    <text x="116" y="276" text-anchor="middle" font-family="var(--font-serif)" font-size="12.5" fill="#7eb8da">permission</text>
    <text x="116" y="291" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.5" fill="rgba(255,255,255,0.42)">what's in your set — down the DAG</text>

    <!-- PRECEDENCE axis: vertical — graded ranking within the permitted set -->
    <line x1="36" y1="252" x2="36" y2="84" stroke="#7eb8da" stroke-width="1.5" marker-end="url(#{id}-axis-arrow)" />
    <text x="22" y="168" text-anchor="middle" font-family="var(--font-serif)" font-size="12.5" fill="#7eb8da" transform="rotate(-90 22 168)">precedence</text>
    <text x="38" y="168" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.5" fill="rgba(255,255,255,0.42)" transform="rotate(-90 38 168)">within it — what ranks</text>

    <!-- Right-angle mark, making the orthogonality explicit -->
    <path d="M36,238 L50,238 L50,252" fill="none" stroke="rgba(255,255,255,0.3)" stroke-width="0.75" />

    <!-- Graded ranking bars climbing the precedence axis (salience) -->
    <g>
      <rect x="62" y="216" width="118" height="9" rx="2" fill="url(#{id}-rank)" opacity="0.45" />
      <rect x="62" y="178" width="90" height="9" rx="2" fill="url(#{id}-rank)" opacity="0.65" />
      <rect x="62" y="140" width="64" height="9" rx="2" fill="url(#{id}-rank)" opacity="0.85" />
      <rect x="62" y="102" width="40" height="9" rx="2" fill="url(#{id}-rank)" />
    </g>

    <text
      x="100" y="332"
      text-anchor="middle"
      font-family="var(--font-serif)"
      font-style="italic"
      font-size="11"
      fill="rgba(255,255,255,0.45)"
    >two independent answers</text>
  </g>
</svg>

<style>
  svg {
    display: block;
    width: 100%;
    height: auto;
  }
</style>
