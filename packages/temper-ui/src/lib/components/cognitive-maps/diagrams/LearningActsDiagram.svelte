<script lang="ts">
  /**
   * The HERO for "how a map grows": the steward's learning workflow. An agent
   * watches the event stream off the ledger, loads the map's charter + regulation,
   * makes a relevance call against *this map's* telos, and produces one of five
   * learning-acts — form, modify, decay, fold, scar — then emits provenance back
   * to the ledger. Growth is a loop, not a state machine.
   *
   * The five acts fan out as a flow, not rigid states. modify/reinforcement is
   * marked DERIVED (read from the reference stream, not a stored bump); the others
   * STORE an event. The SCAR branch is the trace of a past mistake carried
   * forward: it is the only one in red, and it is the only act that loops back to
   * write a regulation resource (with a provenance link to the folded block).
   *
   * `id` namespaces the marker/gradient defs so multiple instances don't clash.
   */
  let { id = 'learning-acts' }: { id?: string } = $props();
</script>

<svg
  viewBox="0 0 760 440"
  xmlns="http://www.w3.org/2000/svg"
  role="img"
  aria-label="A steward agent's learning workflow: it watches the event stream off the ledger, loads the map's charter and regulation, makes a relevance call against this map's telos, then produces one of five learning-acts — form, modify, decay, fold, scar — and emits provenance back to the ledger as a loop. The scar branch, drawn in red, folds a block and writes a regulation resource, linking provenance back to what it replaced."
>
  <defs>
    <!-- Flow arrowhead, blue -->
    <marker id="{id}-arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
      <path d="M0,0 L10,5 L0,10 z" fill="#7eb8da" opacity="0.8" />
    </marker>
    <!-- Scar arrowhead, red — the trace of a past mistake carried forward -->
    <marker id="{id}-arrow-scar" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
      <path d="M0,0 L10,5 L0,10 z" fill="#fca5a5" opacity="0.85" />
    </marker>
    <!-- Faint feedback arrowhead for the loop back to the ledger -->
    <marker id="{id}-arrow-dim" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
      <path d="M0,0 L10,5 L0,10 z" fill="rgba(126,184,218,0.45)" />
    </marker>
  </defs>

  <!-- ══ The ledger: the event stream the agent watches ════════════════ -->
  <g>
    <rect x="24" y="150" width="120" height="140" rx="3"
      fill="rgba(255,255,255,0.02)" stroke="var(--rule-2)" stroke-width="1" />
    <text x="84" y="142" text-anchor="middle" font-family="var(--font-mono)" font-size="8.5" letter-spacing="1.5" fill="rgba(255,255,255,0.45)">THE LEDGER</text>
    <!-- inbound events, stacked -->
    <g font-family="var(--font-mono)" font-size="8" fill="rgba(255,255,255,0.5)">
      <rect x="36" y="166" width="96" height="20" rx="2" fill="rgba(126,184,218,0.08)" stroke="var(--temper-blue-border-dim)" stroke-width="0.75" />
      <text x="84" y="179" text-anchor="middle">event · ref</text>
      <rect x="36" y="194" width="96" height="20" rx="2" fill="rgba(126,184,218,0.08)" stroke="var(--temper-blue-border-dim)" stroke-width="0.75" />
      <text x="84" y="207" text-anchor="middle">event · merge</text>
      <rect x="36" y="222" width="96" height="20" rx="2" fill="rgba(126,184,218,0.08)" stroke="var(--temper-blue-border-dim)" stroke-width="0.75" />
      <text x="84" y="235" text-anchor="middle">event · backfire</text>
      <rect x="36" y="250" width="96" height="20" rx="2" fill="rgba(255,255,255,0.03)" stroke="var(--rule)" stroke-width="0.75" />
      <text x="84" y="263" text-anchor="middle" fill="rgba(255,255,255,0.3)">…</text>
    </g>
  </g>

  <!-- watch: ledger → agent -->
  <line x1="144" y1="200" x2="206" y2="200" stroke="#7eb8da" stroke-width="1" opacity="0.7" marker-end="url(#{id}-arrow)" />
  <text x="175" y="192" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="1" fill="rgba(255,255,255,0.4)">watches</text>

  <!-- ══ The steward agent ═════════════════════════════════════════════ -->
  <g>
    <rect x="206" y="156" width="138" height="88" rx="4"
      fill="rgba(126,184,218,0.06)" stroke="var(--temper-blue-border)" stroke-width="1" />
    <text x="275" y="184" text-anchor="middle" font-family="var(--font-serif)" font-size="15" fill="#e8e4df">steward agent</text>
    <text x="275" y="203" text-anchor="middle" font-family="var(--font-mono)" font-size="8" letter-spacing="0.5" fill="var(--temper-blue)">persona · steward</text>
    <line x1="226" y1="214" x2="324" y2="214" stroke="var(--rule)" stroke-width="0.75" />
    <text x="275" y="230" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.5" fill="rgba(255,255,255,0.42)">loads charter + regulation</text>
  </g>

  <!-- charter / regulation it reads (above the agent) -->
  <g font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.5">
    <rect x="206" y="92" width="138" height="46" rx="3" fill="rgba(255,255,255,0.02)" stroke="var(--rule)" stroke-width="0.75" />
    <text x="275" y="84" text-anchor="middle" fill="rgba(255,255,255,0.4)" letter-spacing="1.5">WHAT IT READS</text>
    <text x="216" y="110" fill="var(--temper-blue)">cogmap_charter</text>
    <text x="216" y="123" fill="var(--temper-blue)">cogmap_questions</text>
    <text x="216" y="136" fill="var(--temper-blue)">cogmap_regulation</text>
  </g>
  <line x1="275" y1="156" x2="275" y2="140" stroke="rgba(126,184,218,0.45)" stroke-width="0.75" marker-end="url(#{id}-arrow-dim)" />

  <!-- relevance call -->
  <line x1="344" y1="200" x2="408" y2="200" stroke="#7eb8da" stroke-width="1" opacity="0.7" marker-end="url(#{id}-arrow)" />
  <g>
    <path d="M453,200 L420,178 L387,200 L420,222 z" fill="rgba(126,184,218,0.06)" stroke="var(--temper-blue-border)" stroke-width="1" />
    <text x="420" y="196" text-anchor="middle" font-family="var(--font-serif)" font-size="10" fill="#e8e4df">relevant to</text>
    <text x="420" y="209" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" fill="var(--temper-blue)">this telos?</text>
  </g>

  <!-- relevance → the acts -->
  <line x1="453" y1="200" x2="516" y2="200" stroke="#7eb8da" stroke-width="1" opacity="0.7" marker-end="url(#{id}-arrow)" />

  <!-- ══ The five learning-acts — a fan-out, not rigid states ══════════ -->
  <text x="600" y="40" text-anchor="middle" font-family="var(--font-mono)" font-size="8.5" letter-spacing="1.5" fill="rgba(255,255,255,0.45)">A LEARNING-ACT</text>

  <!-- distribution point on the spine -->
  <circle cx="520" cy="200" r="2.5" fill="#7eb8da" opacity="0.8" />

  <!-- connectors from spine to each act -->
  <g fill="none" stroke="#7eb8da" stroke-width="0.9" opacity="0.55">
    <path d="M520,200 C548,200 540,64 568,64" marker-end="url(#{id}-arrow)" />
    <path d="M520,200 C548,200 540,118 568,118" marker-end="url(#{id}-arrow)" />
    <path d="M520,200 C548,200 540,172 568,172" marker-end="url(#{id}-arrow)" />
    <path d="M520,200 C548,200 540,226 568,226" marker-end="url(#{id}-arrow)" />
  </g>
  <!-- scar connector, red -->
  <path d="M520,200 C548,200 540,300 568,300" fill="none" stroke="#fca5a5" stroke-width="1.1" opacity="0.7" marker-end="url(#{id}-arrow-scar)" />

  <!-- form -->
  <g>
    <rect x="568" y="48" width="150" height="32" rx="3" fill="rgba(255,255,255,0.025)" stroke="var(--rule-2)" stroke-width="0.75" />
    <text x="580" y="62" font-family="var(--font-serif)" font-size="13" fill="#e8e4df">form</text>
    <text x="580" y="74" font-family="var(--font-mono)" font-size="7" fill="rgba(255,255,255,0.42)">new concept or relation · STORE</text>
  </g>
  <!-- modify -->
  <g>
    <rect x="568" y="102" width="150" height="32" rx="3" fill="rgba(255,255,255,0.025)" stroke="var(--rule-2)" stroke-width="0.75" />
    <text x="580" y="116" font-family="var(--font-serif)" font-size="13" fill="#e8e4df">modify</text>
    <text x="580" y="128" font-family="var(--font-mono)" font-size="7" fill="var(--temper-blue)">reinforce · DERIVED, not stored</text>
  </g>
  <!-- decay -->
  <g>
    <rect x="568" y="156" width="150" height="32" rx="3" fill="rgba(255,255,255,0.025)" stroke="var(--rule-2)" stroke-width="0.75" />
    <text x="580" y="170" font-family="var(--font-serif)" font-size="13" fill="#e8e4df">decay</text>
    <text x="580" y="182" font-family="var(--font-mono)" font-size="7" fill="rgba(255,255,255,0.42)">standing falls · RESTRAINT</text>
  </g>
  <!-- fold -->
  <g>
    <rect x="568" y="210" width="150" height="32" rx="3" fill="rgba(255,255,255,0.025)" stroke="var(--rule-2)" stroke-width="0.75" />
    <text x="580" y="224" font-family="var(--font-serif)" font-size="13" fill="#e8e4df">fold</text>
    <text x="580" y="236" font-family="var(--font-mono)" font-size="7" fill="rgba(255,255,255,0.42)">is_folded · preserved, not wrong</text>
  </g>
  <!-- scar — the unmistakable red branch -->
  <g>
    <rect x="568" y="284" width="174" height="60" rx="3" fill="rgba(252,165,165,0.07)" stroke="#fca5a5" stroke-width="1.1" />
    <text x="580" y="300" font-family="var(--font-serif)" font-size="13" fill="#fca5a5">scar</text>
    <text x="580" y="314" font-family="var(--font-mono)" font-size="7" fill="rgba(252,165,165,0.85)">fold the block</text>
    <text x="580" y="326" font-family="var(--font-mono)" font-size="7" fill="rgba(252,165,165,0.85)"><tspan font-weight="600">and</tspan> write regulation</text>
    <text x="580" y="338" font-family="var(--font-mono)" font-size="7" fill="rgba(252,165,165,0.7)">provenance → folded block</text>
  </g>

  <!-- scar feeds regulation: red branch down to the regulation resource -->
  <g>
    <rect x="416" y="350" width="160" height="46" rx="3" fill="rgba(252,165,165,0.05)" stroke="#fca5a5" stroke-width="1" />
    <text x="496" y="370" text-anchor="middle" font-family="var(--font-serif)" font-size="12" fill="#fca5a5">regulation resource</text>
    <text x="496" y="385" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" fill="rgba(252,165,165,0.8)">“pair on the first PR”</text>
  </g>
  <path d="M655,344 C655,374 600,373 580,373"
    fill="none" stroke="#fca5a5" stroke-width="1.1" opacity="0.75" marker-end="url(#{id}-arrow-scar)" />
  <text x="648" y="362" text-anchor="middle" font-family="var(--font-mono)" font-size="7" letter-spacing="0.5" fill="rgba(252,165,165,0.75)">writes forward</text>

  <!-- ══ Emit provenance back to the ledger — the loop ═════════════════ -->
  <text x="640" y="412" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="0.5" fill="rgba(255,255,255,0.4)">every act emits provenance-with-stance</text>
  <!-- feedback path: from the acts region back round the bottom to the ledger -->
  <path d="M416,373 C220,373 84,360 84,290"
    fill="none" stroke="rgba(126,184,218,0.4)" stroke-width="1" stroke-dasharray="4 4" marker-end="url(#{id}-arrow-dim)" />
  <text x="240" y="367" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="1" fill="rgba(126,184,218,0.55)">emit → kb_block_provenance</text>

  <!-- the loop, named -->
  <text x="84" y="312" text-anchor="middle" font-family="var(--font-serif)" font-style="italic" font-size="11" fill="rgba(255,255,255,0.45)">growth is a loop</text>
</svg>

<style>
  svg {
    display: block;
    width: 100%;
    height: auto;
  }
</style>
