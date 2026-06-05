<script lang="ts">
  /**
   * The how-maps-relate HERO: two cognitive maps with a boundary between them,
   * drawn to make legible partiality felt. The left map (side-map) projects its
   * SHAPE across the boundary — a soft density region carrying a label, a
   * salience weight, and a member_count blur — rendered as something the right
   * map (team-b) can SEE. The MATERIAL INTERIOR — the actual member resources —
   * stays drawn on the far side, behind the boundary, deliberately unreadable
   * across it. The asymmetry is the whole point: shape crosses the boundary,
   * interior does not.
   *
   * Honest basis: cogmap_shape() (salience / label / member_count, never member
   * identities) over kb_cogmap_regions / kb_cogmap_region_members, and the
   * shared-team relation cogmaps_share_a_team.
   *
   * `id` namespaces the gradient/filter defs so multiple instances don't clash.
   */
  let { id = 'shape-boundary' }: { id?: string } = $props();
</script>

<svg
  viewBox="0 0 660 380"
  xmlns="http://www.w3.org/2000/svg"
  role="img"
  aria-label="Two cognitive maps separated by a boundary. The left map's salience-weighted 'sprint rituals' region projects its shape across the boundary as something the right map can see; the left map's actual member resources stay behind the boundary, fogged and unreadable across it. Shape crosses, interior does not."
>
  <defs>
    <!-- The projecting region's density glow (shape that crosses) -->
    <radialGradient id="{id}-shape" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(126,184,218,0.30)" />
      <stop offset="55%" stop-color="rgba(126,184,218,0.12)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0)" />
    </radialGradient>
    <!-- The same shape echoed across the boundary, fainter — what team-b sees -->
    <radialGradient id="{id}-echo" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(126,184,218,0.16)" />
      <stop offset="60%" stop-color="rgba(126,184,218,0.05)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0)" />
    </radialGradient>
    <!-- Large soft atmospheric washes — the "weather" of each map -->
    <radialGradient id="{id}-weather" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(126,184,218,0.06)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0)" />
    </radialGradient>
    <!-- Members are drawn but fogged — present on the far side, not legible -->
    <filter id="{id}-fog" x="-40%" y="-40%" width="180%" height="180%">
      <feGaussianBlur stdDeviation="4.5" />
    </filter>
    <!-- Heavier fog for the interior that does NOT cross the boundary -->
    <filter id="{id}-fog-heavy" x="-60%" y="-60%" width="220%" height="220%">
      <feGaussianBlur stdDeviation="8" />
    </filter>
  </defs>

  <!-- ── The boundary between the two maps ────────────────────────────── -->
  <line x1="330" y1="44" x2="330" y2="336" stroke="var(--rule)" stroke-width="1" stroke-dasharray="2 6" opacity="0.85" />
  <text x="330" y="32" text-anchor="middle" font-family="var(--font-mono)" font-size="8" letter-spacing="2" fill="rgba(255,255,255,0.30)">BOUNDARY</text>

  <!-- ── LEFT MAP: side-map(team-a) — projects its shape ──────────────── -->
  <text x="150" y="32" text-anchor="middle" font-family="var(--font-mono)" font-size="9" letter-spacing="1.5" fill="rgba(255,255,255,0.42)">side-map(team-a)</text>

  <!-- Atmospheric ground for the left map -->
  <ellipse cx="150" cy="200" rx="200" ry="170" fill="url(#{id}-weather)" />

  <!-- Substrate: scattered, faint concept-points across the left field -->
  <g fill="#e8e4df">
    <circle cx="50" cy="80" r="1.6" opacity="0.13" />
    <circle cx="90" cy="320" r="1.6" opacity="0.12" />
    <circle cx="270" cy="300" r="1.6" opacity="0.11" />
    <circle cx="40" cy="220" r="1.4" opacity="0.11" />
    <circle cx="220" cy="70" r="1.6" opacity="0.12" />
    <circle cx="180" cy="330" r="1.4" opacity="0.10" />
  </g>

  <!-- The projecting region: "sprint rituals", high salience -->
  <g>
    <ellipse cx="150" cy="206" rx="120" ry="100" fill="url(#{id}-shape)" />

    <!-- Region label -->
    <text
      x="150" y="118"
      text-anchor="middle"
      font-family="var(--font-serif)"
      font-size="16"
      fill="#e8e4df"
    >“sprint rituals”</text>

    <!-- Salience weight bar -->
    <g transform="translate(96,134)">
      <text x="0" y="0" font-family="var(--font-mono)" font-size="8" letter-spacing="1.5" fill="rgba(255,255,255,0.45)">SALIENCE</text>
      <rect x="56" y="-7" width="56" height="4" rx="2" fill="rgba(255,255,255,0.12)" />
      <rect x="56" y="-7" width="50" height="4" rx="2" fill="#7eb8da" />
      <text x="118" y="0" font-family="var(--font-mono)" font-size="8" letter-spacing="1" fill="#7eb8da">high</text>
    </g>

    <!-- The MATERIAL INTERIOR: actual members, heavily fogged. These do NOT
         cross the boundary — they sit on the far (left) side, unreadable. -->
    <g filter="url(#{id}-fog-heavy)" fill="#7eb8da">
      <circle cx="118" cy="190" r="5.5" opacity="0.7" />
      <circle cx="166" cy="178" r="5" opacity="0.65" />
      <circle cx="188" cy="216" r="5.5" opacity="0.7" />
      <circle cx="134" cy="234" r="5" opacity="0.65" />
      <circle cx="100" cy="222" r="4.5" opacity="0.6" />
      <circle cx="172" cy="248" r="5" opacity="0.65" />
      <circle cx="150" cy="206" r="6" opacity="0.75" />
    </g>

    <!-- member_count blur readout — what you get is the count, not the contents -->
    <text x="150" y="304" text-anchor="middle" font-family="var(--font-mono)" font-size="9" letter-spacing="1" fill="rgba(255,255,255,0.5)">
      ≈ 7 members · interior stays home
    </text>
  </g>

  <!-- ── The shape crossing the boundary ──────────────────────────────── -->
  <!-- An arc carrying the shape (not the members) rightward across the line -->
  <path d="M 270 196 C 320 188, 372 188, 420 196" fill="none" stroke="#7eb8da" stroke-width="1" opacity="0.4" />
  <polygon points="420,196 410,191 411,201" fill="#7eb8da" opacity="0.55" />
  <text x="345" y="178" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="1.5" fill="rgba(126,184,218,0.6)">shape →</text>

  <!-- ── RIGHT MAP: team-b — sees the projected shape ─────────────────── -->
  <text x="510" y="32" text-anchor="middle" font-family="var(--font-mono)" font-size="9" letter-spacing="1.5" fill="rgba(255,255,255,0.42)">team-b map</text>

  <!-- Atmospheric ground for the right map -->
  <ellipse cx="510" cy="200" rx="170" ry="160" fill="url(#{id}-weather)" />
  <g fill="#e8e4df">
    <circle cx="620" cy="90" r="1.6" opacity="0.12" />
    <circle cx="580" cy="320" r="1.6" opacity="0.11" />
    <circle cx="450" cy="300" r="1.4" opacity="0.10" />
    <circle cx="640" cy="220" r="1.4" opacity="0.11" />
  </g>

  <!-- What team-b sees of side-map(team-a): the SHAPE only — outline + weight.
       No members are drawn here. The echo is the salience made visible. -->
  <g>
    <ellipse cx="500" cy="206" rx="96" ry="82" fill="url(#{id}-echo)" />
    <!-- Outline-only: the shape, present without contents -->
    <ellipse cx="500" cy="206" rx="78" ry="66" fill="none" stroke="rgba(126,184,218,0.35)" stroke-width="1" stroke-dasharray="3 4" />
    <text x="500" y="124" text-anchor="middle" font-family="var(--font-serif)" font-size="13" fill="rgba(232,228,223,0.7)">“sprint rituals”</text>
    <text x="500" y="142" text-anchor="middle" font-family="var(--font-mono)" font-size="8" letter-spacing="1" fill="rgba(126,184,218,0.7)">salience high · ≈ 7</text>
    <text x="500" y="300" text-anchor="middle" font-family="var(--font-mono)" font-size="8" letter-spacing="1" fill="rgba(255,255,255,0.4)">
      seen, not read
    </text>
  </g>
</svg>

<style>
  svg {
    display: block;
    width: 100%;
    height: auto;
  }
</style>
