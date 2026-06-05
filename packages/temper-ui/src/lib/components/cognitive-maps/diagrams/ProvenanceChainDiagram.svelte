<script lang="ts">
  /**
   * The closing HERO: the correlated reasoning-provenance chain. One causal
   * line runs left-to-right across a system boundary. Outside Temper (left): a
   * remote act — PR #123 merged on GitHub. It crosses the boundary as a webhook
   * event carrying a correlation id. Inside: the triage agent wakes, reads the
   * charter, and emits a mutation event with its reasoning in the payload, which
   * writes a regulation and reinforces a charter question. Every node shares the
   * same correlation thread, drawn as one unbroken connecting spine — you can
   * trace not only WHAT the system believes but HOW it came to believe it.
   *
   * Real-vs-proposed is load-bearing: the CHAIN is real (solid spine, solid
   * nodes); the analytics DASHBOARD reading it is proposed (dashed, faint).
   *
   * Honest basis: kb_events.correlation_id + emitter_entity_id thread the chain;
   * the open metadata jsonb carries the agent's reasoning; cogmap_questions'
   * reinforce_count is the provenance accretion. The cross-system query and the
   * assembled provenance graph are proposed.
   *
   * `id` namespaces the gradient/filter defs so multiple instances don't clash.
   */
  let { id = 'provenance-chain' }: { id?: string } = $props();
</script>

<svg
  viewBox="0 0 660 380"
  xmlns="http://www.w3.org/2000/svg"
  role="img"
  aria-label="A correlated reasoning-provenance chain drawn left-to-right across a system boundary: a PR merged on GitHub crosses the boundary as a webhook event carrying a correlation id, the triage agent wakes and emits a mutation event with its reasoning, which writes a regulation and reinforces a charter question — one unbroken solid chain, with a proposed analytics dashboard drawn faint and dashed reading it"
>
  <defs>
    <!-- Glow under each real chain node -->
    <radialGradient id="{id}-node" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(126,184,218,0.32)" />
      <stop offset="60%" stop-color="rgba(126,184,218,0.10)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0)" />
    </radialGradient>
    <!-- Soft atmospheric wash behind the chain -->
    <radialGradient id="{id}-weather" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(126,184,218,0.06)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0)" />
    </radialGradient>
    <!-- Arrowhead carried along the solid correlation spine -->
    <marker
      id="{id}-tip"
      viewBox="0 0 10 10"
      refX="8"
      refY="5"
      markerWidth="7"
      markerHeight="7"
      orient="auto-start-reverse"
    >
      <path d="M0,0 L9,5 L0,10 z" fill="#7eb8da" />
    </marker>
  </defs>

  <!-- Atmospheric ground wash -->
  <ellipse cx="330" cy="208" rx="360" ry="170" fill="url(#{id}-weather)" />

  <!-- ── The system boundary: outside Temper (left) | inside (right) ───── -->
  <line
    x1="150" y1="44" x2="150" y2="336"
    stroke="var(--rule)" stroke-width="1" stroke-dasharray="2 5"
  />
  <text x="74" y="358" text-anchor="middle" font-family="var(--font-mono)" font-size="8" letter-spacing="1.5" fill="rgba(255,255,255,0.30)">
    GITHUB
  </text>
  <text x="408" y="358" text-anchor="middle" font-family="var(--font-mono)" font-size="8" letter-spacing="1.5" fill="rgba(255,255,255,0.30)">
    TEMPER · the cognitive substrate
  </text>

  <!-- ── The correlation spine: SOLID = real, one unbroken thread ─────── -->
  <!-- thread back-label rides under the spine -->
  <text x="330" y="252" text-anchor="middle" font-family="var(--font-mono)" font-size="8" letter-spacing="1" fill="rgba(126,184,218,0.55)">
    correlation_id — one thread, end to end
  </text>
  <g stroke="#7eb8da" stroke-width="1.5" fill="none">
    <line x1="92" y1="208" x2="138" y2="208" marker-end="url(#{id}-tip)" />
    <line x1="162" y1="208" x2="248" y2="208" marker-end="url(#{id}-tip)" />
    <line x1="304" y1="208" x2="400" y2="208" marker-end="url(#{id}-tip)" />
    <line x1="456" y1="208" x2="544" y2="208" marker-end="url(#{id}-tip)" />
  </g>
  <!-- the spine forks at the end: writes a regulation / reinforces a question -->
  <g stroke="#7eb8da" stroke-width="1.5" fill="none">
    <path d="M600,208 C620,208 624,158 600,150" marker-end="url(#{id}-tip)" />
    <path d="M600,208 C620,208 624,258 600,266" marker-end="url(#{id}-tip)" />
  </g>

  <!-- ── Node 1: PR #123 merged (outside, the remote act) ─────────────── -->
  <g>
    <circle cx="60" cy="208" r="34" fill="url(#{id}-node)" />
    <circle cx="60" cy="208" r="17" fill="none" stroke="#7eb8da" stroke-width="1.25" />
    <text x="60" y="205" text-anchor="middle" font-family="var(--font-serif)" font-size="13" fill="#e8e4df">PR #123</text>
    <text x="60" y="219" text-anchor="middle" font-family="var(--font-serif)" font-size="11" fill="#e8e4df">merged</text>
    <text x="60" y="160" text-anchor="middle" font-family="var(--font-mono)" font-size="8" letter-spacing="1.5" fill="rgba(255,255,255,0.42)">REMOTE ACT</text>
  </g>

  <!-- crossing label on the boundary-spanning arrow -->
  <text x="150" y="192" text-anchor="middle" font-family="var(--font-mono)" font-size="8" letter-spacing="0.5" fill="rgba(126,184,218,0.7)">
    webhook event
  </text>
  <text x="150" y="180" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.5" fill="rgba(255,255,255,0.38)">
    carries correlation id
  </text>

  <!-- ── Node 2: agent wakes (inside) ──────────────────────────────────── -->
  <g>
    <circle cx="276" cy="208" r="34" fill="url(#{id}-node)" />
    <circle cx="276" cy="208" r="17" fill="none" stroke="#7eb8da" stroke-width="1.25" />
    <text x="276" y="205" text-anchor="middle" font-family="var(--font-serif)" font-size="12" fill="#e8e4df">agent</text>
    <text x="276" y="219" text-anchor="middle" font-family="var(--font-serif)" font-size="12" fill="#e8e4df">wakes</text>
    <text x="276" y="160" text-anchor="middle" font-family="var(--font-mono)" font-size="8" letter-spacing="1.5" fill="rgba(255,255,255,0.42)">READS CHARTER</text>
  </g>

  <!-- ── Node 3: mutation event, reasoning in payload (inside) ─────────── -->
  <g>
    <circle cx="428" cy="208" r="36" fill="url(#{id}-node)" />
    <circle cx="428" cy="208" r="18" fill="none" stroke="#7eb8da" stroke-width="1.25" />
    <text x="428" y="205" text-anchor="middle" font-family="var(--font-serif)" font-size="12" fill="#e8e4df">mutation</text>
    <text x="428" y="219" text-anchor="middle" font-family="var(--font-serif)" font-size="12" fill="#e8e4df">event</text>
    <text x="428" y="158" text-anchor="middle" font-family="var(--font-mono)" font-size="8" letter-spacing="0.5" fill="rgba(126,184,218,0.7)">reasoning in payload</text>
    <text x="428" y="146" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.5" fill="rgba(255,255,255,0.38)">metadata jsonb</text>
  </g>

  <!-- ── Node 4a: writes a regulation (top fork) ──────────────────────── -->
  <g>
    <circle cx="592" cy="146" r="28" fill="url(#{id}-node)" />
    <circle cx="592" cy="146" r="14" fill="none" stroke="#7eb8da" stroke-width="1.25" />
    <text x="592" y="143" text-anchor="middle" font-family="var(--font-serif)" font-size="10.5" fill="#e8e4df">writes a</text>
    <text x="592" y="156" text-anchor="middle" font-family="var(--font-serif)" font-size="10.5" fill="#e8e4df">regulation</text>
  </g>

  <!-- ── Node 4b: reinforces a charter question (bottom fork) ──────────── -->
  <g>
    <circle cx="592" cy="270" r="28" fill="url(#{id}-node)" />
    <circle cx="592" cy="270" r="14" fill="none" stroke="#7eb8da" stroke-width="1.25" />
    <text x="592" y="267" text-anchor="middle" font-family="var(--font-serif)" font-size="10.5" fill="#e8e4df">reinforces</text>
    <text x="592" y="280" text-anchor="middle" font-family="var(--font-serif)" font-size="10.5" fill="#e8e4df">a question</text>
    <text x="592" y="306" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.5" fill="rgba(255,255,255,0.38)">reinforce_count ++</text>
  </g>

  <!-- ── PROPOSED: the analytics dashboard reading the chain ──────────── -->
  <!-- dashed, faint — the columns exist; this layer does not yet -->
  <g opacity="0.55">
    <rect
      x="44" y="36" width="572" height="40" rx="4"
      fill="none" stroke="var(--temper-blue-border-dim)" stroke-width="1" stroke-dasharray="4 4"
    />
    <text x="60" y="54" font-family="var(--font-mono)" font-size="8.5" letter-spacing="2" fill="rgba(126,184,218,0.6)">
      ANALYTICS DASHBOARD
    </text>
    <text x="60" y="67" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.5" fill="rgba(255,255,255,0.34)">
      query the provenance: why did our understanding of onboarding shift this week?
    </text>
    <text x="600" y="61" text-anchor="end" font-family="var(--font-mono)" font-size="7.5" letter-spacing="1" fill="rgba(255,255,255,0.30)" font-style="italic">
      proposed
    </text>
  </g>
  <!-- faint dashed "reads" tethers from the dashboard down onto the real chain -->
  <g stroke="var(--temper-blue-border-dim)" stroke-width="0.75" stroke-dasharray="3 4" opacity="0.45" fill="none">
    <line x1="60" y1="76" x2="60" y2="172" />
    <line x1="276" y1="76" x2="276" y2="172" />
    <line x1="428" y1="76" x2="428" y2="170" />
    <line x1="592" y1="76" x2="592" y2="116" />
  </g>
</svg>

<style>
  svg {
    display: block;
    width: 100%;
    height: auto;
  }
</style>
