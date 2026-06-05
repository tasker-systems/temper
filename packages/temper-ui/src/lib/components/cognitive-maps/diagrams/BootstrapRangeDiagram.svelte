<script lang="ts">
  /**
   * The deployment HERO: the 0→1→N bootstrap path drawn as the invariant SEED
   * (solid), wrapped by the org-shaped DEPLOYMENT-SHAPE RANGE (a faint dashed
   * band that widens as the path moves from one map to many).
   *
   * The invariant spine reads left→right: 0 (empty Postgres) → schema loaded →
   * Temper serving CLI / API / MCP → the first seed (temper-system +
   * system-default, "this is the seed file") → 1→N (a new map by authoring, a
   * webhook stream arriving, an agent waking). That spine is solid because it's
   * the same on every deployment. Around the 1→N stretch the range opens
   * between a near-minimal floor (serverless / Neon / single-tenant / platform
   * agents — temperkb.io's point, marked) and a fuller ceiling (cluster /
   * operated DB / multi-tenant + per-tenant webhooks / dedicated agent infra).
   *
   * Honest basis: the 0→1 floor is real (temper-system + system-default seed,
   * cogmap_genesis for authoring, kb_entities + emitter_entity_id NOT NULL for
   * integrations-as-entities, kb_topics for topic bounds). The topology /
   * tenancy / per-tenant-subscription / agent-platform choices are operational,
   * not in the artifact — drawn as a range; the contract and trigger mechanisms
   * are designed, not yet built — drawn as proposed (dashed/faint).
   *
   * `id` namespaces the gradient/filter defs so multiple instances don't clash.
   */
  let { id = 'bootstrap-range' }: { id?: string } = $props();
</script>

<svg
  viewBox="0 0 660 380"
  xmlns="http://www.w3.org/2000/svg"
  role="img"
  aria-label="The deployment bootstrap path drawn left to right — empty Postgres, schema loaded, Temper serving CLI / API / MCP, the first seed marked 'this is the seed file', then 1 to many — with the invariant spine solid and the org-shaped deployment range drawn around the one-to-many stretch as a faint dashed band between a near-minimal floor tagged temperkb.io and a fuller ceiling"
>
  <defs>
    <!-- The range band fill — faint, proposed, org-shaped -->
    <linearGradient id="{id}-range" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0%" stop-color="rgba(126,184,218,0)" />
      <stop offset="35%" stop-color="rgba(126,184,218,0.04)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0.10)" />
    </linearGradient>
    <!-- Seed node glow — the invariant pivot -->
    <radialGradient id="{id}-seed" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="rgba(126,184,218,0.32)" />
      <stop offset="60%" stop-color="rgba(126,184,218,0.10)" />
      <stop offset="100%" stop-color="rgba(126,184,218,0)" />
    </radialGradient>
  </defs>

  <!-- ── The deployment-shape RANGE: a faint dashed band widening over 1→N ── -->
  <!-- Drawn first so the solid invariant spine reads on top of it. -->
  <g>
    <!-- Band fill between the near-minimal floor and the fuller ceiling -->
    <path
      d="M 360 200 L 612 96 L 612 304 Z"
      fill="url(#{id}-range)"
    />
    <!-- Ceiling edge: the fuller shape (proposed → dashed) -->
    <path
      d="M 360 200 L 612 96"
      fill="none"
      stroke="rgba(126,184,218,0.40)"
      stroke-width="1"
      stroke-dasharray="5 4"
      opacity="0.7"
    />
    <!-- Floor edge: the near-minimal shape (proposed → dashed) -->
    <path
      d="M 360 200 L 612 304"
      fill="none"
      stroke="rgba(126,184,218,0.40)"
      stroke-width="1"
      stroke-dasharray="5 4"
      opacity="0.7"
    />

    <!-- Range caption -->
    <text
      x="500" y="64"
      text-anchor="middle"
      font-family="var(--font-mono)"
      font-size="8.5"
      letter-spacing="1.5"
      fill="rgba(255,255,255,0.45)"
    >DEPLOYMENT-SHAPE RANGE · YOURS</text>

    <!-- Ceiling label: the fuller shape -->
    <g font-family="var(--font-mono)" font-size="8" fill="rgba(255,255,255,0.45)">
      <text x="618" y="92" text-anchor="end">cluster · operated DB</text>
      <text x="618" y="105" text-anchor="end">multi-tenant + per-tenant webhooks</text>
      <text x="618" y="118" text-anchor="end">dedicated agent infra</text>
    </g>

    <!-- Floor label: the near-minimal shape, with temperkb.io marked -->
    <g font-family="var(--font-mono)" font-size="8" fill="rgba(255,255,255,0.45)">
      <text x="618" y="296" text-anchor="end">serverless · Neon · single-tenant</text>
      <text x="618" y="309" text-anchor="end">platform agents</text>
    </g>
    <!-- temperkb.io: one labelled point near the minimum (floor) -->
    <g>
      <circle cx="612" cy="304" r="4" fill="#7eb8da" />
      <circle cx="612" cy="304" r="4" fill="none" stroke="#7eb8da" stroke-width="1" opacity="0.4" />
      <text
        x="618" y="326"
        text-anchor="end"
        font-family="var(--font-mono)"
        font-size="8.5"
        letter-spacing="0.5"
        fill="#7eb8da"
      >▸ temperkb.io · one point near the minimum</text>
    </g>
  </g>

  <!-- ── The INVARIANT spine: 0 → 1 → N, drawn SOLID ──────────────────────── -->
  <!-- Baseline thread -->
  <line x1="48" y1="200" x2="360" y2="200" stroke="#7eb8da" stroke-width="1.5" opacity="0.65" />

  <!-- 0: empty Postgres -->
  <g>
    <circle cx="48" cy="200" r="4" fill="none" stroke="#e8e4df" stroke-width="1.25" opacity="0.55" />
    <text x="48" y="178" text-anchor="middle" font-family="var(--font-mono)" font-size="11" letter-spacing="1" fill="rgba(255,255,255,0.45)">0</text>
    <text x="48" y="224" text-anchor="middle" font-family="var(--font-mono)" font-size="8" fill="rgba(255,255,255,0.45)">empty</text>
    <text x="48" y="235" text-anchor="middle" font-family="var(--font-mono)" font-size="8" fill="rgba(255,255,255,0.45)">Postgres</text>
  </g>

  <!-- schema loaded -->
  <g>
    <circle cx="152" cy="200" r="4" fill="#e8e4df" opacity="0.75" />
    <text x="152" y="224" text-anchor="middle" font-family="var(--font-mono)" font-size="8" fill="rgba(255,255,255,0.55)">schema</text>
    <text x="152" y="235" text-anchor="middle" font-family="var(--font-mono)" font-size="8" fill="rgba(255,255,255,0.55)">loaded</text>
  </g>

  <!-- Temper serving its surfaces -->
  <g>
    <circle cx="256" cy="200" r="4" fill="#e8e4df" opacity="0.75" />
    <text x="256" y="224" text-anchor="middle" font-family="var(--font-mono)" font-size="8" fill="rgba(255,255,255,0.55)">Temper serving</text>
    <text x="256" y="235" text-anchor="middle" font-family="var(--font-mono)" font-size="8" fill="rgba(255,255,255,0.55)">CLI / API / MCP</text>
  </g>

  <!-- 1: the seed — temper-system + system-default, the invariant pivot -->
  <g>
    <circle cx="360" cy="200" r="34" fill="url(#{id}-seed)" />
    <circle cx="360" cy="200" r="7" fill="#7eb8da" />
    <text x="360" y="160" text-anchor="middle" font-family="var(--font-mono)" font-size="11" letter-spacing="1" fill="#7eb8da">1</text>
    <text x="360" y="245" text-anchor="middle" font-family="var(--font-serif)" font-size="13" fill="#e8e4df">“this is the seed file”</text>
    <text x="360" y="262" text-anchor="middle" font-family="var(--font-mono)" font-size="8" fill="rgba(255,255,255,0.55)">temper-system · system-default</text>
    <text x="360" y="180" text-anchor="middle" font-family="var(--font-mono)" font-size="7.5" letter-spacing="1.5" fill="rgba(255,255,255,0.4)">INVARIANT · SOLID</text>
  </g>

  <!-- N: the three motions of 1→many, riding the solid spine into the range -->
  <line x1="367" y1="200" x2="430" y2="200" stroke="#7eb8da" stroke-width="1.5" opacity="0.5" />
  <polygon points="430,196 440,200 430,204" fill="#7eb8da" opacity="0.65" />
  <g font-family="var(--font-mono)" font-size="8.5" fill="rgba(255,255,255,0.6)">
    <text x="452" y="178">N · a new map by authoring</text>
    <text x="452" y="194">N · a webhook stream arriving</text>
    <text x="452" y="210">N · an agent waking</text>
  </g>
  <text x="452" y="232" font-family="var(--font-mono)" font-size="7.5" letter-spacing="1" fill="rgba(255,255,255,0.4)">…the shape opens here</text>
</svg>

<style>
  svg {
    display: block;
    width: 100%;
    height: auto;
  }
</style>
