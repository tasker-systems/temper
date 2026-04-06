<svelte:head>
  <title>Getting Started — temper</title>
</svelte:head>

<div class="docs">
  <h1>Getting Started</h1>
  <p class="lede">
    Temper gives your work a throughline — the connective thread across sessions,
    decisions, and evolving understanding. Everything resolves to markdown.
  </p>

  <!-- ── Installation ──────────────────────────────────────────────── -->
  <section>
    <h2>Install</h2>
    <div class="cli-block">
      <pre><code>cargo install temper-cli</code></pre>
    </div>
    <p>
      Or build from source:
    </p>
    <div class="cli-block">
      <pre><code>git clone https://github.com/tasker-systems/temper.git
cd temper
cargo install --path crates/temper-cli</code></pre>
    </div>
  </section>

  <!-- ── CLI Commands ──────────────────────────────────────────────── -->
  <section>
    <h2>CLI Commands</h2>

    <h3>Core</h3>
    <table>
      <tbody>
        <tr><td><code>temper init</code></td><td>Initialize a new vault</td></tr>
        <tr><td><code>temper check</code></td><td>Verify vault integrity and tool health</td></tr>
        <tr><td><code>temper status</code></td><td>Vault overview</td></tr>
        <tr><td><code>temper warmup --context &lt;ctx&gt;</code></td><td>Context primer for new sessions</td></tr>
        <tr><td><code>temper doctor</code></td><td>Validate frontmatter and repair drift</td></tr>
        <tr><td><code>temper events</code></td><td>Show recent vault events</td></tr>
      </tbody>
    </table>

    <h3>Content</h3>
    <table>
      <tbody>
        <tr><td><code>temper add &lt;path&gt;</code></td><td>Add a file, URL, or directory to the vault</td></tr>
        <tr><td><code>temper session save [title]</code></td><td>Create or update a session note</td></tr>
        <tr><td><code>temper session list [--limit &lt;n&gt;]</code></td><td>List recent sessions</td></tr>
        <tr><td><code>temper session show &lt;slug&gt;</code></td><td>Show a session's content</td></tr>
        <tr><td><code>temper research save &lt;title&gt;</code></td><td>Create a research note</td></tr>
        <tr><td><code>temper note create &lt;type&gt; &lt;title&gt;</code></td><td>Create a note from template</td></tr>
      </tbody>
    </table>

    <h3>Search</h3>
    <table>
      <tbody>
        <tr><td><code>temper search &lt;query&gt;</code></td><td>Full-text and semantic search across the knowledge base</td></tr>
      </tbody>
    </table>

    <h3>Goals and Tasks</h3>
    <table>
      <tbody>
        <tr><td><code>temper goal create --title &lt;t&gt;</code></td><td>Create a goal</td></tr>
        <tr><td><code>temper goal list</code></td><td>List goals with task stage counts</td></tr>
        <tr><td><code>temper task create --title &lt;t&gt;</code></td><td>Create a task</td></tr>
        <tr><td><code>temper task list [--stage &lt;s&gt;]</code></td><td>List tasks, optionally filtered by stage</td></tr>
        <tr><td><code>temper task show &lt;slug&gt;</code></td><td>Show a task (accepts slug, suffix, or seq number)</td></tr>
        <tr><td><code>temper task move &lt;slug&gt; --stage &lt;s&gt;</code></td><td>Move a task to a new stage</td></tr>
        <tr><td><code>temper task done &lt;slug&gt;</code></td><td>Mark a task as done</td></tr>
      </tbody>
    </table>

    <h3>Contexts and Skills</h3>
    <table>
      <tbody>
        <tr><td><code>temper context add &lt;name&gt;</code></td><td>Add a context</td></tr>
        <tr><td><code>temper context list</code></td><td>List contexts</td></tr>
        <tr><td><code>temper skill generate</code></td><td>Generate CLI reference from current commands</td></tr>
        <tr><td><code>temper skill install</code></td><td>Install skill files and command wrapper</td></tr>
        <tr><td><code>temper skill check</code></td><td>Check skill installation status</td></tr>
      </tbody>
    </table>

    <h3>Cloud</h3>
    <table>
      <tbody>
        <tr><td><code>temper auth login</code></td><td>Authenticate with temper cloud</td></tr>
        <tr><td><code>temper sync run</code></td><td>Sync local vault with temper cloud</td></tr>
        <tr><td><code>temper sync status</code></td><td>Show sync status without changes</td></tr>
        <tr><td><code>temper pull &lt;resource&gt;</code></td><td>Pull a resource from the cloud</td></tr>
        <tr><td><code>temper remove &lt;resource&gt;</code></td><td>Remove a resource from the cloud</td></tr>
      </tbody>
    </table>
  </section>

  <!-- ── MCP Server ────────────────────────────────────────────────── -->
  <section>
    <h2>MCP Server</h2>
    <p>
      The remote MCP server exposes vault operations as structured tools over
      Streamable HTTP. Agents authenticate via Auth0 using the standard
      OAuth 2.1 + PKCE flow.
    </p>

    <h3>Available Tools</h3>
    <table>
      <tbody>
        <tr><td><code>list_resources</code></td><td>List resources, optionally filtered by context</td></tr>
        <tr><td><code>get_resource</code></td><td>Get a resource by ID, optionally with full content</td></tr>
        <tr><td><code>get_resource_content</code></td><td>Get the full markdown content of a resource</td></tr>
        <tr><td><code>create_resource</code></td><td>Create a new resource in a context</td></tr>
        <tr><td><code>update_resource</code></td><td>Update a resource's title, slug, or mimetype</td></tr>
        <tr><td><code>delete_resource</code></td><td>Soft-delete a resource by ID</td></tr>
        <tr><td><code>search</code></td><td>Full-text and semantic search across the knowledge base</td></tr>
        <tr><td><code>list_contexts</code></td><td>List available contexts (workspaces)</td></tr>
        <tr><td><code>get_context</code></td><td>Get details of a specific context</td></tr>
        <tr><td><code>create_context</code></td><td>Create a new context (workspace)</td></tr>
        <tr><td><code>list_events</code></td><td>List events, optionally filtered by resource or type</td></tr>
        <tr><td><code>get_profile</code></td><td>Get the authenticated user's profile</td></tr>
      </tbody>
    </table>

    <h3>Connect</h3>
    <p>Add to your Claude Desktop or Claude Code MCP configuration:</p>
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
  </section>

  <!-- ── Session Pre-Warming ───────────────────────────────────────── -->
  <section>
    <h2>Claude Code Integration</h2>
    <p>
      Generate a Claude Code skill file tailored to your vault:
    </p>
    <div class="cli-block">
      <pre><code>temper skill install</code></pre>
    </div>
    <p>
      To automatically prime new sessions with recent context, add a
      <code>SessionStart</code> hook:
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
    font-family: var(--serif);
    font-size: 2.2rem;
    font-weight: 300;
    color: var(--text);
    margin-bottom: 0.75rem;
  }

  .lede {
    font-family: var(--serif);
    font-size: 1.05rem;
    color: var(--text-mid);
    line-height: 1.8;
    margin-bottom: 3rem;
  }

  section {
    margin-bottom: 3rem;
    padding-left: 1.25rem;
    border-left: 2px solid var(--rule);
  }

  h2 {
    font-family: var(--mono);
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--blue);
    letter-spacing: 0.12em;
    text-transform: uppercase;
    margin-bottom: 1.25rem;
  }

  h3 {
    font-family: var(--mono);
    font-size: 0.7rem;
    font-weight: 500;
    color: var(--text-dim);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    margin-top: 1.5rem;
    margin-bottom: 0.75rem;
  }

  p {
    font-family: var(--serif);
    font-size: 0.95rem;
    color: var(--text-mid);
    line-height: 1.8;
    margin-bottom: 1rem;
  }

  a {
    color: var(--blue);
    text-decoration: none;
    transition: color 0.2s;
  }

  a:hover {
    color: var(--text);
  }

  table {
    width: 100%;
    border-collapse: collapse;
    margin-bottom: 0.5rem;
  }

  td {
    font-family: var(--mono);
    font-size: 0.72rem;
    padding: 0.45rem 0;
    border-bottom: 1px solid var(--rule);
    vertical-align: top;
  }

  td:first-child {
    white-space: nowrap;
    padding-right: 1.5rem;
    color: var(--text);
  }

  td:last-child {
    color: var(--text-dim);
    font-family: var(--serif);
    font-size: 0.82rem;
  }

  code {
    font-family: var(--mono);
    font-size: 0.72rem;
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
    color: var(--text);
    font-size: 0.72rem;
    line-height: 1.7;
  }

  .back-link {
    display: inline-block;
    margin-top: 1rem;
    font-family: var(--mono);
    font-size: 0.75rem;
    color: var(--text-dim);
    letter-spacing: 0.05em;
  }

  .back-link:hover {
    color: var(--blue);
  }
</style>
