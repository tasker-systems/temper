<script lang="ts">
  /**
   * The page-1 HERO: a cognitive map drawn honestly — an emergent field of
   * concept-points that has settled into density regions, each carrying a
   * label, a salience weight, and a member_count blur. The members themselves
   * are fogged (drawn, but deliberately not legible): you see *that* roughly N
   * things cluster here, not *what* they are. A shape with weight and weather,
   * not a walled garden with a gate.
   *
   * Honest basis: kb_cogmap_regions (centroid / salience / label /
   * member_count) and cogmap_shape(), which returns salience/label/count and
   * never member identities.
   *
   * `id` namespaces the gradient/filter defs so multiple instances don't clash.
   */
  let { id = 'region-field' }: { id?: string } = $props();
</script>

<svg
  viewBox="0 0 660 380"
  xmlns="http://www.w3.org/2000/svg"
  role="img"
  aria-label="A cognitive map as a field of concept-points settled into one strong density region labelled 'first-week confidence' with high salience and a fogged member count, and a second fainter region forming"
>
  <defs>
    <!-- The strong region's density glow -->
    <radialGradient id="{id}-primary" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(126,184,218,0.30)" />
      <stop offset="55%" stop-color="rgba(126,184,218,0.12)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0)" />
    </radialGradient>
    <!-- The nascent second region, fainter -->
    <radialGradient id="{id}-secondary" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(126,184,218,0.16)" />
      <stop offset="60%" stop-color="rgba(126,184,218,0.05)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0)" />
    </radialGradient>
    <!-- Large soft atmospheric washes — the "weather" -->
    <radialGradient id="{id}-weather" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(126,184,218,0.06)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0)" />
    </radialGradient>
    <!-- Members are drawn but fogged — present, not legible -->
    <filter id="{id}-fog" x="-40%" y="-40%" width="180%" height="180%">
      <feGaussianBlur stdDeviation="4.5" />
    </filter>
  </defs>

  <!-- Atmospheric ground washes -->
  <ellipse cx="250" cy="200" rx="320" ry="240" fill="url(#{id}-weather)" />
  <ellipse cx="500" cy="150" rx="200" ry="160" fill="url(#{id}-weather)" />

  <!-- Substrate: scattered, faint concept-points across the field -->
  <g fill="#e8e4df">
    <circle cx="60" cy="70" r="1.6" opacity="0.14" />
    <circle cx="120" cy="320" r="1.6" opacity="0.13" />
    <circle cx="600" cy="300" r="1.6" opacity="0.12" />
    <circle cx="560" cy="60" r="1.6" opacity="0.14" />
    <circle cx="40" cy="200" r="1.6" opacity="0.12" />
    <circle cx="330" cy="40" r="1.6" opacity="0.13" />
    <circle cx="420" cy="340" r="1.6" opacity="0.13" />
    <circle cx="640" cy="200" r="1.6" opacity="0.11" />
    <circle cx="200" cy="50" r="1.6" opacity="0.12" />
    <circle cx="90" cy="270" r="1.4" opacity="0.10" />
    <circle cx="300" cy="330" r="1.4" opacity="0.11" />
    <circle cx="540" cy="330" r="1.4" opacity="0.10" />
  </g>

  <!-- ── Primary region: "first-week confidence", high salience ───────── -->
  <g>
    <ellipse cx="248" cy="206" rx="150" ry="120" fill="url(#{id}-primary)" />

    <!-- Fogged members: you see THAT ~7 cluster here, not what they are -->
    <g filter="url(#{id}-fog)" fill="#7eb8da">
      <circle cx="214" cy="188" r="5.5" opacity="0.85" />
      <circle cx="262" cy="176" r="5" opacity="0.8" />
      <circle cx="284" cy="214" r="5.5" opacity="0.85" />
      <circle cx="232" cy="232" r="5" opacity="0.8" />
      <circle cx="196" cy="222" r="4.5" opacity="0.75" />
      <circle cx="270" cy="246" r="5" opacity="0.8" />
      <circle cx="246" cy="206" r="6" opacity="0.9" />
    </g>

    <!-- Region label -->
    <text
      x="248" y="108"
      text-anchor="middle"
      font-family="var(--font-serif)"
      font-size="17"
      fill="#e8e4df"
    >“first-week confidence”</text>

    <!-- Salience weight bar -->
    <g transform="translate(188,126)">
      <text x="0" y="0" font-family="var(--font-mono)" font-size="8.5" letter-spacing="1.5" fill="rgba(255,255,255,0.45)">SALIENCE</text>
      <rect x="60" y="-7" width="72" height="4" rx="2" fill="rgba(255,255,255,0.12)" />
      <rect x="60" y="-7" width="62" height="4" rx="2" fill="#7eb8da" />
      <text x="140" y="0" font-family="var(--font-mono)" font-size="8.5" letter-spacing="1" fill="#7eb8da">high</text>
    </g>

    <!-- member_count blur readout -->
    <text x="248" y="306" text-anchor="middle" font-family="var(--font-mono)" font-size="9" letter-spacing="1" fill="rgba(255,255,255,0.5)">
      ≈ 7 members · identities withheld
    </text>
  </g>

  <!-- ── Secondary region: forming, lower salience, unlabelled ────────── -->
  <g>
    <ellipse cx="492" cy="168" rx="92" ry="78" fill="url(#{id}-secondary)" />
    <g filter="url(#{id}-fog)" fill="#7eb8da" opacity="0.55">
      <circle cx="476" cy="156" r="4.5" />
      <circle cx="508" cy="176" r="4" />
      <circle cx="492" cy="190" r="3.5" />
    </g>
    <text x="492" y="96" text-anchor="middle" font-family="var(--font-mono)" font-size="8.5" letter-spacing="1.5" fill="rgba(255,255,255,0.3)">
      a region forming
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
