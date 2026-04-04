<script>
  import Section from '$lib/components/landing/Section.svelte';
  import Footer from '$lib/components/landing/Footer.svelte';
</script>

<svelte:head>
  <title>how temper works — from markdown to connected knowledge</title>
  <meta name="description" content="Your vault is markdown files on your machine. Temper connects them into a searchable knowledge graph and syncs them to the cloud. Everything resolves back to files you can read." />
</svelte:head>

<section class="hero">
  <div class="hero-label">How it works</div>
  <h1>From markdown to connected <em>knowledge</em></h1>
  <p class="tagline">
    Your vault is files on your machine. Temper connects them,
    indexes them, and syncs them to the cloud. Everything resolves
    back to markdown you can read.
  </p>
</section>

<!-- The Vault -->
<Section label="The vault">
  <h2>Markdown with <em>frontmatter</em></h2>
  <p>
    Every piece of knowledge in temper is a markdown file with YAML frontmatter.
    The frontmatter carries structure — type, status, relationships, mode, effort.
    The content carries the thinking. Together they form a file that's simultaneously
    human-readable, git-trackable, and natively understood by language models.
  </p>
  <div class="vault-visual">
    <svg viewBox="0 0 560 200" xmlns="http://www.w3.org/2000/svg" class="vault-svg">
      <g transform="translate(0, 10)">
        <rect x="0" y="0" width="100" height="130" rx="4" fill="rgba(126,184,218,0.1)" stroke="rgba(126,184,218,0.35)" stroke-width="0.5"/>
        <rect x="0" y="0" width="100" height="32" rx="4" fill="rgba(126,184,218,0.15)"/>
        <text x="50" y="20" text-anchor="middle" fill="rgba(126,184,218,0.85)" font-family="var(--mono)" font-size="8" letter-spacing="0.1em">GOAL</text>
        <line x1="8" y1="44" x2="60" y2="44" stroke="rgba(255,255,255,0.18)" stroke-width="0.5"/>
        <line x1="8" y1="54" x2="80" y2="54" stroke="rgba(255,255,255,0.13)" stroke-width="0.5"/>
        <line x1="8" y1="64" x2="72" y2="64" stroke="rgba(255,255,255,0.13)" stroke-width="0.5"/>
        <line x1="8" y1="80" x2="88" y2="80" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>
        <line x1="8" y1="90" x2="68" y2="90" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>
        <line x1="8" y1="100" x2="82" y2="100" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>
        <line x1="8" y1="110" x2="50" y2="110" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>

        <rect x="116" y="20" width="100" height="130" rx="4" fill="rgba(126,184,218,0.1)" stroke="rgba(126,184,218,0.35)" stroke-width="0.5"/>
        <rect x="116" y="20" width="100" height="32" rx="4" fill="rgba(126,184,218,0.15)"/>
        <text x="166" y="40" text-anchor="middle" fill="rgba(126,184,218,0.85)" font-family="var(--mono)" font-size="8" letter-spacing="0.1em">TASK</text>
        <line x1="124" y1="64" x2="176" y2="64" stroke="rgba(255,255,255,0.18)" stroke-width="0.5"/>
        <line x1="124" y1="74" x2="196" y2="74" stroke="rgba(255,255,255,0.13)" stroke-width="0.5"/>
        <line x1="124" y1="84" x2="184" y2="84" stroke="rgba(255,255,255,0.13)" stroke-width="0.5"/>
        <line x1="124" y1="100" x2="204" y2="100" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>
        <line x1="124" y1="110" x2="170" y2="110" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>
        <line x1="124" y1="120" x2="192" y2="120" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>

        <rect x="232" y="5" width="100" height="130" rx="4" fill="rgba(134,239,172,0.08)" stroke="rgba(134,239,172,0.28)" stroke-width="0.5"/>
        <rect x="232" y="5" width="100" height="32" rx="4" fill="rgba(134,239,172,0.13)"/>
        <text x="282" y="25" text-anchor="middle" fill="rgba(134,239,172,0.8)" font-family="var(--mono)" font-size="8" letter-spacing="0.1em">SESSION</text>
        <line x1="240" y1="49" x2="292" y2="49" stroke="rgba(255,255,255,0.18)" stroke-width="0.5"/>
        <line x1="240" y1="59" x2="312" y2="59" stroke="rgba(255,255,255,0.13)" stroke-width="0.5"/>
        <line x1="240" y1="69" x2="300" y2="69" stroke="rgba(255,255,255,0.13)" stroke-width="0.5"/>
        <line x1="240" y1="85" x2="320" y2="85" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>
        <line x1="240" y1="95" x2="286" y2="95" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>
        <line x1="240" y1="105" x2="308" y2="105" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>

        <rect x="348" y="30" width="100" height="130" rx="4" fill="rgba(252,211,77,0.07)" stroke="rgba(252,211,77,0.25)" stroke-width="0.5"/>
        <rect x="348" y="30" width="100" height="32" rx="4" fill="rgba(252,211,77,0.12)"/>
        <text x="398" y="50" text-anchor="middle" fill="rgba(252,211,77,0.8)" font-family="var(--mono)" font-size="8" letter-spacing="0.1em">DECISION</text>
        <line x1="356" y1="74" x2="408" y2="74" stroke="rgba(255,255,255,0.18)" stroke-width="0.5"/>
        <line x1="356" y1="84" x2="428" y2="84" stroke="rgba(255,255,255,0.13)" stroke-width="0.5"/>
        <line x1="356" y1="94" x2="416" y2="94" stroke="rgba(255,255,255,0.13)" stroke-width="0.5"/>
        <line x1="356" y1="110" x2="436" y2="110" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>
        <line x1="356" y1="120" x2="400" y2="120" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>

        <rect x="464" y="10" width="96" height="130" rx="4" fill="rgba(148,163,184,0.07)" stroke="rgba(148,163,184,0.25)" stroke-width="0.5"/>
        <rect x="464" y="10" width="96" height="32" rx="4" fill="rgba(148,163,184,0.12)"/>
        <text x="512" y="30" text-anchor="middle" fill="rgba(148,163,184,0.75)" font-family="var(--mono)" font-size="8" letter-spacing="0.1em">RESEARCH</text>
        <line x1="472" y1="54" x2="524" y2="54" stroke="rgba(255,255,255,0.18)" stroke-width="0.5"/>
        <line x1="472" y1="64" x2="544" y2="64" stroke="rgba(255,255,255,0.13)" stroke-width="0.5"/>
        <line x1="472" y1="74" x2="532" y2="74" stroke="rgba(255,255,255,0.13)" stroke-width="0.5"/>
        <line x1="472" y1="90" x2="548" y2="90" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>
        <line x1="472" y1="100" x2="510" y2="100" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>
      </g>
    </svg>
  </div>
  <p>
    The vault is a directory on your machine. Browse it in any editor,
    in Obsidian, or on GitHub. No database, no proprietary format.
    Import existing docs with <span class="cmd-inline">temper add</span> —
    temper extracts markdown and adds the frontmatter structure.
  </p>
</Section>

<!-- Connected Knowledge -->
<Section label="Connected knowledge">
  <h2>Every idea knows its <em>neighbors</em></h2>
  <p>
    Files in your vault aren't isolated — they're connected. A task relates to
    a goal. A session produces decisions. Research informs the next task. Temper
    tracks these relationships and builds a knowledge graph that grows with
    every session. When you search, you don't just find a file — you find
    the constellation of ideas around it.
  </p>
  <div class="graph-visual">
    <svg viewBox="0 0 560 340" xmlns="http://www.w3.org/2000/svg" class="graph-svg">
      <!-- Connection lines -->
      <line x1="280" y1="60" x2="160" y2="130" stroke="rgba(126,184,218,0.25)" stroke-width="0.5"/>
      <line x1="280" y1="60" x2="380" y2="120" stroke="rgba(126,184,218,0.25)" stroke-width="0.5"/>
      <line x1="280" y1="60" x2="240" y2="150" stroke="rgba(126,184,218,0.2)" stroke-width="0.5"/>
      <line x1="160" y1="130" x2="90" y2="200" stroke="rgba(134,239,172,0.22)" stroke-width="0.5"/>
      <line x1="160" y1="130" x2="180" y2="220" stroke="rgba(134,239,172,0.22)" stroke-width="0.5"/>
      <line x1="380" y1="120" x2="420" y2="200" stroke="rgba(134,239,172,0.22)" stroke-width="0.5"/>
      <line x1="380" y1="120" x2="340" y2="210" stroke="rgba(134,239,172,0.22)" stroke-width="0.5"/>
      <line x1="240" y1="150" x2="260" y2="230" stroke="rgba(134,239,172,0.18)" stroke-width="0.5"/>
      <line x1="90" y1="200" x2="130" y2="280" stroke="rgba(252,211,77,0.22)" stroke-width="0.5"/>
      <line x1="420" y1="200" x2="470" y2="270" stroke="rgba(252,211,77,0.22)" stroke-width="0.5"/>
      <line x1="340" y1="210" x2="380" y2="290" stroke="rgba(252,211,77,0.18)" stroke-width="0.5"/>
      <line x1="500" y1="100" x2="380" y2="120" stroke="rgba(148,163,184,0.18)" stroke-width="0.5"/>
      <line x1="500" y1="100" x2="470" y2="270" stroke="rgba(148,163,184,0.14)" stroke-width="0.5"/>
      <line x1="60" y1="110" x2="160" y2="130" stroke="rgba(148,163,184,0.18)" stroke-width="0.5"/>
      <line x1="60" y1="110" x2="130" y2="280" stroke="rgba(148,163,184,0.14)" stroke-width="0.5"/>
      <!-- Feedback lines (dashed) -->
      <line x1="130" y1="280" x2="240" y2="150" stroke="rgba(126,184,218,0.16)" stroke-width="0.5" stroke-dasharray="3,4"/>
      <line x1="470" y1="270" x2="380" y2="120" stroke="rgba(126,184,218,0.16)" stroke-width="0.5" stroke-dasharray="3,4"/>

      <!-- Goal node -->
      <circle cx="280" cy="60" r="14" fill="rgba(126,184,218,0.16)" stroke="rgba(126,184,218,0.55)" stroke-width="0.5"/>
      <circle cx="280" cy="60" r="4" fill="rgba(126,184,218,0.8)"/>
      <text x="280" y="38" text-anchor="middle" fill="rgba(126,184,218,0.7)" font-family="var(--mono)" font-size="8" letter-spacing="0.08em">goal</text>

      <!-- Task nodes -->
      <circle cx="160" cy="130" r="10" fill="rgba(126,184,218,0.12)" stroke="rgba(126,184,218,0.45)" stroke-width="0.5"/>
      <circle cx="160" cy="130" r="3" fill="rgba(126,184,218,0.7)"/>
      <circle cx="380" cy="120" r="10" fill="rgba(126,184,218,0.12)" stroke="rgba(126,184,218,0.45)" stroke-width="0.5"/>
      <circle cx="380" cy="120" r="3" fill="rgba(126,184,218,0.7)"/>
      <circle cx="240" cy="150" r="8" fill="rgba(126,184,218,0.1)" stroke="rgba(126,184,218,0.35)" stroke-width="0.5"/>
      <circle cx="240" cy="150" r="2.5" fill="rgba(126,184,218,0.6)"/>
      <text x="135" y="118" text-anchor="middle" fill="rgba(126,184,218,0.6)" font-family="var(--mono)" font-size="7">task</text>
      <text x="405" y="110" text-anchor="middle" fill="rgba(126,184,218,0.6)" font-family="var(--mono)" font-size="7">task</text>

      <!-- Session nodes -->
      <circle cx="90" cy="200" r="7" fill="rgba(134,239,172,0.1)" stroke="rgba(134,239,172,0.4)" stroke-width="0.5"/>
      <circle cx="90" cy="200" r="2.5" fill="rgba(134,239,172,0.7)"/>
      <circle cx="180" cy="220" r="6" fill="rgba(134,239,172,0.08)" stroke="rgba(134,239,172,0.35)" stroke-width="0.5"/>
      <circle cx="180" cy="220" r="2" fill="rgba(134,239,172,0.6)"/>
      <circle cx="420" cy="200" r="7" fill="rgba(134,239,172,0.1)" stroke="rgba(134,239,172,0.4)" stroke-width="0.5"/>
      <circle cx="420" cy="200" r="2.5" fill="rgba(134,239,172,0.7)"/>
      <circle cx="340" cy="210" r="6" fill="rgba(134,239,172,0.08)" stroke="rgba(134,239,172,0.35)" stroke-width="0.5"/>
      <circle cx="340" cy="210" r="2" fill="rgba(134,239,172,0.6)"/>
      <circle cx="260" cy="230" r="5" fill="rgba(134,239,172,0.07)" stroke="rgba(134,239,172,0.28)" stroke-width="0.5"/>
      <circle cx="260" cy="230" r="1.5" fill="rgba(134,239,172,0.55)"/>
      <text x="66" y="192" text-anchor="middle" fill="rgba(134,239,172,0.55)" font-family="var(--mono)" font-size="7">session</text>

      <!-- Decision nodes -->
      <circle cx="130" cy="280" r="6" fill="rgba(252,211,77,0.1)" stroke="rgba(252,211,77,0.35)" stroke-width="0.5"/>
      <circle cx="130" cy="280" r="2" fill="rgba(252,211,77,0.7)"/>
      <circle cx="470" cy="270" r="6" fill="rgba(252,211,77,0.1)" stroke="rgba(252,211,77,0.35)" stroke-width="0.5"/>
      <circle cx="470" cy="270" r="2" fill="rgba(252,211,77,0.7)"/>
      <circle cx="380" cy="290" r="5" fill="rgba(252,211,77,0.07)" stroke="rgba(252,211,77,0.28)" stroke-width="0.5"/>
      <circle cx="380" cy="290" r="1.5" fill="rgba(252,211,77,0.6)"/>
      <text x="155" y="290" text-anchor="start" fill="rgba(252,211,77,0.55)" font-family="var(--mono)" font-size="7">decision</text>

      <!-- Research nodes -->
      <circle cx="500" cy="100" r="6" fill="rgba(148,163,184,0.08)" stroke="rgba(148,163,184,0.35)" stroke-width="0.5"/>
      <circle cx="500" cy="100" r="2" fill="rgba(148,163,184,0.6)"/>
      <circle cx="60" cy="110" r="6" fill="rgba(148,163,184,0.08)" stroke="rgba(148,163,184,0.35)" stroke-width="0.5"/>
      <circle cx="60" cy="110" r="2" fill="rgba(148,163,184,0.6)"/>
      <text x="520" y="96" text-anchor="start" fill="rgba(148,163,184,0.5)" font-family="var(--mono)" font-size="7">research</text>

      <!-- Ambient dots -->
      <circle cx="320" cy="170" r="1.5" fill="rgba(126,184,218,0.28)"/>
      <circle cx="200" cy="180" r="1" fill="rgba(134,239,172,0.25)"/>
      <circle cx="450" cy="160" r="1.5" fill="rgba(126,184,218,0.22)"/>
      <circle cx="110" cy="250" r="1" fill="rgba(252,211,77,0.2)"/>
      <circle cx="300" cy="310" r="1" fill="rgba(148,163,184,0.2)"/>
      <circle cx="510" cy="220" r="1" fill="rgba(134,239,172,0.2)"/>
      <circle cx="40" cy="180" r="1" fill="rgba(126,184,218,0.18)"/>
      <circle cx="530" cy="300" r="1" fill="rgba(252,211,77,0.18)"/>
    </svg>
  </div>
  <p>
    The dashed lines are the interesting ones — decisions feeding back to
    inform future tasks, research shaping goals. The graph captures the
    connections that context rot erases. When an agent runs
    <span class="cmd-inline">temper warmup</span>, it reads not just the current
    task but the neighborhood of decisions and sessions that shaped it.
  </p>
</Section>

<!-- Temper Cloud -->
<Section label="Temper Cloud">
  <h2>Your vault, <em>everywhere</em></h2>
  <p>
    Temper Cloud is Postgres-native — a single source of truth for your
    knowledge base, with pgvector powering semantic search across everything
    you've written. Your local vault stays as markdown files on your machine.
    The cloud makes them searchable, syncable, and accessible from any device.
  </p>
  <div class="cloud-visual">
    <svg viewBox="0 0 560 240" xmlns="http://www.w3.org/2000/svg" class="cloud-svg">
      <rect x="0" y="20" width="200" height="200" rx="8" fill="none" stroke="rgba(126,184,218,0.25)" stroke-width="0.5" stroke-dasharray="4,4"/>
      <text x="100" y="12" text-anchor="middle" fill="rgba(126,184,218,0.7)" font-family="var(--mono)" font-size="9" letter-spacing="0.1em">YOUR VAULT</text>

      <rect x="16" y="44" width="56" height="68" rx="3" fill="rgba(126,184,218,0.08)" stroke="rgba(126,184,218,0.28)" stroke-width="0.5"/>
      <line x1="22" y1="56" x2="56" y2="56" stroke="rgba(255,255,255,0.14)" stroke-width="0.5"/>
      <line x1="22" y1="64" x2="62" y2="64" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>
      <line x1="22" y1="72" x2="50" y2="72" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>
      <line x1="22" y1="80" x2="58" y2="80" stroke="rgba(255,255,255,0.08)" stroke-width="0.5"/>

      <rect x="80" y="54" width="56" height="68" rx="3" fill="rgba(134,239,172,0.07)" stroke="rgba(134,239,172,0.22)" stroke-width="0.5"/>
      <line x1="86" y1="66" x2="120" y2="66" stroke="rgba(255,255,255,0.14)" stroke-width="0.5"/>
      <line x1="86" y1="74" x2="126" y2="74" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>
      <line x1="86" y1="82" x2="114" y2="82" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>

      <rect x="144" y="38" width="48" height="68" rx="3" fill="rgba(252,211,77,0.06)" stroke="rgba(252,211,77,0.2)" stroke-width="0.5"/>
      <line x1="150" y1="50" x2="176" y2="50" stroke="rgba(255,255,255,0.14)" stroke-width="0.5"/>
      <line x1="150" y1="58" x2="182" y2="58" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>
      <line x1="150" y1="66" x2="170" y2="66" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/>

      <rect x="20" y="140" width="164" height="28" rx="4" fill="rgba(255,255,255,0.03)" stroke="rgba(255,255,255,0.14)" stroke-width="0.5"/>
      <text x="102" y="158" text-anchor="middle" fill="rgba(255,255,255,0.4)" font-family="var(--mono)" font-size="7">.temper/manifest.json</text>

      <!-- Sync arrows -->
      <line x1="220" y1="100" x2="340" y2="100" stroke="rgba(126,184,218,0.45)" stroke-width="0.5"/>
      <polygon points="336,97 342,100 336,103" fill="rgba(126,184,218,0.45)"/>
      <line x1="340" y1="140" x2="220" y2="140" stroke="rgba(134,239,172,0.45)" stroke-width="0.5"/>
      <polygon points="224,137 218,140 224,143" fill="rgba(134,239,172,0.45)"/>

      <text x="280" y="90" text-anchor="middle" fill="rgba(126,184,218,0.55)" font-family="var(--mono)" font-size="7">push</text>
      <text x="280" y="156" text-anchor="middle" fill="rgba(134,239,172,0.55)" font-family="var(--mono)" font-size="7">pull</text>
      <text x="280" y="120" text-anchor="middle" fill="rgba(255,255,255,0.35)" font-family="var(--mono)" font-size="8">temper sync</text>

      <rect x="360" y="20" width="200" height="200" rx="8" fill="rgba(126,184,218,0.05)" stroke="rgba(126,184,218,0.25)" stroke-width="0.5"/>
      <text x="460" y="12" text-anchor="middle" fill="rgba(126,184,218,0.7)" font-family="var(--mono)" font-size="9" letter-spacing="0.1em">TEMPER CLOUD</text>

      <circle cx="410" cy="70" r="8" fill="rgba(126,184,218,0.12)" stroke="rgba(126,184,218,0.4)" stroke-width="0.5"/>
      <circle cx="410" cy="70" r="2.5" fill="rgba(126,184,218,0.7)"/>
      <circle cx="470" cy="90" r="6" fill="rgba(134,239,172,0.1)" stroke="rgba(134,239,172,0.35)" stroke-width="0.5"/>
      <circle cx="470" cy="90" r="2" fill="rgba(134,239,172,0.6)"/>
      <circle cx="440" cy="130" r="5" fill="rgba(252,211,77,0.08)" stroke="rgba(252,211,77,0.28)" stroke-width="0.5"/>
      <circle cx="440" cy="130" r="1.5" fill="rgba(252,211,77,0.6)"/>
      <circle cx="510" cy="120" r="6" fill="rgba(126,184,218,0.1)" stroke="rgba(126,184,218,0.35)" stroke-width="0.5"/>
      <circle cx="510" cy="120" r="2" fill="rgba(126,184,218,0.6)"/>
      <circle cx="390" cy="110" r="5" fill="rgba(134,239,172,0.08)" stroke="rgba(134,239,172,0.28)" stroke-width="0.5"/>
      <circle cx="390" cy="110" r="1.5" fill="rgba(134,239,172,0.55)"/>

      <line x1="410" y1="70" x2="470" y2="90" stroke="rgba(126,184,218,0.18)" stroke-width="0.5"/>
      <line x1="410" y1="70" x2="440" y2="130" stroke="rgba(126,184,218,0.15)" stroke-width="0.5"/>
      <line x1="470" y1="90" x2="510" y2="120" stroke="rgba(126,184,218,0.15)" stroke-width="0.5"/>
      <line x1="440" y1="130" x2="390" y2="110" stroke="rgba(134,239,172,0.15)" stroke-width="0.5"/>
      <line x1="390" y1="110" x2="410" y2="70" stroke="rgba(126,184,218,0.12)" stroke-width="0.5"/>

      <rect x="380" y="160" width="160" height="28" rx="4" fill="rgba(126,184,218,0.07)" stroke="rgba(126,184,218,0.22)" stroke-width="0.5"/>
      <text x="460" y="178" text-anchor="middle" fill="rgba(126,184,218,0.6)" font-family="var(--mono)" font-size="7">pgvector · semantic search</text>

      <rect x="380" y="196" width="160" height="20" rx="4" fill="rgba(134,239,172,0.05)" stroke="rgba(134,239,172,0.18)" stroke-width="0.5"/>
      <text x="460" y="210" text-anchor="middle" fill="rgba(134,239,172,0.55)" font-family="var(--mono)" font-size="7">MCP server · agent access</text>
    </svg>
  </div>
</Section>

<!-- Sync -->
<Section label="Sync">
  <h2>Changes flow, conflicts <em>resolve</em></h2>
  <p>
    <span class="cmd-inline">temper sync</span> uses a manifest-based protocol.
    Your vault keeps a local manifest that records the last-known state of every
    resource — a three-way comparison between the file on your machine, the
    manifest's record, and the server's content. Non-conflicting changes
    merge automatically at the paragraph level using Rust-native diffing.
    Genuine conflicts get annotated in a <span class="cmd-inline">.conflict.md</span>
    file for human resolution.
  </p>
  <div class="sync-visual">
    <svg viewBox="0 0 560 160" xmlns="http://www.w3.org/2000/svg" class="sync-svg">
      <text x="93" y="16" text-anchor="middle" fill="rgba(126,184,218,0.7)" font-family="var(--mono)" font-size="8" letter-spacing="0.08em">local file</text>
      <text x="280" y="16" text-anchor="middle" fill="rgba(255,255,255,0.45)" font-family="var(--mono)" font-size="8" letter-spacing="0.08em">manifest</text>
      <text x="467" y="16" text-anchor="middle" fill="rgba(126,184,218,0.7)" font-family="var(--mono)" font-size="8" letter-spacing="0.08em">server</text>

      <rect x="43" y="28" width="100" height="120" rx="4" fill="rgba(126,184,218,0.1)" stroke="rgba(126,184,218,0.35)" stroke-width="0.5"/>
      <rect x="51" y="42" width="84" height="8" rx="2" fill="rgba(126,184,218,0.15)"/>
      <rect x="51" y="56" width="60" height="6" rx="2" fill="rgba(255,255,255,0.07)"/>
      <rect x="51" y="68" width="72" height="6" rx="2" fill="rgba(134,239,172,0.2)" stroke="rgba(134,239,172,0.35)" stroke-width="0.5"/>
      <rect x="51" y="80" width="48" height="6" rx="2" fill="rgba(255,255,255,0.07)"/>
      <rect x="51" y="92" width="68" height="6" rx="2" fill="rgba(255,255,255,0.07)"/>
      <text x="93" y="118" text-anchor="middle" fill="rgba(134,239,172,0.6)" font-family="var(--mono)" font-size="6">changed</text>

      <rect x="230" y="28" width="100" height="120" rx="4" fill="rgba(255,255,255,0.03)" stroke="rgba(255,255,255,0.14)" stroke-width="0.5"/>
      <text x="280" y="60" text-anchor="middle" fill="rgba(255,255,255,0.35)" font-family="var(--mono)" font-size="7">sha256:a3f...</text>
      <text x="280" y="80" text-anchor="middle" fill="rgba(255,255,255,0.25)" font-family="var(--mono)" font-size="7">synced_at</text>
      <text x="280" y="95" text-anchor="middle" fill="rgba(255,255,255,0.25)" font-family="var(--mono)" font-size="7">2026-04-02</text>
      <text x="280" y="118" text-anchor="middle" fill="rgba(255,255,255,0.35)" font-family="var(--mono)" font-size="6">baseline</text>

      <rect x="417" y="28" width="100" height="120" rx="4" fill="rgba(126,184,218,0.07)" stroke="rgba(126,184,218,0.28)" stroke-width="0.5"/>
      <rect x="425" y="42" width="84" height="8" rx="2" fill="rgba(126,184,218,0.15)"/>
      <rect x="425" y="56" width="60" height="6" rx="2" fill="rgba(255,255,255,0.07)"/>
      <rect x="425" y="68" width="72" height="6" rx="2" fill="rgba(255,255,255,0.07)"/>
      <rect x="425" y="80" width="48" height="6" rx="2" fill="rgba(126,184,218,0.18)" stroke="rgba(126,184,218,0.35)" stroke-width="0.5"/>
      <rect x="425" y="92" width="68" height="6" rx="2" fill="rgba(255,255,255,0.07)"/>
      <text x="467" y="118" text-anchor="middle" fill="rgba(126,184,218,0.55)" font-family="var(--mono)" font-size="6">changed</text>

      <line x1="148" y1="88" x2="226" y2="88" stroke="rgba(255,255,255,0.22)" stroke-width="0.5" stroke-dasharray="3,3"/>
      <line x1="334" y1="88" x2="413" y2="88" stroke="rgba(255,255,255,0.22)" stroke-width="0.5" stroke-dasharray="3,3"/>

      <text x="187" y="80" text-anchor="middle" fill="rgba(255,255,255,0.3)" font-family="var(--mono)" font-size="6">diff</text>
      <text x="373" y="80" text-anchor="middle" fill="rgba(255,255,255,0.3)" font-family="var(--mono)" font-size="6">diff</text>
    </svg>
  </div>
  <p>
    Your vault can also live in a git repo, sync to Obsidian, or coexist
    with anything else that reads files. Temper's sync is self-contained —
    it doesn't depend on or interfere with external version control.
  </p>
</Section>

<!-- Resolution -->
<Section label="Resolution">
  <h2>Everything resolves to <em>markdown</em></h2>
  <p>
    This is the invariant. Whatever happens in the cloud — search indexing,
    vector embeddings, knowledge graph construction, team access, agent queries —
    it all resolves back to markdown files with frontmatter in your vault.
    The files are the source of truth. The cloud is a lens on them.
    If temper disappeared tomorrow, your knowledge would still be there:
    readable, portable, yours.
  </p>
</Section>

<Footer />

<style>
  .hero { min-height: 60vh; display: flex; flex-direction: column; justify-content: center; align-items: center; text-align: center; padding: 6rem 2.5rem 2rem; }
  .hero-label { font-family: var(--mono); font-size: 0.65rem; letter-spacing: 0.2em; text-transform: uppercase; color: var(--blue); margin-bottom: 1.5rem; }
  .hero h1 { font-family: var(--serif); font-size: clamp(2.4rem, 5vw, 3.8rem); font-weight: 300; line-height: 1.2; margin-bottom: 1.5rem; letter-spacing: 0.02em; color: var(--text); }
  .hero h1 em { color: var(--blue); font-style: italic; }
  .tagline { font-family: var(--serif); font-size: 1.1rem; color: var(--text-dim); font-style: italic; max-width: 36em; line-height: 1.7; }
  .cmd-inline { font-family: var(--mono); font-size: 0.85rem; color: var(--blue); }
  .vault-visual, .graph-visual, .cloud-visual, .sync-visual { margin: 2rem 0 1.5rem; }
  .vault-svg, .graph-svg, .cloud-svg, .sync-svg { width: 100%; max-width: 560px; display: block; }
</style>
