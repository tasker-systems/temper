<script lang="ts">
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();
</script>

<svelte:head>
  <title>Reference — temper</title>
</svelte:head>

<div class="docs">
  <h1>Reference</h1>
  <p class="lede">
    The operational reference for the temper CLI, cloud sync, and MCP server.
    For the conceptual frame — what Temper is, and why — start with
    <a href="/cognitive-maps">cognitive maps</a>, or read the commitments in
    <a href="/theory">theory</a>.
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
      Supported platforms: macOS (Apple Silicon), Linux (x86_64), and Windows
      (x86_64). File issues at
      <a href="https://github.com/tasker-systems/temper/issues">github.com/tasker-systems/temper/issues</a>
      if you hit problems.
    </p>
    <p>
      To pin a version, pass <code>--version vX.Y.Z</code> to the install
      script (<code>… | sh -s -- --version vX.Y.Z</code>). The
      <a href="https://github.com/tasker-systems/temper/blob/main/docs/playbooks/install-temper.md">install playbook</a>
      covers Homebrew, uninstalling, other platforms, and troubleshooting.
    </p>

    <h3>Build from source</h3>
    <p>
      If you'd rather compile locally (Intel Mac, Linux arm64, or just
      to hack on temper itself):
    </p>
    <div class="cli-block">
      <pre><code>git clone https://github.com/tasker-systems/temper.git
cd temper
cargo install --path crates/temper-cli --locked --features embed,extract</code></pre>
    </div>
    <p>
      The <code>embed</code> feature pulls in ONNX Runtime for local
      embeddings; <code>extract</code> enables document ingestion via
      kreuzberg. Both are enabled by default.
    </p>
  </section>

  <!-- ── Getting started ──────────────────────────────────────────── -->
  <section>
    <h2>Getting started</h2>
    <p>
      The local vault is a directory of markdown files with YAML frontmatter — a
      read-only <em>projection</em> of cloud state, not the source of truth (see
      <a href="#cloud">Cloud</a>). Temper resolves its config from
      <code>~/.config/temper/config.toml</code> or a per-project
      <code>.temper/config.toml</code>.
    </p>
    <table>
      <tbody>
        <tr><td><code>temper init</code></td><td>Initialise a new vault — walks vault location, contexts to create, and instance setup.</td></tr>
        <tr><td><code>temper context create &lt;name&gt;</code></td><td>Create a context (project) on the server. The output carries its ref (<code>@you/&lt;slug&gt;</code>) — use that ref everywhere else.</td></tr>
        <tr><td><code>temper warmup --context &lt;ctx-ref&gt;</code></td><td>Session primer — recent work, open tasks, recent decisions. Pipe into an agent's first prompt. The context is a ref (<code>@me/myapp</code>) — bare names are not addressable.</td></tr>
        <tr><td><code>temper pull &lt;ctx&gt;</code></td><td>Re-materialise a context's projection from the cloud into the local vault.</td></tr>
        <tr><td><code>temper status</code></td><td>Vault overview: contexts, resource counts, recent activity.</td></tr>
        <tr><td><code>temper check</code></td><td>Verify vault integrity and tool health.</td></tr>
      </tbody>
    </table>
    <p>
      To add content, create a resource and pipe in a body —
      <code>temper resource create --type research --title "…" --body @notes.md</code>.
      See <a href="#resources">Resources</a>.
    </p>
  </section>

  <!-- ── Resources ─────────────────────────────────────────────────── -->
  <section id="resources">
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

    <p>
      Resources are addressed by <strong>ref</strong> — a UUID, or the decorated
      <code>slug-&lt;uuid&gt;</code> form (resolved by the trailing UUID; the slug
      half is presentation, so a stale slug is harmless). Every
      <code>list</code> / <code>show</code> / <code>search</code> row prints a
      <code>ref</code> field — copy it, paste it.
    </p>
    <h3>Create · list · show · update · delete</h3>
    <table>
      <tbody>
        <tr><td><code>temper resource create --type &lt;t&gt; --title &lt;title&gt;</code></td><td>Create a resource. Add <code>--body @file.md</code> (or pipe markdown via stdin) for content; <code>--context</code>, <code>--goal</code>, <code>--mode</code>, <code>--effort</code> as needed.</td></tr>
        <tr><td><code>temper resource list --type &lt;t&gt;</code></td><td>List resources of a type (<code>--type</code> is required). Filters: <code>--context</code>, <code>--stage</code>, <code>--goal</code>, <code>--limit</code>; <code>--with open-meta</code> / <code>--fields</code> for cheaper reads.</td></tr>
        <tr><td><code>temper resource show &lt;ref&gt;</code></td><td>Show a resource by ref. Add <code>--edges</code> for its graph edges, or <code>--without body</code> for frontmatter without the body.</td></tr>
        <tr><td><code>temper resource update &lt;ref&gt; --title &lt;t&gt;</code></td><td>Update the title. (Body: <code>--body @file.md</code> or pipe markdown via stdin.)</td></tr>
        <tr><td><code>temper resource update &lt;ref&gt; --context-to &lt;ctx&gt;</code></td><td>Move the resource to a different context.</td></tr>
        <tr><td><code>temper resource update &lt;ref&gt; --stage &lt;s&gt;</code></td><td>Task stage: <code>backlog</code>, <code>in-progress</code>, <code>done</code>, <code>cancelled</code>.</td></tr>
        <tr><td><code>temper resource update &lt;ref&gt; --mode &lt;m&gt;</code></td><td>Task mode: <code>plan</code>, <code>build</code>.</td></tr>
        <tr><td><code>temper resource update &lt;ref&gt; --effort &lt;e&gt;</code></td><td>Task effort: <code>small</code>, <code>medium</code>, <code>large</code>.</td></tr>
        <tr><td><code>temper resource update &lt;ref&gt; --relates-to &lt;ref&gt;</code></td><td>Add a relationship. Repeatable. Similar: <code>--references</code>, <code>--depends-on</code>, <code>--extends</code>, <code>--preceded-by</code>, <code>--derived-from</code>.</td></tr>
        <tr><td><code>temper resource update &lt;ref&gt; --branch &lt;name&gt; --pr &lt;url&gt;</code></td><td>Task metadata: attach a git branch or PR URL.</td></tr>
        <tr><td><code>temper resource delete &lt;ref&gt;</code></td><td>Soft-delete on the server (the authoritative action; the row is preserved server-side). <code>--force</code> is accepted but vestigial — deletion is non-interactive.</td></tr>
      </tbody>
    </table>
  </section>

  <!-- ── Search ────────────────────────────────────────────────────── -->
  <section>
    <h2>Search</h2>
    <p>
      <code>temper search</code> combines full-text and semantic search — your
      query is embedded locally and matched by meaning, not just keywords.
    </p>
    <table>
      <tbody>
        <tr><td><code>temper search &lt;query&gt;</code></td><td>Hybrid search across the vault.</td></tr>
        <tr><td><code>--context &lt;ctx-ref&gt;</code></td><td>Scope to one context.</td></tr>
        <tr><td><code>--cogmap &lt;ref&gt;</code></td><td>Scope to a single cognitive map (mutually exclusive with <code>--context</code>).</td></tr>
        <tr><td><code>--doc-type &lt;type&gt;</code></td><td>Filter by doc type.</td></tr>
        <tr><td><code>--limit &lt;n&gt;</code></td><td>Cap results (default 10).</td></tr>
        <tr><td><code>--offset &lt;n&gt;</code></td><td>Skip this many results.</td></tr>
        <tr><td><code>--within &lt;ref&gt;</code></td><td>Narrow to specific resources, by ref. Repeatable; composes with <code>--context</code> / <code>--cogmap</code>.</td></tr>
        <tr><td><code>--text-only</code></td><td>Skip semantic search (no local embedding needed).</td></tr>
      </tbody>
    </table>
  </section>

  <!-- ── Relationships ─────────────────────────────────────────────── -->
  <section>
    <h2>Relationships</h2>
    <p>
      Resources aren't isolated — typed edges connect them, and search can
      expand along those edges to surface neighbors. Edges are first-class:
      asserted explicitly, then re-typed, re-weighted, or folded as the
      understanding changes. Source and target are <strong>refs</strong>; an
      edge is addressed by its own handle (a UUID).
    </p>
    <table>
      <tbody>
        <tr><td><code>temper edge assert &lt;source&gt; &lt;target&gt; --kind &lt;k&gt; --polarity &lt;p&gt; --label &lt;text&gt;</code></td><td>Assert a typed edge. Kinds: <code>express</code>, <code>contains</code>, <code>leads-to</code>, <code>near</code>. Polarity: <code>forward</code> or <code>inverse</code>. Optional <code>--weight</code> (default 1.0).</td></tr>
        <tr><td><code>temper edge retype &lt;edge-handle&gt; --kind &lt;k&gt; --polarity &lt;p&gt;</code></td><td>Change an edge's kind and polarity.</td></tr>
        <tr><td><code>temper edge reweight &lt;edge-handle&gt; --weight &lt;n&gt;</code></td><td>Change an edge's weight.</td></tr>
        <tr><td><code>temper edge fold &lt;edge-handle&gt; [--reason &lt;text&gt;]</code></td><td>Fold (supersede) an edge — it stops contributing to projections, with the reason recorded.</td></tr>
      </tbody>
    </table>
  </section>

  <!-- ── Cloud ─────────────────────────────────────────────────────── -->
  <section id="cloud">
    <h2>Cloud</h2>
    <p>
      Temper Cloud is the Postgres-native <strong>source of truth</strong>, with
      pgvector-powered semantic search. The local vault is the inverse of the
      old model: a <strong>read-only projection cache</strong>, not the canonical
      copy. Writes (<code>resource create</code>/<code>update</code>/<code>delete</code>,
      <code>edge</code>) route straight to the API and take effect immediately;
      <code>temper pull &lt;ctx&gt;</code> re-materialises the local markdown from
      server state. Removing a projected file with <code>rm</code> is just a local
      cache miss — it has no server effect.
    </p>

    <h3>Auth</h3>
    <table>
      <tbody>
        <tr><td><code>temper auth login</code></td><td>Browser-based OAuth with PKCE. Caches the token locally.</td></tr>
        <tr><td><code>temper auth status</code></td><td>Show current auth state.</td></tr>
        <tr><td><code>temper auth logout</code></td><td>Clear cached credentials.</td></tr>
        <tr><td><code>temper auth token</code></td><td>Store a JWT directly (for API-only clients or manual auth).</td></tr>
        <tr><td><code>temper auth export-token</code></td><td>Print the cached JWT to stdout — pipe into a cloud session's secret manager as <code>TEMPER_TOKEN</code>.</td></tr>
      </tbody>
    </table>

    <h3>Teams</h3>
    <table>
      <tbody>
        <tr><td><code>temper team create &lt;slug&gt;</code></td><td>Create a team — you become its owner. <code>--name</code> sets the display name; <code>--parent &lt;ref&gt;</code> nests it.</td></tr>
        <tr><td><code>temper team list</code></td><td>List the teams you are a member of.</td></tr>
        <tr><td><code>temper team show &lt;slug&gt;</code></td><td>Show a team's detail and member roster.</td></tr>
        <tr><td><code>temper team invite &lt;team&gt; &lt;email&gt; --role &lt;role&gt;</code></td><td>Invite an email (owner/maintainer). Prints the invitation with its token; no email is sent — the address is a correlator.</td></tr>
        <tr><td><code>temper team join &lt;token&gt;</code></td><td>Accept a team invitation by its token.</td></tr>
        <tr><td><code>temper team add-member &lt;team-id&gt; &lt;profile-id&gt; --role &lt;role&gt;</code></td><td>Add an existing profile directly (owner/maintainer). Takes UUIDs, not slugs.</td></tr>
        <tr><td><code>temper team set-role &lt;team&gt; &lt;profile-id&gt; --role &lt;role&gt;</code></td><td>Change a member's role (owner/maintainer).</td></tr>
        <tr><td><code>temper team remove-member &lt;team&gt; &lt;profile-id&gt;</code></td><td>Remove a member (owner/maintainer).</td></tr>
        <tr><td><code>temper team leave &lt;team&gt;</code></td><td>Leave a team you are a member of.</td></tr>
        <tr><td><code>temper team reassign &lt;team&gt; --from &lt;uuid&gt; --to &lt;uuid&gt;</code></td><td>Bulk-reassign a departing member's team-scoped resources (offboarding).</td></tr>
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

    <h3>Skill file</h3>
    <p>
      A generated skill file teaches an agent your vault's structure, doc
      types, and workflow vocabulary — for Claude Code, opencode, or any
      <code>~/.agents</code>-compatible tool:
    </p>
    <div class="cli-block">
      <pre><code>temper skill install</code></pre>
    </div>
    <table>
      <tbody>
        <tr><td><code>temper skill install</code></td><td>Install the skill (<code>--target agents|claude|opencode</code>, default <code>agents</code>; <code>--path &lt;dir&gt;</code> installs elsewhere).</td></tr>
        <tr><td><code>temper skill generate</code></td><td>Preview the skill content to stdout without installing.</td></tr>
        <tr><td><code>temper skill check</code></td><td>Report installation status.</td></tr>
      </tbody>
    </table>
    <p>
      To automatically prime new agent sessions with recent context,
      add a <code>SessionStart</code> hook:
    </p>
    <div class="cli-block">
      <pre><code>{`{
  "hooks": {
    "SessionStart": [{
      "hooks": [{
        "type": "command",
        "command": "temper warmup --context @me/myapp"
      }]
    }]
  }
}`}</code></pre>
    </div>

    <h3>MCP server</h3>
    <p>
      The remote MCP server exposes vault operations as structured tools over
      Streamable HTTP. Agents authenticate via OAuth 2.1 + PKCE — you'll be
      prompted to log in on first connection. Connect Claude Desktop or
      Claude Code:
    </p>
    <div class="cli-block">
      <pre><code>{`{
  "mcpServers": {
    "temper": {
      "url": "${data.appUrl}/mcp"
    }
  }
}`}</code></pre>
    </div>

    <h4>Core tools</h4>
    <table>
      <tbody>
        <tr><td><code>search</code></td><td>Full-text and semantic search across the knowledge base.</td></tr>
        <tr><td><code>list_resources</code></td><td>List resources, filtered by context and/or doc type. Most recent first.</td></tr>
        <tr><td><code>get_resource</code></td><td>Get a resource by ref, optionally with full markdown content.</td></tr>
        <tr><td><code>create_resource</code></td><td>Create a resource with optional markdown content. Name-based context and doc type.</td></tr>
        <tr><td><code>update_resource</code></td><td>Update a resource's title, slug, or content. New content triggers re-indexing.</td></tr>
        <tr><td><code>update_resource_meta</code></td><td>Update a resource's managed/open frontmatter without touching the body.</td></tr>
        <tr><td><code>delete_resource</code></td><td>Soft-delete a resource by ID.</td></tr>
        <tr><td><code>relationship</code></td><td>Assert, retype, reweight, or fold typed edges between resources (action discriminator).</td></tr>
        <tr><td><code>context_read</code> / <code>context_manage</code></td><td>Read and manage contexts (workspaces).</td></tr>
        <tr><td><code>describe_schema</code></td><td>Describe doc types and their schemas.</td></tr>
        <tr><td><code>run_query</code></td><td>Run a composed query — a declared DAG of acts answered in one round trip.</td></tr>
        <tr><td><code>element_trail</code></td><td>Read a resource's or edge's append-only event trail.</td></tr>
      </tbody>
    </table>
    <p>
      The tool surface also covers cognitive maps (<code>cogmap_*</code>),
      blobs (<code>blob_*</code>), data artifacts
      (<code>commit_data_artifact</code> and friends), and facets
      (<code>facet_set</code> / <code>facets_read</code>) — the full list is
      what the server advertises over MCP, so your client's tool list is the
      reference.
    </p>
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
