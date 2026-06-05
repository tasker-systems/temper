<script lang="ts">
  /**
   * The how-maps-relate INLINE: promotion as a curated send-forward across
   * scopes. THREE maps — side-map(team-a) and a team-b map below, an
   * epd-department map above. Each lower map carries a high-salience
   * "sprint rituals" region; the salience markers, visible across each
   * boundary, are the SIGNAL the department map reads. An actor who stands in
   * both lower scopes authors a send-forward event that seeds a new
   * "good sprint process" concept in the department map and draws relationship
   * edges back down to the two source concepts.
   *
   * The promotion ACT is a designed-but-not-yet-built capability: the
   * send-forward event, the new concept, and the relationship edges are drawn
   * PROPOSED — dashed and faint. The built parts — the two source maps and
   * their salience signal — are solid. Source interiors stay behind their
   * boundaries: referenced, not copied.
   *
   * Honest basis: homed resources/edges (kb_resource_homes,
   * kb_edges.home_anchor_*), the team/cogmap join (kb_team_cogmaps,
   * cogmaps_share_a_team), and the cross-boundary salience signal cogmap_shape.
   *
   * `id` namespaces the gradient/filter defs so multiple instances don't clash.
   */
  let { id = 'promotion' }: { id?: string } = $props();
</script>

<svg
  viewBox="0 0 660 460"
  xmlns="http://www.w3.org/2000/svg"
  role="img"
  aria-label="Three cognitive maps: side-map(team-a) and a team-b map below, an epd-department map above. Both lower maps carry a high-salience 'sprint rituals' region whose salience, visible across each boundary, is the signal the department map reads. An actor standing in both lower scopes authors a send-forward event that seeds a proposed new 'good sprint process' concept in the department map, with proposed relationship edges drawn back to the two source concepts. The promotion act is drawn dashed and faint because it is a designed capability not yet built; the source maps and their salience signal are solid. Source interiors stay behind their boundaries."
>
  <defs>
    <!-- Source region density glow (built, solid) -->
    <radialGradient id="{id}-source" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(126,184,218,0.28)" />
      <stop offset="55%" stop-color="rgba(126,184,218,0.11)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0)" />
    </radialGradient>
    <!-- The proposed department concept — fainter still (not yet built) -->
    <radialGradient id="{id}-proposed" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(126,184,218,0.12)" />
      <stop offset="60%" stop-color="rgba(126,184,218,0.04)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0)" />
    </radialGradient>
    <!-- Atmospheric weather per map -->
    <radialGradient id="{id}-weather" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(126,184,218,0.06)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0)" />
    </radialGradient>
    <!-- Members are drawn but fogged — interior stays behind the boundary -->
    <filter id="{id}-fog" x="-50%" y="-50%" width="200%" height="200%">
      <feGaussianBlur stdDeviation="6" />
    </filter>
  </defs>

  <!-- ════ DEPARTMENT MAP (above) — the destination scope ════════════════ -->
  <text x="330" y="26" text-anchor="middle" font-family="var(--font-mono)" font-size="9" letter-spacing="1.5" fill="rgba(255,255,255,0.42)">epd-department map</text>

  <!-- Department boundary (built scope) -->
  <line x1="120" y1="42" x2="540" y2="42" stroke="var(--rule)" stroke-width="1" stroke-dasharray="2 6" opacity="0.7" />

  <ellipse cx="330" cy="110" rx="220" ry="80" fill="url(#{id}-weather)" />

  <!-- The PROPOSED new concept: "good sprint process". Dashed + faint —
       a designed capability, not yet in the artifact. -->
  <g>
    <ellipse cx="330" cy="118" rx="116" ry="64" fill="url(#{id}-proposed)" />
    <ellipse cx="330" cy="118" rx="98" ry="50" fill="none" stroke="rgba(126,184,218,0.45)" stroke-width="1.25" stroke-dasharray="4 5" />
    <text x="330" y="108" text-anchor="middle" font-family="var(--font-serif)" font-size="15" fill="rgba(232,228,223,0.72)">“good sprint process”</text>
    <text x="330" y="130" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="1.5" fill="rgba(126,184,218,0.6)">PROPOSED · homed in department</text>
  </g>

  <!-- ════ THE SEND-FORWARD ACT (proposed) ═══════════════════════════════ -->
  <!-- The actor who stands in both lower scopes, authoring the send-event.
       Drawn dashed/faint: designed capability, not yet built. -->
  <g>
    <!-- actor badge, sitting astride both lower scopes -->
    <circle cx="330" cy="300" r="13" fill="none" stroke="rgba(126,184,218,0.5)" stroke-width="1.25" stroke-dasharray="3 3" />
    <circle cx="330" cy="295" r="4" fill="none" stroke="rgba(126,184,218,0.5)" stroke-width="1" />
    <path d="M 322 309 q 8 -9 16 0" fill="none" stroke="rgba(126,184,218,0.5)" stroke-width="1" />
    <text x="330" y="334" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="1" fill="rgba(126,184,218,0.62)">actor · member on both sides</text>
    <text x="330" y="346" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="1" fill="rgba(126,184,218,0.45)">curates send-forward event</text>
  </g>

  <!-- The send-forward: actor → seeds the new concept upward. Proposed. -->
  <path d="M 330 287 L 330 184" fill="none" stroke="#7eb8da" stroke-width="1.25" stroke-dasharray="4 5" opacity="0.55" />
  <polygon points="330,182 325,193 335,193" fill="#7eb8da" opacity="0.55" />
  <text x="396" y="238" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="1.5" fill="rgba(126,184,218,0.55)">send-forward</text>

  <!-- Relationship edges drawn back DOWN to the two source concepts. Proposed:
       references, not copies — the interiors stay home. -->
  <path d="M 246 132 C 150 200, 130 280, 152 348" fill="none" stroke="#7eb8da" stroke-width="1" stroke-dasharray="3 4" opacity="0.42" />
  <polygon points="152,348 147,338 157,340" fill="#7eb8da" opacity="0.5" />
  <path d="M 414 132 C 510 200, 530 280, 508 348" fill="none" stroke="#7eb8da" stroke-width="1" stroke-dasharray="3 4" opacity="0.42" />
  <polygon points="508,348 503,338 513,340" fill="#7eb8da" opacity="0.5" />
  <text x="330" y="200" text-anchor="middle" font-family="var(--font-mono)" font-size="7" letter-spacing="1" fill="rgba(126,184,218,0.42)">relates back ↓ (referenced, not copied)</text>

  <!-- ════ LOWER-LEFT MAP: side-map(team-a) — a source ═══════════════════ -->
  <text x="150" y="372" text-anchor="middle" font-family="var(--font-mono)" font-size="8.5" letter-spacing="1.2" fill="rgba(255,255,255,0.42)">side-map(team-a)</text>
  <line x1="40" y1="384" x2="260" y2="384" stroke="var(--rule)" stroke-width="1" stroke-dasharray="2 6" opacity="0.7" />

  <ellipse cx="150" cy="416" rx="120" ry="44" fill="url(#{id}-weather)" />
  <g>
    <ellipse cx="150" cy="418" rx="92" ry="34" fill="url(#{id}-source)" />
    <!-- Source interior: fogged members behind the boundary -->
    <g filter="url(#{id}-fog)" fill="#7eb8da">
      <circle cx="124" cy="416" r="4.5" opacity="0.65" />
      <circle cx="160" cy="410" r="4" opacity="0.6" />
      <circle cx="174" cy="426" r="4.5" opacity="0.65" />
      <circle cx="138" cy="430" r="4" opacity="0.6" />
    </g>
    <text x="150" y="408" text-anchor="middle" font-family="var(--font-serif)" font-size="12" fill="#e8e4df">“sprint rituals”</text>
    <!-- Salience signal (built, solid) — visible across the boundary -->
    <g transform="translate(108,438)">
      <text x="0" y="0" font-family="var(--font-mono)" font-size="7" letter-spacing="1" fill="rgba(255,255,255,0.4)">SAL</text>
      <rect x="26" y="-6" width="44" height="3.5" rx="1.75" fill="rgba(255,255,255,0.12)" />
      <rect x="26" y="-6" width="39" height="3.5" rx="1.75" fill="#7eb8da" />
      <text x="76" y="0" font-family="var(--font-mono)" font-size="7" fill="#7eb8da">high</text>
    </g>
  </g>

  <!-- ════ LOWER-RIGHT MAP: team-b — a source ════════════════════════════ -->
  <text x="510" y="372" text-anchor="middle" font-family="var(--font-mono)" font-size="8.5" letter-spacing="1.2" fill="rgba(255,255,255,0.42)">team-b map</text>
  <line x1="400" y1="384" x2="620" y2="384" stroke="var(--rule)" stroke-width="1" stroke-dasharray="2 6" opacity="0.7" />

  <ellipse cx="510" cy="416" rx="120" ry="44" fill="url(#{id}-weather)" />
  <g>
    <ellipse cx="510" cy="418" rx="92" ry="34" fill="url(#{id}-source)" />
    <g filter="url(#{id}-fog)" fill="#7eb8da">
      <circle cx="484" cy="416" r="4.5" opacity="0.65" />
      <circle cx="520" cy="410" r="4" opacity="0.6" />
      <circle cx="534" cy="426" r="4.5" opacity="0.65" />
      <circle cx="498" cy="430" r="4" opacity="0.6" />
    </g>
    <text x="510" y="408" text-anchor="middle" font-family="var(--font-serif)" font-size="12" fill="#e8e4df">“sprint rituals”</text>
    <g transform="translate(468,438)">
      <text x="0" y="0" font-family="var(--font-mono)" font-size="7" letter-spacing="1" fill="rgba(255,255,255,0.4)">SAL</text>
      <rect x="26" y="-6" width="44" height="3.5" rx="1.75" fill="rgba(255,255,255,0.12)" />
      <rect x="26" y="-6" width="41" height="3.5" rx="1.75" fill="#7eb8da" />
      <text x="76" y="0" font-family="var(--font-mono)" font-size="7" fill="#7eb8da">high</text>
    </g>
  </g>

  <!-- ════ THE SALIENCE SIGNAL (built, solid) ════════════════════════════ -->
  <!-- Both lower maps' high salience, visible across the boundary, is the
       signal the department map reads. Solid lines — this is the built part. -->
  <path d="M 150 388 L 150 360 Q 150 348 168 344 L 300 320" fill="none" stroke="rgba(126,184,218,0.4)" stroke-width="0.9" />
  <path d="M 510 388 L 510 360 Q 510 348 492 344 L 360 320" fill="none" stroke="rgba(126,184,218,0.4)" stroke-width="0.9" />
  <text x="252" y="296" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="1.5" fill="rgba(126,184,218,0.6)">mutual high salience = signal ↑</text>
</svg>

<style>
  svg {
    display: block;
    width: 100%;
    height: auto;
  }
</style>
