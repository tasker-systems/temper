<script>
  import Section from '$lib/components/landing/Section.svelte';
  import CliBlock from '$lib/components/landing/CliBlock.svelte';
  import Footer from '$lib/components/landing/Footer.svelte';
</script>

<svelte:head>
  <title>temper for agents — context that's always ready to hand</title>
  <meta name="description" content="Give your agents a persistent, structured, searchable context layer. Through CLI, MCP server, and generated skill files — the throughline isn't just for humans anymore." />
</svelte:head>

<section class="hero">
  <div class="hero-label">For agents</div>
  <h1>Context that's always <em>ready to hand</em></h1>
  <p class="tagline">Your agent is powerful — but every session, it starts from zero. Temper gives agents what they lack: a persistent, structured, searchable context layer that makes the throughline legible.</p>
  <div class="hero-ctas">
    <a href="/auth/login" class="primary">Get Started</a>
    <a href="https://github.com/tasker-systems/temper" class="secondary">View on GitHub</a>
  </div>
  <div class="cli-wrapper">
    <div class="transcript">
      <div class="message">
        <div class="role">agent <span class="role-via">via mcp</span></div>
        <div class="content agent-code">temper.search("payment retry strategy")</div>
      </div>
      <div class="message">
        <div class="role">vault</div>
        <div class="content">
          <div class="mcp-results">
            <div class="mcp-result"><span>decision/retry-backoff-strategy.md</span><span class="mcp-score">0.96</span></div>
            <div class="mcp-result"><span>session/2026-03-29-payment-service.md</span><span class="mcp-score">0.91</span></div>
            <div class="mcp-result"><span>research/idempotency-patterns.md</span><span class="mcp-score">0.84</span></div>
          </div>
        </div>
      </div>
      <div class="message">
        <div class="role">agent</div>
        <div class="content agent-text">I see we decided on exponential backoff with jitter (Mar 29). The research doc notes a P99 concern above 5 retries. I'll implement with a configurable <span class="hl">max_retries</span> defaulting to 4.</div>
      </div>
    </div>
  </div>
</section>

<Section label="The problem">
  <h2>Powerful but <em>forgetful</em></h2>
  <p>Claude Code, Cursor, Windsurf, Copilot — these tools can write code, analyze architecture, and plan implementations. But they carry nothing between sessions. No memory of yesterday's decision. No awareness of the constraint that shaped today's approach. No sense of where the project is headed or what's already been tried.</p>
  <p>You compensate by writing CLAUDE.md files, pasting context at the start of sessions, and manually steering the agent through decisions it should already know about. This works — until the project grows, the decisions multiply, and the context you need to inject exceeds what you can hold in your head.</p>
  <p>The agent needs the same throughline that you carry unconsciously: what we're building, why, what we've decided, and what's deferred. That throughline needs to be structured, searchable, and persistent. It needs to be a vault.</p>
</Section>

<Section label="Three pathways">
  <h2>One vault, three <em>interfaces</em></h2>
  <p>Agents reach the vault through whichever interface suits their integration model. The same structured knowledge, accessed three different ways.</p>
  <div class="pathways">
    <div class="pathway">
      <div class="pathway-icon">$</div>
      <div class="pathway-name">CLI</div>
      <div class="pathway-desc"><span class="cmd-inline">temper warmup</span>, <span class="cmd-inline">temper search</span>, <span class="cmd-inline">temper session save</span> — agents that can run shell commands get full vault access. Claude Code hooks call <span class="cmd-inline">temper warmup</span> automatically at session start.</div>
    </div>
    <div class="pathway">
      <div class="pathway-icon">⟡</div>
      <div class="pathway-name">MCP Server</div>
      <div class="pathway-desc">The Model Context Protocol server exposes vault operations as structured tools. Agents query <span class="cmd-inline">temper.search</span>, read resources, and write session notes through the protocol — no shell access required.</div>
    </div>
    <div class="pathway">
      <div class="pathway-icon">◇</div>
      <div class="pathway-name">Skill File</div>
      <div class="pathway-desc"><span class="cmd-inline">temper skill install</span> generates a Claude Code skill that teaches the agent your vault's structure, available commands, and workflow conventions. The agent learns how to use temper as a first-class capability.</div>
    </div>
  </div>
</Section>

<Section label="Session pre-warming">
  <h2>Start every session <em>informed</em></h2>
  <p>Add a startup hook to your project, and every new agent session begins with context — not a blank slate. The agent reads what happened, what's in progress, and what decisions shape the current work before writing a single line of code.</p>
  <CliBlock>
    <div class="config-file">
      <div class="config-comment">// .claude/settings.local.json</div>
      <div class="config-line">{'{'}</div>
      <div class="config-line" style="padding-left: 2ch"><span class="config-key">"hooks"</span>: {'{'}</div>
      <div class="config-line" style="padding-left: 4ch"><span class="config-key">"SessionStart"</span>: [{'{'}</div>
      <div class="config-line" style="padding-left: 6ch"><span class="config-key">"matcher"</span>: <span class="config-val">"startup"</span>,</div>
      <div class="config-line" style="padding-left: 6ch"><span class="config-key">"hooks"</span>: [{'{'}</div>
      <div class="config-line" style="padding-left: 8ch"><span class="config-key">"type"</span>: <span class="config-val">"command"</span>,</div>
      <div class="config-line" style="padding-left: 8ch"><span class="config-key">"command"</span>: <span class="config-val">"temper warmup --context myapp"</span></div>
      <div class="config-line" style="padding-left: 6ch">{'}'}]</div>
      <div class="config-line" style="padding-left: 4ch">{'}'}]</div>
      <div class="config-line" style="padding-left: 2ch">{'}'}</div>
      <div class="config-line">{'}'}</div>
    </div>
  </CliBlock>
  <div class="injects-list">
    <p>Every startup injects:</p>
    <div class="inject-items">
      <div class="inject-item"><span class="inject-dot"></span>In-progress tasks with mode and effort</div>
      <div class="inject-item"><span class="inject-dot"></span>Last 5 session summaries</div>
      <div class="inject-item"><span class="inject-dot"></span>Full content of the most recent session</div>
    </div>
  </div>
</Section>

<Section label="MCP server">
  <h2>Direct agent <em>integration</em></h2>
  <p>The MCP server exposes the vault as a set of structured tools that any MCP-compatible agent can call. No file system access needed — the agent queries the vault through the protocol and gets structured results back.</p>
  <div class="mcp-demo">
    <div class="transcript">
      <div class="message">
        <div class="role">agent <span class="role-via">via mcp</span></div>
        <div class="content agent-code">temper.warmup({'{'} context: "myapp" {'}'})</div>
      </div>
      <div class="message">
        <div class="role">vault</div>
        <div class="content">
          <div class="context-block">
            <div class="context-line"><span class="context-key">goal</span> api-v2-migration <span class="dim">(3 tasks, 2 complete)</span></div>
            <div class="context-line"><span class="context-key">active</span> client-sdk-update <span class="tag-sm tag-mode-sm">build</span> <span class="tag-sm tag-effort-sm">medium</span></div>
            <div class="context-line"><span class="context-key">prior</span> 4 sessions <span class="dim">(last: auth middleware, Mar 28)</span></div>
            <div class="context-line"><span class="context-key">decided</span> REST over GraphQL, JWT rotation</div>
            <div class="context-line"><span class="context-key">deferred</span> rate limiting, webhook signatures</div>
          </div>
        </div>
      </div>
      <div class="message">
        <div class="role">agent <span class="role-via">via mcp</span></div>
        <div class="content agent-code">temper.session_save({'{'} title: "Client SDK v2 migration", decisions: ["Kept backward compat for v1 clients"], next: "Integration tests for v1 → v2 upgrade path" {'}'})</div>
      </div>
      <div class="message">
        <div class="role">vault</div>
        <div class="content">
          <div class="mcp-confirm">Session saved. Vault updated. Next warmup will include this context.</div>
        </div>
      </div>
    </div>
  </div>
</Section>

<Section label="Skill generation">
  <h2>Teach the agent your <em>workflow</em></h2>
  <p>The generated skill file is a Claude Code skill that describes your vault's structure, your available commands, and your workflow conventions. The agent doesn't need to be told how to use temper — the skill makes it a first-class capability, the same way a plugin teaches an editor a new language.</p>
  <CliBlock>
    <div class="cli-prompt"><span class="flag">$</span> <span class="cmd">temper skill install</span></div>
    <div class="cli-output">
      <div class="skill-line">Generating skill from vault structure...</div>
      <div class="skill-line dim">  → 3 contexts, 8 goals, 47 tasks</div>
      <div class="skill-line dim">  → Modes: build / plan · Effort: small / medium / large</div>
      <div class="skill-line dim">  → Session template: goal / happened / decisions / next</div>
      <div class="skill-line">Installed to ~/.claude/commands/temper.md</div>
    </div>
  </CliBlock>
  <p>The skill evolves with your vault. As you add projects, refine templates, and develop conventions, <span class="cmd-inline">temper skill install</span> regenerates to match. The agent always has current instructions.</p>
</Section>

<Section label="The philosophy">
  <h2>If it can read files, it can use <em>temper</em></h2>
  <p>Temper doesn't require a specific agent, a specific IDE, or a specific workflow. The vault is markdown. The search is semantic. The protocol is MCP. Any tool that can read files — or call an MCP server — gets the full throughline.</p>
  <p>This is deliberate. The knowledge base is the unit of value, not the tool. Your vault works with Claude Code today and whatever comes next tomorrow. No vendor lock-in, no proprietary format, no walled garden. Your context belongs to you.</p>
</Section>

<div class="cross-sell">
  <p>Agents work best when <a href="/builders">humans temper the context</a>. Temper's session-over-session workflow gives builders and agents the same throughline — what we're building, why, what's decided, and what comes next.</p>
</div>

<Footer />

<style>
  .hero { min-height: 100vh; display: flex; flex-direction: column; justify-content: center; align-items: center; text-align: center; padding: 6rem 2.5rem 4rem; }
  .hero-label { font-family: var(--font-mono); font-size: 0.65rem; letter-spacing: 0.2em; text-transform: uppercase; color: var(--temper-blue); margin-bottom: 1.5rem; }
  .hero h1 { font-family: var(--font-serif); font-size: clamp(2.4rem, 5vw, 3.8rem); font-weight: 300; line-height: 1.2; margin-bottom: 1.5rem; letter-spacing: 0.02em; color: var(--parchment); }
  .hero h1 em { color: var(--temper-blue); font-style: italic; }
  .tagline { font-family: var(--font-serif); font-size: 1.1rem; color: var(--graphite); font-style: italic; max-width: 36em; margin-bottom: 3rem; line-height: 1.7; }
  .hero-ctas { display: flex; gap: 1rem; margin-bottom: 4rem; }
  .hero-ctas a { font-family: var(--font-mono); font-size: 0.8rem; padding: 0.6rem 1.5rem; text-decoration: none; letter-spacing: 0.05em; transition: all 0.2s; }
  .hero-ctas .primary { border: 1px solid var(--temper-blue-border); color: var(--temper-blue); }
  .hero-ctas .primary:hover { background: rgba(126, 184, 218, 0.1); }
  .hero-ctas .secondary { border: 1px solid rgba(255, 255, 255, 0.12); color: var(--graphite); }
  .hero-ctas .secondary:hover { border-color: rgba(255, 255, 255, 0.25); color: var(--chalk); }
  .cli-wrapper { width: 100%; max-width: 620px; }
  .transcript { border: 1px solid rgba(255, 255, 255, 0.06); padding: 1.2rem; font-family: var(--font-mono); font-size: 0.7rem; line-height: 1.8; text-align: left; }
  .message { margin-bottom: 1rem; }
  .message:last-child { margin-bottom: 0; }
  .message + .message { border-top: 1px solid rgba(255, 255, 255, 0.04); padding-top: 0.8rem; }
  .role { color: var(--temper-blue-dim); font-size: 0.6rem; margin-bottom: 0.4rem; }
  .role-via { color: rgba(255, 255, 255, 0.2); font-size: 0.55rem; }
  .content.agent-code { font-family: var(--font-mono); font-size: 0.7rem; color: rgba(255, 255, 255, 0.7); background: rgba(255, 255, 255, 0.02); padding: 0.4rem 0.6rem; border-left: 2px solid rgba(126, 184, 218, 0.2); }
  .content.agent-text { font-family: var(--font-serif); font-size: 0.8rem; color: rgba(255, 255, 255, 0.55); line-height: 1.7; }
  .hl { color: var(--temper-blue); font-family: var(--font-mono); font-size: 0.75rem; }
  .mcp-results { font-size: 0.7rem; color: var(--graphite); }
  .mcp-result { display: flex; justify-content: space-between; padding: 0.3rem 0; border-bottom: 1px solid rgba(255, 255, 255, 0.04); }
  .mcp-result:last-child { border-bottom: none; }
  .mcp-score { color: var(--temper-blue-dim); }
  .mcp-confirm { font-size: 0.7rem; color: rgba(134, 239, 172, 0.6); }
  .context-block { font-size: 0.65rem; line-height: 1.8; }
  .context-line { color: rgba(255, 255, 255, 0.45); }
  .context-key { color: var(--temper-blue-dim); display: inline-block; min-width: 64px; }
  .tag-sm { font-size: 0.55rem; padding: 0 0.3rem; border: 1px solid; letter-spacing: 0.03em; }
  .tag-mode-sm { border-color: rgba(126, 184, 218, 0.3); color: var(--temper-blue); }
  .tag-effort-sm { border-color: rgba(255, 255, 255, 0.12); color: var(--graphite); }
  .dim { color: rgba(255, 255, 255, 0.25); }
  .pathways { display: flex; flex-direction: column; gap: 1.5rem; margin-top: 1.5rem; }
  .pathway { display: grid; grid-template-columns: 32px 1fr; grid-template-rows: auto auto; gap: 0 1rem; align-items: start; }
  .pathway-icon { grid-row: 1 / 3; font-family: var(--font-mono); font-size: 1rem; color: var(--temper-blue); text-align: center; padding-top: 0.15rem; }
  .pathway-name { font-family: var(--font-mono); font-size: 0.8rem; color: var(--parchment); letter-spacing: 0.02em; }
  .pathway-desc { font-family: var(--font-serif); font-size: 0.88rem; color: var(--graphite); line-height: 1.7; margin-top: 0.3rem; }
  .cmd-inline { font-family: var(--font-mono); font-size: 0.78rem; color: var(--temper-blue); }
  .config-file { font-size: 0.7rem; line-height: 1.8; color: rgba(255, 255, 255, 0.4); }
  .config-comment { color: rgba(255, 255, 255, 0.2); font-style: italic; margin-bottom: 0.3rem; }
  .config-key { color: var(--temper-blue-dim); }
  .config-val { color: rgba(134, 239, 172, 0.5); }
  .injects-list { margin-top: 1rem; }
  .injects-list p { font-size: 0.85rem !important; margin-bottom: 0.6rem !important; }
  .inject-items { display: flex; flex-direction: column; gap: 0.5rem; }
  .inject-item { display: flex; align-items: baseline; gap: 0.8rem; font-family: var(--font-serif); font-size: 0.88rem; color: var(--chalk); }
  .inject-dot { width: 4px; height: 4px; background: var(--temper-blue); border-radius: 50%; flex-shrink: 0; margin-top: 0.45rem; }
  .mcp-demo { margin-top: 1.5rem; }
  .cli-output { margin-top: 0.5rem; }
  .skill-line { font-size: 0.7rem; color: rgba(255, 255, 255, 0.5); line-height: 1.8; }
  .skill-line.dim { color: rgba(255, 255, 255, 0.25); }
  .cross-sell { max-width: 800px; margin: 0 auto; padding: 3rem 2.5rem; border-top: 1px solid var(--rule); }
  .cross-sell p { font-family: var(--font-serif); font-size: 0.95rem; color: var(--graphite); font-style: italic; text-align: center; line-height: 1.8; }
  .cross-sell a { color: var(--temper-blue); text-decoration: none; transition: color 0.2s; }
  .cross-sell a:hover { color: var(--parchment); }
  @media (max-width: 640px) {
    .pathway { grid-template-columns: 1fr; }
    .pathway-icon { grid-row: auto; text-align: left; }
  }
</style>
