<script>
  import Section from '$lib/components/landing/Section.svelte';
  import CliBlock from '$lib/components/landing/CliBlock.svelte';
  import Footer from '$lib/components/landing/Footer.svelte';
</script>

<svelte:head>
  <title>temper for builders — remember what you decided</title>
  <meta name="description" content="Session-over-session context for solo developers and small teams. Your vault holds the throughline — what's been done, what's up next, what the agent needs to know." />
</svelte:head>

<!-- Hero -->
<section class="hero">
  <div class="hero-label">For builders</div>
  <h1>Remember what you <em>decided</em></h1>
  <p class="tagline">
    Every session builds on the last. Your vault holds the throughline —
    what's been done, what's up next, what the agent needs to know.
  </p>
  <div class="hero-ctas">
    <a href="/docs" class="primary">Get Started</a>
    <a href="https://github.com/tasker-systems/temper" class="secondary">View on GitHub</a>
  </div>
  <div class="cli-wrapper">
    <CliBlock>
      <div class="cli-prompt">
        <span class="flag">$</span> <span class="cmd">temper warmup</span> <span class="flag">--context</span> myapp
      </div>
      <div class="cli-divider"></div>
      <div class="warmup-output">
        <div class="warmup-line">
          <span class="warmup-label">Last session</span>
          <span class="warmup-value">Mar 28 — Chose JWT rotation over session tokens</span>
        </div>
        <div class="warmup-line">
          <span class="warmup-label">In-progress</span>
          <span class="warmup-value"><span class="tag tag-mode">build</span> <span class="tag tag-effort">medium</span> api-auth-middleware</span>
        </div>
        <div class="warmup-line">
          <span class="warmup-label">Deferred</span>
          <span class="warmup-value"><span class="dim">rate limiting — blocked on load testing</span></span>
        </div>
        <div class="warmup-line">
          <span class="warmup-label">Open decisions</span>
          <span class="warmup-value"><span class="dim">2 — token expiry window, refresh strategy</span></span>
        </div>
      </div>
      <div class="cli-divider"></div>
      <div class="warmup-ready">3 sessions of context loaded. Ready.</div>
    </CliBlock>
  </div>
</section>

<Section label="The problem">
  <h2>The <em>blank</em> session</h2>
  <p>You're three sessions into a feature. On Tuesday you chose JWT rotation over session tokens. Wednesday you explored two caching approaches and picked the one with simpler invalidation. Now it's Friday, and the agent has no idea any of that happened.</p>
  <p>So you re-explain. You paste fragments of old conversations. You write ad-hoc notes that may or may not be found again. The industry has started calling this <strong>context rot</strong> — the progressive degradation of an agent's understanding as work spans sessions. The more complex the project, the heavier the tax.</p>
  <div class="rot-comparison">
    <div class="rot-side rot-without">
      <div class="rot-heading">Without a throughline</div>
      <div class="rot-sessions">
        <div class="rot-session"><span class="rot-name">Session 1</span><span class="rot-bar rot-bar-full"></span></div>
        <div class="rot-loss">context lost</div>
        <div class="rot-session"><span class="rot-name">Session 2</span><span class="rot-bar rot-bar-half"></span></div>
        <div class="rot-loss">context lost</div>
        <div class="rot-session"><span class="rot-name">Session 3</span><span class="rot-bar rot-bar-low"></span></div>
      </div>
      <div class="rot-summary">Understanding degrades. Decisions repeat.</div>
    </div>
    <div class="rot-side rot-with">
      <div class="rot-heading">With temper</div>
      <div class="rot-sessions">
        <div class="rot-session"><span class="rot-name">Session 1</span><span class="rot-bar rot-bar-grow-1"></span></div>
        <div class="rot-gain">saved → warmup</div>
        <div class="rot-session"><span class="rot-name">Session 2</span><span class="rot-bar rot-bar-grow-2"></span></div>
        <div class="rot-gain">saved → warmup</div>
        <div class="rot-session"><span class="rot-name">Session 3</span><span class="rot-bar rot-bar-grow-3"></span></div>
      </div>
      <div class="rot-summary">Context compounds. The throughline grows.</div>
    </div>
  </div>
</Section>

<Section label="The throughline">
  <h2>Context that <em>compounds</em></h2>
  <p>Temper is a local-first knowledge base that embeds <strong>throughline</strong> directly into how you work. Not a task tracker competing with Linear. Not a doc tool competing with Notion. A structured vault of markdown files where every goal, task, session, decision, and research thread has a home — and where the connections between them are always visible.</p>
  <p>The vault grows richer with each session. Decisions persist. Research accumulates. The agent that starts your next session reads what you decided, what you deferred, and what comes next — not because you re-explained it, but because the vault holds the thread.</p>
</Section>

<Section label="Session continuity">
  <h2>Warm up. Work. <em>Save.</em></h2>
  <p>Every session follows a rhythm. Start with context. Do the work. Record what happened. The next session reads what you wrote. This is the core loop — and it's what turns a collection of documents into a living history of what you're building.</p>
  <div class="cycle">
    <div class="cycle-step"><div class="cycle-num">1</div><div class="cycle-content"><span class="cycle-cmd">temper warmup</span><span class="cycle-desc">Injects active tasks, recent sessions, and the last session's full content. The agent resumes where you left off.</span></div></div>
    <div class="cycle-arrow">↓</div>
    <div class="cycle-step"><div class="cycle-num">2</div><div class="cycle-content"><span class="cycle-cmd">Work</span><span class="cycle-desc">Build or plan within a task. The mode and effort shape how the session unfolds — temper doesn't prescribe process, it carries context.</span></div></div>
    <div class="cycle-arrow">↓</div>
    <div class="cycle-step"><div class="cycle-num">3</div><div class="cycle-content"><span class="cycle-cmd">temper session save</span><span class="cycle-desc">Captures what happened, what changed, what decisions were made, and what the next session should pick up.</span></div></div>
    <div class="cycle-return"><span class="cycle-return-label">feeds back into warmup →</span></div>
  </div>
</Section>

<Section label="Semantic search">
  <h2>Find decisions by <em>meaning</em></h2>
  <p>You don't remember the filename. You barely remember the date. But you know you made a decision about authentication strategy somewhere in the last two weeks. Temper embeds your vault using local ML models and searches by meaning, not keywords.</p>
  <div class="search-demo">
    <CliBlock>
      <div class="cli-prompt"><span class="flag">$</span> <span class="cmd">temper search</span> "how did we handle token expiry" <span class="flag">--limit 5</span></div>
      <div class="cli-results">
        <div class="cli-result"><span>decision/jwt-rotation-strategy.md</span><span class="cli-score">0.94</span></div>
        <div class="cli-result"><span>session/2026-03-28-auth-middleware.md</span><span class="cli-score">0.89</span></div>
        <div class="cli-result"><span>research/token-refresh-patterns.md</span><span class="cli-score">0.85</span></div>
        <div class="cli-result"><span>session/2026-03-26-auth-planning.md</span><span class="cli-score">0.79</span></div>
        <div class="cli-result"><span>task/api-auth-middleware.md</span><span class="cli-score">0.72</span></div>
      </div>
    </CliBlock>
    <p class="search-note">Results span decisions, sessions, research, and tasks — all ranked by relevance to what you actually meant, not what you happened to type.</p>
  </div>
</Section>

<Section label="Goals and tasks">
  <h2>Your workflow, <em>your way</em></h2>
  <p>Temper doesn't impose a process. It gives you two building blocks — goals and tasks — and lets you define what they mean for how you work.</p>
  <div class="building-blocks">
    <div class="block-card">
      <div class="block-name">Goals</div>
      <p>The outcome you're building toward. A goal holds the vision and purpose of a feature, a product, a body of work. Tasks and sessions roll up to goals — the throughline runs through them.</p>
    </div>
    <div class="block-card">
      <div class="block-name">Tasks</div>
      <p>A unit of work toward a goal. Every task has a mode — <span class="tag-inline tag-mode">build</span> or <span class="tag-inline tag-plan">plan</span> — and expected effort — <span class="tag-inline tag-effort">small</span>, <span class="tag-inline tag-effort">medium</span>, or <span class="tag-inline tag-effort">large</span>. Code is the common case, but a research proposal, a slide deck, even a novel — the thing you're building is up to you.</p>
    </div>
  </div>
  <p>When you run <span class="cmd-inline">temper init</span>, the setup asks how you work. Do you use superpowers? GSD? Something of your own? Temper adapts to your workflow rather than replacing it — it carries the throughline regardless of what tools and ceremonies you prefer.</p>
</Section>

<Section label="The vault">
  <h2>Everything resolves to <em>markdown</em></h2>
  <p>Your vault is a directory of markdown files with YAML frontmatter. Human-readable in any editor. Version-controllable in git. Natively understood by every language model. No proprietary formats, no required tooling to view your own knowledge.</p>
  <div class="vault-demo">
    <CliBlock>
      <div class="vault-file">
        <div class="vault-frontmatter">
          <div class="vault-fm-line"><span class="vault-key">type:</span> decision</div>
          <div class="vault-fm-line"><span class="vault-key">title:</span> JWT rotation over session tokens</div>
          <div class="vault-fm-line"><span class="vault-key">date:</span> 2026-03-28</div>
          <div class="vault-fm-line"><span class="vault-key">status:</span> accepted</div>
          <div class="vault-fm-line"><span class="vault-key">relates:</span> [api-auth-middleware, oauth-research]</div>
        </div>
        <div class="vault-body">
          <div class="vault-body-line"><span class="vault-h2">## Context</span></div>
          <div class="vault-body-line">The API needs stateless auth for horizontal scaling.</div>
          <div class="vault-body-line">Session tokens require shared state across instances.</div>
          <div class="vault-body-line"></div>
          <div class="vault-body-line"><span class="vault-h2">## Decision</span></div>
          <div class="vault-body-line">JWT with short-lived access tokens (15m) and</div>
          <div class="vault-body-line">rotating refresh tokens. Trade-off: more complexity</div>
          <div class="vault-body-line">in token handling, but no shared session store.</div>
        </div>
      </div>
    </CliBlock>
    <p class="vault-note">The frontmatter carries the throughline — type, status, relationships. The content carries the thinking. Git carries the history.</p>
  </div>
</Section>

<Section label="Get started">
  <h2>Five minutes to your first <em>session</em></h2>
  <CliBlock>
    <div class="cli-prompt"><span class="flag">#</span> <span class="dim">Install temper</span></div>
    <div class="cli-prompt"><span class="flag">$</span> <span class="cmd">cargo install</span> temper-cli</div>
    <div class="cli-spacer"></div>
    <div class="cli-prompt"><span class="flag">#</span> <span class="dim">Initialize a vault — temper asks how you work</span></div>
    <div class="cli-prompt"><span class="flag">$</span> <span class="cmd">temper init</span></div>
    <div class="cli-spacer"></div>
    <div class="cli-prompt"><span class="flag">#</span> <span class="dim">Add a context for your project</span></div>
    <div class="cli-prompt"><span class="flag">$</span> <span class="cmd">temper context</span> add myapp</div>
    <div class="cli-spacer"></div>
    <div class="cli-prompt"><span class="flag">#</span> <span class="dim">Import your docs — temper extracts markdown and indexes it</span></div>
    <div class="cli-prompt"><span class="flag">$</span> <span class="cmd">temper add</span> <span class="flag">--context</span> myapp <span class="flag">--dir</span> ~/projects/myapp/docs</div>
    <div class="cli-spacer"></div>
    <div class="cli-prompt"><span class="flag">#</span> <span class="dim">Generate the Claude Code skill — teaches the agent your vault</span></div>
    <div class="cli-prompt"><span class="flag">$</span> <span class="cmd">temper skill install</span></div>
    <div class="cli-spacer"></div>
    <div class="cli-prompt"><span class="flag">#</span> <span class="dim">Start working. Save when you're done.</span></div>
    <div class="cli-prompt"><span class="flag">$</span> <span class="cmd">temper session save</span> "Set up auth middleware, chose JWT rotation"</div>
  </CliBlock>
</Section>

<div class="cross-sell">
  <p>Temper also works as a <a href="/agents">direct context layer for agents</a> — through the CLI, MCP server, and generated skill files. If your agent can read files, it can use temper.</p>
</div>

<Footer />

<style>
  .hero { min-height: 100vh; display: flex; flex-direction: column; justify-content: center; align-items: center; text-align: center; padding: 6rem 2.5rem 4rem; }
  .hero-label { font-family: var(--mono); font-size: 0.65rem; letter-spacing: 0.2em; text-transform: uppercase; color: var(--blue); margin-bottom: 1.5rem; }
  .hero h1 { font-family: var(--serif); font-size: clamp(2.4rem, 5vw, 3.8rem); font-weight: 300; line-height: 1.2; margin-bottom: 1.5rem; letter-spacing: 0.02em; color: var(--text); }
  .hero h1 em { color: var(--blue); font-style: italic; }
  .tagline { font-family: var(--serif); font-size: 1.1rem; color: var(--text-dim); font-style: italic; max-width: 36em; margin-bottom: 3rem; line-height: 1.7; }
  .hero-ctas { display: flex; gap: 1rem; margin-bottom: 4rem; }
  .hero-ctas a { font-family: var(--mono); font-size: 0.8rem; padding: 0.6rem 1.5rem; text-decoration: none; letter-spacing: 0.05em; transition: all 0.2s; }
  .hero-ctas .primary { border: 1px solid var(--blue-border); color: var(--blue); }
  .hero-ctas .primary:hover { background: rgba(126, 184, 218, 0.1); }
  .hero-ctas .secondary { border: 1px solid rgba(255, 255, 255, 0.12); color: var(--text-dim); }
  .hero-ctas .secondary:hover { border-color: rgba(255, 255, 255, 0.25); color: var(--text-mid); }
  .cli-wrapper { width: 100%; max-width: 620px; }
  .warmup-output { display: flex; flex-direction: column; gap: 0.5rem; }
  .warmup-line { display: flex; gap: 1rem; font-size: 0.7rem; line-height: 1.6; }
  .warmup-label { color: var(--blue-dim); min-width: 110px; flex-shrink: 0; }
  .warmup-value { color: rgba(255, 255, 255, 0.55); }
  .warmup-ready { font-size: 0.7rem; color: var(--blue); }
  .cli-divider { height: 1px; background: rgba(255, 255, 255, 0.04); margin: 0.6rem 0; }
  .tag { font-size: 0.6rem; padding: 0.1rem 0.4rem; border: 1px solid; letter-spacing: 0.05em; }
  .tag-mode { border-color: rgba(126, 184, 218, 0.3); color: var(--blue); }
  .tag-effort { border-color: rgba(255, 255, 255, 0.12); color: var(--text-dim); }
  .dim { color: rgba(255, 255, 255, 0.3); }
  .rot-comparison { display: grid; grid-template-columns: 1fr 1fr; gap: 1rem; margin-top: 1.5rem; }
  .rot-side { border: 1px solid rgba(255, 255, 255, 0.06); padding: 1.2rem; }
  .rot-heading { font-family: var(--mono); font-size: 0.65rem; letter-spacing: 0.1em; text-transform: uppercase; margin-bottom: 1rem; }
  .rot-without .rot-heading { color: rgba(252, 165, 165, 0.7); }
  .rot-with .rot-heading { color: rgba(134, 239, 172, 0.7); }
  .rot-sessions { display: flex; flex-direction: column; gap: 0.3rem; }
  .rot-session { display: flex; align-items: center; gap: 0.8rem; }
  .rot-name { font-family: var(--mono); font-size: 0.6rem; color: var(--text-dim); min-width: 64px; }
  .rot-bar { height: 6px; border-radius: 3px; }
  .rot-bar-full { width: 100%; background: rgba(134, 239, 172, 0.5); }
  .rot-bar-half { width: 45%; background: rgba(251, 191, 36, 0.5); }
  .rot-bar-low { width: 15%; background: rgba(252, 165, 165, 0.5); }
  .rot-bar-grow-1 { width: 50%; background: rgba(134, 239, 172, 0.5); }
  .rot-bar-grow-2 { width: 72%; background: rgba(134, 239, 172, 0.6); }
  .rot-bar-grow-3 { width: 100%; background: rgba(134, 239, 172, 0.7); }
  .rot-loss { font-family: var(--mono); font-size: 0.55rem; color: rgba(252, 165, 165, 0.5); padding-left: 72px; margin: 0.2rem 0; }
  .rot-gain { font-family: var(--mono); font-size: 0.55rem; color: rgba(134, 239, 172, 0.4); padding-left: 72px; margin: 0.2rem 0; }
  .rot-summary { font-family: var(--serif); font-size: 0.8rem; margin-top: 1rem; line-height: 1.6; }
  .rot-without .rot-summary { color: rgba(252, 165, 165, 0.6); }
  .rot-with .rot-summary { color: rgba(134, 239, 172, 0.6); }
  .cycle { margin-top: 1.5rem; display: flex; flex-direction: column; gap: 0; }
  .cycle-step { display: flex; gap: 1rem; align-items: flex-start; padding: 1rem 0; }
  .cycle-num { font-family: var(--mono); font-size: 0.65rem; color: var(--blue); width: 20px; height: 20px; border: 1px solid var(--blue-border-dim); display: flex; align-items: center; justify-content: center; flex-shrink: 0; border-radius: 50%; margin-top: 0.15rem; }
  .cycle-content { display: flex; flex-direction: column; gap: 0.3rem; }
  .cycle-cmd { font-family: var(--mono); font-size: 0.8rem; color: var(--blue); }
  .cycle-desc { font-family: var(--serif); font-size: 0.9rem; color: var(--text-dim); line-height: 1.7; }
  .cycle-arrow { font-family: var(--mono); font-size: 0.7rem; color: rgba(255, 255, 255, 0.15); padding-left: 5px; }
  .cycle-return { margin-top: 0.5rem; padding: 0.6rem 0; border-top: 1px solid rgba(255, 255, 255, 0.04); }
  .cycle-return-label { font-family: var(--mono); font-size: 0.65rem; color: var(--blue-dim); letter-spacing: 0.05em; }
  .search-demo { margin-top: 1.5rem; }
  .search-note { margin-top: 0.8rem; font-size: 0.85rem !important; font-style: italic; }
  .building-blocks { display: grid; grid-template-columns: 1fr 1fr; gap: 1rem; margin: 1.5rem 0; }
  .block-card { border: 1px solid rgba(255, 255, 255, 0.06); padding: 1.2rem; transition: border-color 0.2s; }
  .block-card:hover { border-color: var(--blue-border-dim); }
  .block-name { font-family: var(--mono); font-size: 0.7rem; color: var(--blue); letter-spacing: 0.1em; text-transform: uppercase; margin-bottom: 0.8rem; }
  .block-card :global(p) { font-size: 0.85rem !important; color: var(--text-dim) !important; line-height: 1.7 !important; margin-bottom: 0 !important; }
  .tag-inline { font-family: var(--mono); font-size: 0.7rem; padding: 0.05rem 0.35rem; border: 1px solid; letter-spacing: 0.03em; }
  .tag-inline.tag-mode { border-color: rgba(126, 184, 218, 0.3); color: var(--blue); }
  .tag-inline.tag-plan { border-color: rgba(252, 211, 77, 0.3); color: rgba(252, 211, 77, 0.7); }
  .tag-inline.tag-effort { border-color: rgba(255, 255, 255, 0.12); color: var(--text-dim); }
  .cmd-inline { font-family: var(--mono); font-size: 0.9rem; color: var(--blue); }
  .vault-demo { margin-top: 1.5rem; }
  .vault-file { font-size: 0.7rem; line-height: 1.8; }
  .vault-frontmatter { border-left: 2px solid rgba(126, 184, 218, 0.3); padding-left: 0.8rem; margin-bottom: 0.8rem; }
  .vault-fm-line { color: rgba(255, 255, 255, 0.4); }
  .vault-key { color: var(--blue-dim); }
  .vault-body { color: rgba(255, 255, 255, 0.5); }
  .vault-body-line { min-height: 1.4em; }
  .vault-h2 { color: rgba(255, 255, 255, 0.65); font-weight: 500; }
  .vault-note { margin-top: 0.8rem; font-size: 0.85rem !important; font-style: italic; }
  :global(.cli-spacer) { height: 0.5rem; }
  .cross-sell { max-width: 800px; margin: 0 auto; padding: 3rem 2.5rem; border-top: 1px solid var(--rule); }
  .cross-sell p { font-family: var(--serif); font-size: 0.95rem; color: var(--text-dim); font-style: italic; text-align: center; line-height: 1.8; }
  .cross-sell a { color: var(--blue); text-decoration: none; transition: color 0.2s; }
  .cross-sell a:hover { color: var(--text); }
  @media (max-width: 640px) {
    .rot-comparison, .building-blocks { grid-template-columns: 1fr; }
    .warmup-line { flex-direction: column; gap: 0.2rem; }
    .warmup-label { min-width: auto; }
  }
</style>
