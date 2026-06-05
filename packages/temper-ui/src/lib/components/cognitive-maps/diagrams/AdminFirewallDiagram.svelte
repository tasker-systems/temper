<script lang="ts">
  /**
   * Two surfaces over one ledger, drawn honestly.
   *
   * AUTHORING (left) is built and invariant: an MCP call into `cogmap_genesis`
   * writes a new map *inside* existing boundaries — so it's drawn solid.
   *
   * The ADMINISTRATIVE surface (right) reshapes the access graph beneath every
   * map. Its *shape* is organization-specific — how separated / audited /
   * authenticated it must be — so it's drawn as a proposed DIAL from a thin
   * operator surface (minimal) to a separated, SAML-backed plane (guarded),
   * and the whole thing is dashed: unbuilt.
   *
   * Both write EVENTS to one ledger. But administrative events branch into a
   * FIREWALLED compliance-audit stream — a separate channel that by design does
   * NOT feed cognitive maps / subscriptions / relationships. And a dashed line
   * at the bottom marks the Postgres responsibility boundary: a command issued
   * straight to the database falls below the ledger entirely.
   *
   * `id` namespaces the marker/gradient defs so multiple instances don't clash.
   */
  let { id = 'admin-firewall' }: { id?: string } = $props();
</script>

<svg
  viewBox="0 0 660 420"
  xmlns="http://www.w3.org/2000/svg"
  role="img"
  aria-label="Two surfaces over one event ledger: authoring via cogmap_genesis (built, solid) on the left and a proposed, organization-shaped administrative dial (dashed, ranging from minimal to guarded) on the right. Both emit events, but administrative events branch into a firewalled compliance-audit stream that does not feed the cognitive maps, subscriptions, or relationships. A dashed line at the bottom marks the Postgres responsibility boundary, below which commands can bypass the ledger."
>
  <defs>
    <!-- Solid arrowhead — built paths (authoring, the ledger spine) -->
    <marker id="{id}-arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M0,1 L9,5 L0,9 z" fill="#7eb8da" opacity="0.85" />
    </marker>
    <!-- Faint arrowhead — proposed paths (admin → events) -->
    <marker id="{id}-arrow-dim" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M0,1 L9,5 L0,9 z" fill="rgba(126,184,218,0.35)" />
    </marker>
    <!-- The compliance dial sweep, faint -->
    <linearGradient id="{id}-dial" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0%" stop-color="rgba(126,184,218,0.10)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0.40)" />
    </linearGradient>
  </defs>

  <!-- ─────────────────────────── TOP: TWO SURFACES ─────────────────────── -->

  <!-- ── Authoring surface (left) — BUILT, solid ──────────────────────────── -->
  <g>
    <text x="150" y="26" text-anchor="middle" font-family="var(--font-mono)" font-size="8.5" letter-spacing="2.5" fill="rgba(255,255,255,0.45)">AUTHORING</text>
    <text x="150" y="40" text-anchor="middle" font-family="var(--font-mono)" font-size="7" letter-spacing="1.5" fill="#9ed3af">BUILT</text>

    <rect x="58" y="52" width="184" height="64" rx="3" fill="rgba(126,184,218,0.08)" stroke="#7eb8da" stroke-width="1.1" />
    <text x="150" y="78" text-anchor="middle" font-family="var(--font-mono)" font-size="12" fill="#e8e4df">cogmap_genesis</text>
    <text x="150" y="98" text-anchor="middle" font-family="var(--font-serif)" font-style="italic" font-size="10" fill="rgba(255,255,255,0.5)">a new map, inside the boundaries</text>
  </g>

  <!-- ── Administrative surface (right) — PROPOSED dial, dashed ───────────── -->
  <g>
    <text x="500" y="26" text-anchor="middle" font-family="var(--font-mono)" font-size="8.5" letter-spacing="2.5" fill="rgba(255,255,255,0.45)">ADMINISTRATION</text>
    <text x="500" y="40" text-anchor="middle" font-family="var(--font-mono)" font-size="7" letter-spacing="1.5" fill="rgba(126,184,218,0.55)">PROPOSED · ORG-SHAPED</text>

    <rect x="408" y="52" width="184" height="64" rx="3" fill="none" stroke="rgba(126,184,218,0.45)" stroke-width="1.1" stroke-dasharray="5 4" />
    <text x="500" y="74" text-anchor="middle" font-family="var(--font-serif)" font-size="10.5" fill="rgba(232,228,223,0.78)">reshape the access graph</text>
    <text x="500" y="90" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.4" fill="rgba(255,255,255,0.42)">profiles · teams · DAG · team↔map</text>

    <!-- The dial: minimal → guarded -->
    <path d="M428 110 A 72 72 0 0 1 572 110" fill="none" stroke="rgba(126,184,218,0.18)" stroke-width="3.5" />
    <path d="M428 110 A 72 72 0 0 1 572 110" fill="none" stroke="url(#{id}-dial)" stroke-width="3.5" stroke-dasharray="4 3" />
    <!-- dial needle, sitting low (temperkb.io: minimal end) -->
    <line x1="500" y1="110" x2="448" y2="92" stroke="#7eb8da" stroke-width="1.4" opacity="0.85" />
    <circle cx="500" cy="110" r="2.4" fill="#7eb8da" opacity="0.85" />
    <text x="424" y="124" text-anchor="start" font-family="var(--font-mono)" font-size="7" letter-spacing="0.6" fill="rgba(255,255,255,0.4)">minimal</text>
    <text x="576" y="124" text-anchor="end" font-family="var(--font-mono)" font-size="7" letter-spacing="0.6" fill="rgba(255,255,255,0.4)">guarded · SAML</text>
  </g>

  <!-- ─────────────────────── EVENTS DOWN INTO LEDGER ───────────────────── -->

  <!-- Authoring → ledger (solid, built) -->
  <line x1="150" y1="116" x2="150" y2="176" stroke="#7eb8da" stroke-width="1.1" marker-end="url(#{id}-arrow)" />
  <text x="160" y="150" text-anchor="start" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.5" fill="rgba(255,255,255,0.42)">event</text>

  <!-- Administration → ledger (faint, proposed) -->
  <line x1="500" y1="138" x2="500" y2="176" stroke="rgba(126,184,218,0.35)" stroke-width="1.1" stroke-dasharray="5 4" marker-end="url(#{id}-arrow-dim)" />
  <text x="510" y="160" text-anchor="start" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.5" fill="rgba(255,255,255,0.42)">event</text>

  <!-- ───────────────────────────── THE LEDGER ──────────────────────────── -->
  <g>
    <rect x="58" y="178" width="544" height="50" rx="3" fill="rgba(126,184,218,0.08)" stroke="#7eb8da" stroke-width="1.1" />
    <text x="78" y="200" text-anchor="start" font-family="var(--font-mono)" font-size="11" fill="#e8e4df">kb_events</text>
    <text x="78" y="216" text-anchor="start" font-family="var(--font-serif)" font-style="italic" font-size="9.5" fill="rgba(255,255,255,0.5)">one ledger · emitter + producing anchor</text>
    <text x="582" y="208" text-anchor="end" font-family="var(--font-mono)" font-size="7.5" letter-spacing="1.2" fill="#9ed3af">BUILT</text>
  </g>

  <!-- ────────────── BELOW THE LEDGER: TWO PROJECTION PATHS ──────────────── -->

  <!-- Authoring events → cognition (the maps grow) -->
  <line x1="150" y1="228" x2="150" y2="288" stroke="#7eb8da" stroke-width="1.1" marker-end="url(#{id}-arrow)" />
  <g>
    <rect x="46" y="290" width="208" height="54" rx="3" fill="rgba(126,184,218,0.08)" stroke="#7eb8da" stroke-width="1.1" />
    <text x="150" y="312" text-anchor="middle" font-family="var(--font-serif)" font-size="11" fill="#e8e4df">cognition</text>
    <text x="150" y="330" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.3" fill="rgba(255,255,255,0.48)">cogmaps · subscriptions · relationships</text>
  </g>

  <!-- Admin events → firewalled compliance-audit stream -->
  <line x1="500" y1="228" x2="500" y2="288" stroke="rgba(126,184,218,0.35)" stroke-width="1.1" stroke-dasharray="5 4" marker-end="url(#{id}-arrow-dim)" />
  <g>
    <rect x="406" y="290" width="190" height="54" rx="3" fill="none" stroke="rgba(126,184,218,0.45)" stroke-width="1.1" stroke-dasharray="5 4" />
    <text x="501" y="310" text-anchor="middle" font-family="var(--font-serif)" font-size="10.5" fill="rgba(232,228,223,0.82)">compliance-audit stream</text>
    <text x="501" y="328" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.3" fill="rgba(255,255,255,0.45)">privacy- &amp; auth-bound · kept for compliance</text>
  </g>

  <!-- ── THE FIREWALL: admin events do NOT cross into cognition ──────────── -->
  <g>
    <!-- vertical separation wall between the two channels -->
    <line x1="330" y1="240" x2="330" y2="356" stroke="rgba(126,184,218,0.30)" stroke-width="1" stroke-dasharray="2 5" />
    <text x="330" y="262" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="2" fill="rgba(255,255,255,0.42)">FIREWALL</text>
    <!-- the blocked crossing: a grant is not a concept -->
    <line x1="406" y1="317" x2="262" y2="317" stroke="rgba(126,184,218,0.30)" stroke-width="0.9" stroke-dasharray="4 4" />
    <g transform="translate(330,317)" stroke="rgba(255,255,255,0.42)" stroke-width="1.1">
      <line x1="-5" y1="-5" x2="5" y2="5" />
      <line x1="-5" y1="5" x2="5" y2="-5" />
    </g>
    <text x="330" y="300" text-anchor="middle" font-family="var(--font-serif)" font-style="italic" font-size="8.5" fill="rgba(255,255,255,0.4)">a grant is not a concept</text>
  </g>

  <!-- ──────────────── POSTGRES RESPONSIBILITY BOUNDARY ─────────────────── -->
  <g>
    <line x1="40" y1="382" x2="620" y2="382" stroke="rgba(126,184,218,0.35)" stroke-width="1" stroke-dasharray="6 5" />
    <text x="44" y="375" text-anchor="start" font-family="var(--font-mono)" font-size="7.5" letter-spacing="1.5" fill="rgba(255,255,255,0.42)">POSTGRES RESPONSIBILITY BOUNDARY</text>
    <text x="616" y="375" text-anchor="end" font-family="var(--font-serif)" font-style="italic" font-size="8.5" fill="rgba(255,255,255,0.4)">the ledger stops here</text>
    <!-- a command straight to Postgres falls below the ledger -->
    <line x1="500" y1="396" x2="500" y2="382" stroke="rgba(255,255,255,0.30)" stroke-width="0.9" stroke-dasharray="3 4" />
    <text x="500" y="412" text-anchor="middle" font-family="var(--font-mono)" font-size="7" letter-spacing="0.4" fill="rgba(255,255,255,0.38)">a direct command can bypass the ledger</text>
  </g>
</svg>

<style>
  svg {
    display: block;
    width: 100%;
    height: auto;
  }
</style>
