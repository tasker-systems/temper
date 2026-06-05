<script lang="ts">
  /**
   * The audits and their homes — two kinds of audit, kept deliberately apart in
   * two different homes. INSIDE the system boundary (solid, built): the event
   * ledger as the epistemic trail of *how understanding formed* (assertions,
   * folds, reinforcements, scars), and — on the same ledger but firewalled into
   * a separate compliance channel that never feeds cognition — the governance
   * trail of *who was granted what, when*. OUTSIDE (dashed, proposed): operational
   * audit — a running Temper emitting traces and metrics into external tooling
   * (OpenTelemetry / Prometheus), with the metric scope an organization-shaped
   * dial. A dashed line beneath everything marks the Postgres responsibility
   * boundary, below which direct database commands fall outside the ledger.
   *
   * Honest basis: kb_events (emitter_entity_id, correlation_id, producing anchor)
   * + the fold/provenance trail (kb_block_provenance, is_folded). Operational is
   * external tooling, not in the artifact — drawn outside / proposed.
   *
   * `id` namespaces the def so multiple instances don't clash.
   */
  let { id = 'audit-homes' }: { id?: string } = $props();
</script>

<svg
  viewBox="0 0 660 400"
  xmlns="http://www.w3.org/2000/svg"
  role="img"
  aria-label="Two kinds of audit in two homes: inside the solid system boundary, the event ledger carries the epistemic trail of how understanding formed and a firewalled governance trail of who was granted what; outside, drawn dashed and proposed, operational audit emits traces and metrics into external OpenTelemetry and Prometheus tooling; a dashed Postgres responsibility boundary runs beneath everything"
>
  <defs>
    <!-- The ledger's interior glow — the built, inside thing -->
    <radialGradient id="{id}-ledger" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(126,184,218,0.14)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0)" />
    </radialGradient>
  </defs>

  <!-- ── INSIDE: the system boundary (solid = built) ─────────────────── -->
  <rect
    x="40" y="40" width="400" height="266"
    rx="6"
    fill="url(#{id}-ledger)"
    stroke="var(--temper-blue-border)"
    stroke-width="1.25"
  />
  <text x="56" y="66" font-family="var(--font-mono)" font-size="8.5" letter-spacing="2" fill="rgba(126,184,218,0.8)">
    INSIDE · THE EVENT LEDGER
  </text>
  <text x="56" y="80" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.5" fill="rgba(255,255,255,0.32)">
    built · the audit you get for free
  </text>

  <!-- Epistemic stream: how understanding formed -->
  <g>
    <rect x="56" y="98" width="368" height="84" rx="4" fill="rgba(126,184,218,0.05)" stroke="var(--rule)" stroke-width="0.75" />
    <text x="72" y="120" font-family="var(--font-serif)" font-size="15" fill="#e8e4df">Epistemic</text>
    <text x="72" y="136" font-family="var(--font-mono)" font-size="8" letter-spacing="0.5" fill="rgba(255,255,255,0.45)">
      how understanding formed
    </text>
    <!-- the trail of acts, each a solid event on the thread -->
    <g font-family="var(--font-mono)" font-size="8.5">
      <g transform="translate(72,158)">
        <circle cx="0" cy="-3" r="3" fill="#7eb8da" />
        <text x="10" y="0" fill="rgba(255,255,255,0.62)">assert</text>
      </g>
      <g transform="translate(150,158)">
        <circle cx="0" cy="-3" r="3" fill="#7eb8da" />
        <text x="10" y="0" fill="rgba(255,255,255,0.62)">fold</text>
      </g>
      <g transform="translate(216,158)">
        <circle cx="0" cy="-3" r="3" fill="#7eb8da" />
        <text x="10" y="0" fill="rgba(255,255,255,0.62)">reinforce</text>
      </g>
      <g transform="translate(312,158)">
        <circle cx="0" cy="-3" r="3" fill="#7eb8da" />
        <text x="10" y="0" fill="rgba(255,255,255,0.62)">scar</text>
      </g>
    </g>
    <!-- correlation thread linking the acts -->
    <line x1="72" y1="155" x2="348" y2="155" stroke="rgba(126,184,218,0.4)" stroke-width="0.75" />
    <text x="424" y="120" text-anchor="end" font-family="var(--font-mono)" font-size="7.5" fill="rgba(255,255,255,0.3)">
      feeds the maps
    </text>
  </g>

  <!-- Governance stream: same ledger, firewalled, never feeds cognition -->
  <g>
    <rect x="56" y="196" width="368" height="92" rx="4" fill="rgba(126,184,218,0.05)" stroke="var(--rule)" stroke-width="0.75" />
    <text x="72" y="218" font-family="var(--font-serif)" font-size="15" fill="#e8e4df">Governance</text>
    <text x="72" y="234" font-family="var(--font-mono)" font-size="8" letter-spacing="0.5" fill="rgba(255,255,255,0.45)">
      who was granted what, when
    </text>
    <g font-family="var(--font-mono)" font-size="8.5">
      <g transform="translate(72,258)">
        <circle cx="0" cy="-3" r="3" fill="#7eb8da" />
        <text x="10" y="0" fill="rgba(255,255,255,0.62)">grant</text>
      </g>
      <g transform="translate(150,258)">
        <circle cx="0" cy="-3" r="3" fill="#7eb8da" />
        <text x="10" y="0" fill="rgba(255,255,255,0.62)">revoke</text>
      </g>
    </g>
    <line x1="72" y1="255" x2="200" y2="255" stroke="rgba(126,184,218,0.4)" stroke-width="0.75" />
    <text x="424" y="218" text-anchor="end" font-family="var(--font-mono)" font-size="7.5" fill="rgba(255,255,255,0.3)">
      compliance · firewalled
    </text>
    <!-- the firewall: a separation that does NOT feed cognition -->
    <line x1="318" y1="246" x2="408" y2="246" stroke="var(--temper-blue-border-dim)" stroke-width="0.75" stroke-dasharray="2 3" />
    <text x="408" y="244" text-anchor="end" font-family="var(--font-mono)" font-size="7" fill="rgba(255,255,255,0.28)">
      does not feed the maps
    </text>
  </g>

  <!-- ── OUTSIDE: operational audit (dashed = proposed, external) ─────── -->
  <rect
    x="470" y="40" width="156" height="266"
    rx="6"
    fill="none"
    stroke="var(--temper-blue-border-dim)"
    stroke-width="1"
    stroke-dasharray="5 4"
  />
  <text x="486" y="66" font-family="var(--font-mono)" font-size="8.5" letter-spacing="2" fill="rgba(255,255,255,0.42)">
    OUTSIDE
  </text>
  <text x="486" y="80" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.5" fill="rgba(255,255,255,0.28)">
    proposed · external tooling
  </text>

  <text x="486" y="110" font-family="var(--font-serif)" font-size="15" fill="rgba(232,228,223,0.7)">Operational</text>
  <text x="486" y="126" font-family="var(--font-mono)" font-size="8" letter-spacing="0.5" fill="rgba(255,255,255,0.4)">
    is the system healthy
  </text>

  <g font-family="var(--font-mono)" font-size="8.5" fill="rgba(255,255,255,0.5)">
    <text x="486" y="152">calls</text>
    <text x="486" y="168">latency</text>
    <text x="486" y="184">errors</text>
  </g>

  <!-- external sinks, dashed -->
  <g>
    <rect x="486" y="200" width="124" height="26" rx="3" fill="none" stroke="var(--temper-blue-border-dim)" stroke-width="0.75" stroke-dasharray="3 3" />
    <text x="548" y="217" text-anchor="middle" font-family="var(--font-mono)" font-size="8.5" fill="rgba(255,255,255,0.5)">OpenTelemetry</text>
    <rect x="486" y="234" width="124" height="26" rx="3" fill="none" stroke="var(--temper-blue-border-dim)" stroke-width="0.75" stroke-dasharray="3 3" />
    <text x="548" y="251" text-anchor="middle" font-family="var(--font-mono)" font-size="8.5" fill="rgba(255,255,255,0.5)">Prometheus</text>
  </g>
  <text x="486" y="282" font-family="var(--font-mono)" font-size="7" fill="rgba(255,255,255,0.3)">which metrics:</text>
  <text x="486" y="294" font-family="var(--font-mono)" font-size="7" fill="rgba(255,255,255,0.3)">an org-scoped dial</text>

  <!-- emit arrow: running system → external tooling (dashed = proposed) -->
  <line x1="440" y1="200" x2="468" y2="200" stroke="var(--temper-blue-border-dim)" stroke-width="1" stroke-dasharray="4 3" />
  <polygon points="468,196 476,200 468,204" fill="var(--temper-blue-border-dim)" />
  <text x="454" y="192" text-anchor="middle" font-family="var(--font-mono)" font-size="7" fill="rgba(255,255,255,0.32)">emit</text>

  <!-- ── Postgres responsibility boundary (dashed, beneath everything) ── -->
  <line x1="40" y1="344" x2="620" y2="344" stroke="var(--rule)" stroke-width="1" stroke-dasharray="6 5" />
  <text x="40" y="336" font-family="var(--font-mono)" font-size="8" letter-spacing="1.5" fill="rgba(255,255,255,0.42)">
    POSTGRES RESPONSIBILITY BOUNDARY
  </text>
  <text x="40" y="368" font-family="var(--font-mono)" font-size="8" fill="rgba(255,255,255,0.3)">
    below: direct database commands — outside the ledger entirely
  </text>
</svg>

<style>
  svg {
    display: block;
    width: 100%;
    height: auto;
  }
</style>
