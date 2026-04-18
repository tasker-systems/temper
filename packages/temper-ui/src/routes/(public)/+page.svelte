<script>
  import Hero from '$lib/components/landing/Hero.svelte';
  import Section from '$lib/components/landing/Section.svelte';
  import CliBlock from '$lib/components/landing/CliBlock.svelte';
  import AgentTranscript from '$lib/components/landing/AgentTranscript.svelte';
  import Footer from '$lib/components/landing/Footer.svelte';
</script>

<Hero />

<!-- The Premise -->
<Section label="The premise">
  <h2>Knowledge work deserves <em>structure</em></h2>
  <p>
    Code is an expression of intent. So are specifications, plans, and decisions.
    But the context behind them — the <strong>why</strong>, the alternatives considered,
    the constraints that shaped the choice — scatters across conversations, documents,
    and memory.
  </p>
  <p>
    Temper gives that context a home. Every goal, task, session, research thread,
    and decision lives as <strong>markdown with frontmatter</strong> in your vault.
    The frontmatter carries the throughline. The content carries the thinking.
  </p>
</Section>

<!-- How It Works -->
<Section label="How it works">
  <h2>Write markdown. Let temper do the <em>rest</em>.</h2>
  <div class="workflow">
    <div class="workflow-step">
      <span class="workflow-cmd">temper init</span>
      <span class="workflow-desc">Create a vault and tell temper how you work — your tools, your conventions, your rhythm.</span>
    </div>
    <div class="workflow-step">
      <span class="workflow-cmd">temper add</span>
      <span class="workflow-desc">Bring in your docs. Temper extracts markdown, adds frontmatter, and makes everything searchable.</span>
    </div>
    <div class="workflow-step">
      <span class="workflow-cmd">temper search</span>
      <span class="workflow-desc">Semantic search across your vault. Find decisions by meaning, not just keywords.</span>
    </div>
    <div class="workflow-step">
      <span class="workflow-cmd">temper sync</span>
      <span class="workflow-desc">Push to the cloud. Pull to another machine. Your vault follows you.</span>
    </div>
  </div>
</Section>

<!-- What Temper Tracks -->
<Section label="What temper tracks">
  <h2>The vocabulary of <em>structured</em> knowledge work</h2>
  <p>
    Every file in your vault has a doc type that temper understands.
    These aren't arbitrary tags — they're the building blocks of how
    work actually progresses.
  </p>
  <div class="concepts">
    <div class="concept">
      <div class="concept-name">Goals</div>
      <p>The outcome you're working toward. Tasks and sessions roll up to goals.</p>
    </div>
    <div class="concept">
      <div class="concept-name">Tasks</div>
      <p>Discrete units of work with mode (plan/build) and effort (small/medium/large).</p>
    </div>
    <div class="concept">
      <div class="concept-name">Sessions</div>
      <p>What happened in a working session — decisions made, context discovered, next steps.</p>
    </div>
    <div class="concept">
      <div class="concept-name">Research</div>
      <p>Investigation and analysis. Design explorations, comparisons, architectural options.</p>
    </div>
    <div class="concept">
      <div class="concept-name">Decisions</div>
      <p>The choice, the alternatives, the constraints. Captured so you never re-litigate.</p>
    </div>
    <div class="concept">
      <div class="concept-name">Concepts</div>
      <p>Domain knowledge. The vocabulary of your project that humans and agents share.</p>
    </div>
  </div>
</Section>

<!-- For Humans and Agents -->
<Section label="For humans and agents">
  <h2>Context that's always <em>ready to hand</em></h2>
  <p>
    Agentic tools like Claude Code and Cursor are powerful — but only when
    they have context. Temper gives agents the same throughline that humans
    carry in their heads: what we're building, why, what we've decided, and
    what's deferred.
  </p>
  <AgentTranscript />
  <p class="after-transcript">
    Subscribe to contexts across projects. Everything arrives as markdown
    in your vault — no special tooling, no vendor lock-in. If it can read
    files, it can use temper.
  </p>
</Section>

<!-- Temper Cloud -->
<Section label="Temper Cloud">
  <h2>Your vault, <em>everywhere</em></h2>
  <p>
    Work on your laptop. Pick up on your desktop. Let a cloud agent
    contribute while you sleep. Temper Cloud syncs your vault across
    machines and team members with semantic search built in.
  </p>
  <div class="cloud-features">
    <div class="cloud-feature"><div class="dot"></div><span>Cross-machine sync with conflict resolution</span></div>
    <div class="cloud-feature"><div class="dot"></div><span>Semantic search powered by pgvector embeddings</span></div>
    <div class="cloud-feature"><div class="dot"></div><span>Team contexts with granular access control</span></div>
    <div class="cloud-feature"><div class="dot"></div><span>Knowledge graph connecting your resources</span></div>
    <div class="cloud-feature"><div class="dot"></div><span>Self-host or use temperkb.io — same protocol, your choice</span></div>
  </div>
</Section>

<Footer />

<style>
  .workflow { display: flex; flex-direction: column; gap: 1.2rem; margin-top: 1.5rem; }
  .workflow-step { display: flex; align-items: flex-start; gap: 1.2rem; }
  .workflow-cmd { font-family: var(--font-mono); font-size: 0.75rem; padding: 0.4rem 0.8rem; border: 1px solid rgba(255, 255, 255, 0.1); color: var(--temper-blue); white-space: nowrap; min-width: 140px; }
  .workflow-desc { font-family: var(--font-serif); font-size: 0.9rem; color: var(--graphite); padding-top: 0.3rem; line-height: 1.7; }
  .concepts { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 1rem; margin-top: 1.5rem; }
  .concept { border: 1px solid rgba(255, 255, 255, 0.06); padding: 1.2rem; transition: border-color 0.2s; }
  .concept:hover { border-color: var(--temper-blue-border-dim); }
  .concept-name { font-family: var(--font-mono); font-size: 0.7rem; color: var(--temper-blue); letter-spacing: 0.1em; text-transform: uppercase; margin-bottom: 0.5rem; }
  .concept :global(p) { font-size: 0.85rem; color: var(--graphite); line-height: 1.6; margin-bottom: 0; }
  .after-transcript { margin-top: 1.5rem; }
  .cloud-features { display: flex; flex-direction: column; gap: 0.8rem; margin-top: 1.5rem; }
  .cloud-feature { display: flex; gap: 1rem; align-items: baseline; }
  .dot { width: 4px; height: 4px; background: var(--temper-blue); border-radius: 50%; flex-shrink: 0; margin-top: 0.5rem; }
  .cloud-feature span { font-family: var(--font-serif); font-size: 0.95rem; color: var(--chalk); }
</style>
