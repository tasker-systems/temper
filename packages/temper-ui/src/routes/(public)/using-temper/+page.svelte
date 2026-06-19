<svelte:head>
  <title>Reference — temper</title>
</svelte:head>

<div class="docs">
  <h1>Reference</h1>
  <p class="lede">
    The operational reference for the temper CLI, cloud sync, and MCP server.
    For the conceptual frame — what Temper is, and why — see
    <a href="/theory">Theory</a>.
  </p>

  <!-- ── Install ───────────────────────────────────────────────────── -->
  <section>
    <h2>Install</h2>
    <p>
      macOS (Apple Silicon) and Linux (x86_64):
    </p>
    <div class="cli-block">
      <pre><code>curl -fsSL https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.sh | sh</code></pre>
    </div>
    <p>Windows (x86_64, PowerShell):</p>
    <div class="cli-block">
      <pre><code>irm https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.ps1 | iex</code></pre>
    </div>
    <p>
      Windows support is experimental in the 0.1.x line — file issues at
      <a href="https://github.com/tasker-systems/temper/issues">github.com/tasker-systems/temper/issues</a>
      if you hit problems.
    </p>
    <p>
      To pin a version, pass <code>--version vX.Y.Z</code> to the install
      script. See
      <a href="https://github.com/tasker-systems/temper/blob/main/docs/guides/install.md">docs/guides/install.md</a>
      for uninstall instructions and Linux arm64 / Intel Mac notes.
    </p>

    <h3>Build from source</h3>
    <p>
      If you'd rather compile locally (Intel Mac, Linux arm64, or just
      to hack on temper itself):
    </p>
    <div class="cli-block">
      <pre><code>git clone https://github.com/tasker-systems/temper.git
cd temper
cargo install --path crates/temper-cli --features embed,extract,hnsw</code></pre>
    </div>
    <p>
      The <code>embed</code> feature pulls in ONNX Runtime for local
      embeddings; <code>extract</code> enables document ingestion via
      kreuzberg; <code>hnsw</code> enables the local vector index.
      Drop any you don't need.
    </p>
  </section>

  <!-- ── Getting started ──────────────────────────────────────────── -->
  <section>
    <h2>Getting started</h2>
    <p>
      A vault is a directory of markdown files with YAML frontmatter. Temper
      resolves its config from <code>~/.config/temper/config.toml</code> or
      a per-project <code>.temper/config.toml</code>.
    </p>
    <table>
      <tbody>
        <tr><td><code>temper init</code></td><td>Initialise a new vault — asks how you work, writes config.</td></tr>
        <tr><td><code>temper context add &lt;name&gt;</code></td><td>Subscribe to a context (project). Contexts keep resources scoped.</td></tr>
        <tr><td><code>temper add &lt;path&gt; --context &lt;ctx&gt;</code></td><td>Import a file, URL, or directory into the vault. Extracts markdown and adds frontmatter.</td></tr>
        <tr><td><code>temper add &lt;path&gt; --dir --context &lt;ctx&gt;</code></td><td>Import every file in a directory.</td></tr>
        <tr><td><code>temper warmup --context &lt;ctx&gt;</code></td><td>Session primer — recent work, open tasks, recent decisions. Pipe into an agent's first prompt.</td></tr>
        <tr><td><code>temper status</code></td><td>Vault overview: contexts, resource counts, recent activity.</td></tr>
        <tr><td><code>temper check</code></td><td>Verify vault integrity and tool health.</td></tr>
        <tr><td><code>temper doctor</code></td><td>Validate frontmatter across the vault; repair drift.</td></tr>
        <tr><td><code>temper events</code></td><td>Show recent vault events.</td></tr>
      </tbody>
    </table>
  </section>

  <!-- ── Resources ─────────────────────────────────────────────────── -->
  <section>
    <h2>Resources</h2>
    <p>
      Every piece of knowledge in temper is a <em>resource</em> — a markdown
      file with typed frontmatter. Six doc types cover the vocabulary of
      structured work:
    </p>
    <table>
      <tbody>
        <tr><td><code>goal</code></td><td>Outcome — what we're building.</td></tr>
        <tr><td><code>research</code></td><td>Survey of alternatives, constraints, prior art.</td></tr>
        <tr><td><code>decision</code></td><td>What we chose, and why.</td></tr>
        <tr><td><code>task</code></td><td>Unit of work — what comes next.</td></tr>
        <tr><td><code>session</code></td><td>What happened — record of a working session.</td></tr>
        <tr><td><code>concept</code></td><td>Shared vocabulary within the vault.</td></tr>
      </tbody>
    </table>

    <h3>Create · list · show · update</h3>
    <table>
      <tbody>
        <tr><td><code>temper resource create --type &lt;t&gt; --title &lt;title&gt;</code></td><td>Create a new resource of any type.</td></tr>
        <tr><td><code>temper resource list</code></td><td>List all resources across types and contexts.</td></tr>
        <tr><td><code>temper resource list --type task</code></td><td>Filter by type (goals show task stage counts).</td></tr>
        <tr><td><code>temper resource show &lt;id&gt;</code></td><td>Show a resource. Accepts slug, slug suffix, or task sequence number.</td></tr>
        <tr><td><code>temper resource update &lt;id&gt; --title &lt;t&gt;</code></td><td>Update the title.</td></tr>
        <tr><td><code>temper resource update &lt;id&gt; --context-to &lt;ctx&gt;</code></td><td>Move the resource to a different context.</td></tr>
        <tr><td><code>temper resource update &lt;id&gt; --stage &lt;s&gt;</code></td><td>Task stage: <code>backlog</code>, <code>in-progress</code>, <code>done</code>, <code>cancelled</code>.</td></tr>
        <tr><td><code>temper resource update &lt;id&gt; --mode &lt;m&gt;</code></td><td>Task mode: <code>plan</code>, <code>build</code>.</td></tr>
        <tr><td><code>temper resource update &lt;id&gt; --effort &lt;e&gt;</code></td><td>Task effort: <code>small</code>, <code>medium</code>, <code>large</code>.</td></tr>
        <tr><td><code>temper resource update &lt;id&gt; --relates-to &lt;slug&gt;</code></td><td>Add a relationship. Repeatable. Similar flags: <code>--references</code>, <code>--depends-on</code>, <code>--extends</code>, <code>--preceded-by</code>, <code>--derived-from</code>.</td></tr>
        <tr><td><code>temper resource update &lt;id&gt; --branch &lt;name&gt; --pr &lt;url&gt;</code></td><td>Task-specific metadata: attach a git branch or PR URL.</td></tr>
      </tbody>
    </table>
  </section>

  <!-- ── Search ────────────────────────────────────────────────────── -->
  <section>
    <h2>Search</h2>
    <p>
      <code>temper search</code> combines full-text and semantic search with
      optional graph expansion — seeds spread along typed edges to surface
      neighbors.
    </p>
    <table>
      <tbody>
        <tr><td><code>temper search &lt;query&gt;</code></td><td>Hybrid search across the vault.</td></tr>
        <tr><td><code>--context &lt;ctx&gt;</code></td><td>Scope to one context.</td></tr>
        <tr><td><code>--doc-type &lt;type&gt;</code></td><td>Filter by doc type.</td></tr>
        <tr><td><code>--limit &lt;n&gt;</code></td><td>Cap results (default 10).</td></tr>
        <tr><td><code>--text-only</code></td><td>Skip semantic search (no local embedding needed).</td></tr>
        <tr><td><code>--seed &lt;id&gt;</code></td><td>Explicit seed resource for graph expansion. Repeatable.</td></tr>
        <tr><td><code>--depth &lt;n&gt;</code></td><td>Max hops for graph traversal (default 2, max 10).</td></tr>
        <tr><td><code>--no-graph</code></td><td>Disable graph expansion.</td></tr>
      </tbody>
    </table>
  </section>

  <!-- ── Knowledge graph ───────────────────────────────────────────── -->
  <section>
    <h2>Knowledge graph</h2>
    <p>
      Resources aren't isolated — frontmatter references
      (<code>relates_to</code>, <code>depends_on</code>, etc.) form a graph.
      Two commands populate it:
    </p>
    <table>
      <tbody>
        <tr><td><code>temper graph build</code></td><td>Scan markdown bodies for references and seed the frontmatter relationships. Additive — doesn't overwrite existing edges.</td></tr>
        <tr><td><code>temper graph build --dry-run</code></td><td>Preview edges without writing.</td></tr>
        <tr><td><code>temper graph index</code></td><td>Discover concepts via LLM judgment over the local HNSW index. Requires <code>temper index</code> to have been run first.</td></tr>
        <tr><td><code>temper index</code></td><td>Build the HNSW vector index over the vault. Needed for semantic search and the graph concept-discovery pass.</td></tr>
        <tr><td><code>temper index --full</code></td><td>Force a full rebuild.</td></tr>
      </tbody>
    </table>
  </section>

  <!-- ── Cloud sync ────────────────────────────────────────────────── -->
  <section>
    <h2>Cloud sync</h2>
    <p>
      Temper Cloud is a Postgres-native source of truth for your vault with
      pgvector-powered semantic search. Your local markdown files remain
      canonical — the cloud is a searchable, syncable lens on them.
    </p>

    <h3>Auth</h3>
    <table>
      <tbody>
        <tr><td><code>temper auth login</code></td><td>Browser-based OAuth with PKCE. Caches the token locally.</td></tr>
        <tr><td><code>temper auth status</code></td><td>Show current auth state.</td></tr>
        <tr><td><code>temper auth logout</code></td><td>Clear cached credentials.</td></tr>
        <tr><td><code>temper auth token</code></td><td>Store a JWT directly (for API-only clients or manual auth).</td></tr>
      </tbody>
    </table>

    <h3>Sync</h3>
    <p>
      Sync uses a manifest-based three-way compare between your local file,
      a manifest record, and the server. Non-conflicting changes auto-merge at
      the paragraph level; genuine conflicts are written to
      <code>.conflict.md</code> for human resolution.
    </p>
    <table>
      <tbody>
        <tr><td><code>temper sync run</code></td><td>Run a full sync cycle — push local changes, pull remote ones.</td></tr>
        <tr><td><code>temper sync run --context &lt;ctx&gt;</code></td><td>Sync only one or more contexts.</td></tr>
        <tr><td><code>temper sync status</code></td><td>Show what would change, without making changes.</td></tr>
        <tr><td><code>temper sync refresh</code></td><td>Refresh manifest from server — non-destructive interleave.</td></tr>
        <tr><td><code>temper sync reset</code></td><td>Reset manifest from scratch (backs up first, then full rebuild).</td></tr>
        <tr><td><code>temper pull &lt;resource-id&gt;</code></td><td>Pull a single resource by UUID.</td></tr>
        <tr><td><code>temper resource delete &lt;slug&gt; --type &lt;doctype&gt; [--force]</code></td><td>Delete a resource: cloud-first soft-delete, then local cleanup tail in local mode.</td></tr>
      </tbody>
    </table>

    <h3>Teams</h3>
    <table>
      <tbody>
        <tr><td><code>temper team join</code></td><td>Request to join a team — defaults to system access.</td></tr>
        <tr><td><code>temper team status</code></td><td>Check request or membership status.</td></tr>
        <tr><td><code>temper team leave</code></td><td>Withdraw a pending request or leave a team.</td></tr>
      </tbody>
    </table>
  </section>

  <!-- ── Agents ────────────────────────────────────────────────────── -->
  <section>
    <h2>Agents</h2>
    <p>
      Temper is built to be legible to AI coding agents. Two integration
      paths — a Claude Code skill file, or the remote MCP server.
    </p>

    <h3>Claude Code skill</h3>
    <p>
      A skill file teaches an agent your vault's structure, doc types, and
      workflow vocabulary. Temper generates one tailored to your vault:
    </p>
    <div class="cli-block">
      <pre><code>temper skill install</code></pre>
    </div>
    <table>
      <tbody>
        <tr><td><code>temper skill install</code></td><td>Install skill directory and command wrapper.</td></tr>
        <tr><td><code>temper skill generate</code></td><td>Preview the skill content to stdout without installing.</td></tr>
        <tr><td><code>temper skill check</code></td><td>Report installation status.</td></tr>
      </tbody>
    </table>
    <p>
      To automatically prime new Claude Code sessions with recent context,
      add a <code>SessionStart</code> hook:
    </p>
    <div class="cli-block">
      <pre><code>{`{
  "hooks": {
    "SessionStart": [{
      "hooks": [{
        "type": "command",
        "command": "temper warmup --context myapp"
      }]
    }]
  }
}`}</code></pre>
    </div>

    <h3>MCP server</h3>
    <p>
      The remote MCP server exposes vault operations as structured tools over
      Streamable HTTP. Agents authenticate via Auth0 using the OAuth 2.1 +
      PKCE flow. Connect Claude Desktop or Claude Code:
    </p>
    <div class="cli-block">
      <pre><code>{`{
  "mcpServers": {
    "temper": {
      "url": "https://temperkb.io/mcp"
    }
  }
}`}</code></pre>
    </div>
    <p>
      The client handles OAuth automatically — you'll be prompted to log in
      on first connection.
    </p>

    <h4>Available tools</h4>
    <table>
      <tbody>
        <tr><td><code>list_resources</code></td><td>List resources, filtered by context and/or doc type. Most recent first.</td></tr>
        <tr><td><code>get_resource</code></td><td>Get a resource by ID or slug, optionally with full markdown content.</td></tr>
        <tr><td><code>create_resource</code></td><td>Create a resource with optional markdown content. Name-based context and doc type.</td></tr>
        <tr><td><code>update_resource</code></td><td>Update a resource's title, slug, or content. New content triggers re-indexing.</td></tr>
        <tr><td><code>delete_resource</code></td><td>Soft-delete a resource by ID.</td></tr>
        <tr><td><code>search</code></td><td>Full-text and semantic search across the knowledge base.</td></tr>
        <tr><td><code>list_contexts</code></td><td>List available contexts (workspaces).</td></tr>
        <tr><td><code>get_context</code></td><td>Get details of a specific context.</td></tr>
        <tr><td><code>create_context</code></td><td>Create a new context (workspace).</td></tr>
        <tr><td><code>list_doc_types</code></td><td>List available document types.</td></tr>
        <tr><td><code>list_events</code></td><td>List events, optionally filtered by resource or type.</td></tr>
        <tr><td><code>get_profile</code></td><td>Get the authenticated user's profile.</td></tr>
      </tbody>
    </table>
  </section>

  <!-- ── Config ────────────────────────────────────────────────────── -->
  <section>
    <h2>Config</h2>
    <p>
      Global config lives at <code>~/.config/temper/config.toml</code>. A
      per-project <code>.temper/config.toml</code> overrides it when temper
      is run from that directory.
    </p>
    <table>
      <tbody>
        <tr><td><code>temper config edit</code></td><td>Open config.toml in <code>$EDITOR</code> — validate-then-save semantics.</td></tr>
        <tr><td><code>TEMPER_VAULT=&lt;path&gt;</code></td><td>Env var override for the vault path. Also accepted as <code>--vault &lt;path&gt;</code> on any command.</td></tr>
      </tbody>
    </table>
  </section>

  <a href="/" class="back-link">&larr; Back to home</a>
</div>

<style>
  .docs {
    max-width: 700px;
    margin: 0 auto;
    padding: 10rem 2.5rem 6rem;
  }

  h1 {
    font-family: var(--font-serif);
    font-size: 2.2rem;
    font-weight: 300;
    color: var(--parchment);
    margin-bottom: 0.75rem;
  }

  .lede {
    font-family: var(--font-serif);
    font-size: 1.05rem;
    color: var(--chalk);
    line-height: 1.8;
    margin-bottom: 3rem;
  }

  section {
    margin-bottom: 3rem;
    padding-left: 1.25rem;
    border-left: 2px solid var(--rule);
  }

  h2 {
    font-family: var(--font-mono);
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--temper-blue);
    letter-spacing: 0.12em;
    text-transform: uppercase;
    margin-bottom: 1.25rem;
  }

  h3 {
    font-family: var(--font-mono);
    font-size: 0.7rem;
    font-weight: 500;
    color: var(--graphite);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    margin-top: 1.75rem;
    margin-bottom: 0.75rem;
  }

  h4 {
    font-family: var(--font-mono);
    font-size: 0.65rem;
    font-weight: 500;
    color: var(--graphite);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    margin-top: 1.5rem;
    margin-bottom: 0.5rem;
  }

  p {
    font-family: var(--font-serif);
    font-size: 0.95rem;
    color: var(--chalk);
    line-height: 1.8;
    margin-bottom: 1rem;
  }
  p em {
    color: var(--temper-blue);
    font-style: italic;
  }

  a {
    color: var(--temper-blue);
    text-decoration: none;
    transition: color 0.2s;
  }

  a:hover {
    color: var(--parchment);
  }

  table {
    width: 100%;
    border-collapse: collapse;
    margin-bottom: 0.5rem;
  }

  td {
    font-family: var(--font-mono);
    font-size: 0.72rem;
    padding: 0.45rem 0;
    border-bottom: 1px solid var(--rule);
    vertical-align: top;
  }

  td:first-child {
    white-space: nowrap;
    padding-right: 1.5rem;
    color: var(--parchment);
  }

  td:last-child {
    color: var(--graphite);
    font-family: var(--font-serif);
    font-size: 0.82rem;
  }

  code {
    font-family: var(--font-mono);
    font-size: 0.72rem;
  }
  p code, td:last-child code {
    color: var(--parchment);
    background: rgba(255, 255, 255, 0.04);
    padding: 0.05rem 0.35rem;
    border-radius: 2px;
    font-size: 0.8em;
  }

  .cli-block {
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid var(--rule);
    border-radius: 4px;
    padding: 0.8rem 1rem;
    margin-bottom: 1rem;
    overflow-x: auto;
  }

  .cli-block pre {
    margin: 0;
  }

  .cli-block code {
    color: var(--parchment);
    font-size: 0.72rem;
    line-height: 1.7;
    background: none;
    padding: 0;
  }

  .back-link {
    display: inline-block;
    margin-top: 1rem;
    font-family: var(--font-mono);
    font-size: 0.75rem;
    color: var(--graphite);
    letter-spacing: 0.05em;
  }

  .back-link:hover {
    color: var(--temper-blue);
  }
</style>
