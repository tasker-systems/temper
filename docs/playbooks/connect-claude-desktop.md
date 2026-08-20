# Connect Temper to Claude Desktop

**For individual users** — someone wiring Temper into Claude Desktop or claude.ai
as an MCP server, on their own machine, against a deployment someone else runs
(or their own).

After this playbook you will have Claude Desktop (or claude.ai) connected to your
Temper knowledge base as an MCP server, and the Temper skill uploaded so Claude
knows how to use it well. Claude will be able to search your
[cognitive map](https://temperkb.io/cognitive-maps/what-a-cognitive-map-is),
attach documents from it into a conversation, and create new resources — all
through the connector, with no local binary to install or run.

## Prerequisites

- **A Temper deployment you can reach.** The hosted instance serves its MCP
  connector at `https://temperkb.io/mcp`. If you are self-hosting, use your
  instance's `/mcp` path instead.
- **An approved account on that deployment.** Authentication is OAuth — Claude
  will prompt you on first connect. A brand-new signup is born denied and needs
  an admin's approval before any data call succeeds. See
  [the trust boundary](../concepts/trust-boundary.md) for what the server
  enforces.
- **Claude Desktop or a claude.ai account.**
- **The Temper skill zip.** Download `temper-skill-v<version>.zip` from the
  [latest release](https://github.com/tasker-systems/temper/releases/latest).
  You will upload this in step 2.

> **You do not need to install the Temper CLI for this path.** Claude Desktop
> and claude.ai drive MCP over HTTP — they talk to the connector, not to a
> local binary. You authenticate through the connector's own OAuth prompt, never
> through `temper init` or `temper auth login`. The CLI is a separate surface;
> see [Claude Code vs Claude Desktop](#claude-code-vs-claude-desktop) below.

---

## 1. Connect the MCP server

In Claude Desktop: **Settings → Connectors → Add custom connector**, with the
URL:

```
https://temperkb.io/mcp
```

Authentication is OAuth via Auth0 and happens on first connect — Claude will
prompt you.

> **Do not use `claude_desktop_config.json` for this.** That file configures
> *local, stdio* MCP servers. Claude Desktop will not connect to a remote
> server declared there, so a `"url"` entry in `mcpServers` silently does
> nothing — it looks configured and never connects. Remote connectors go
> through the Connectors UI, not the config file.

Once connected you get:

| Surface | What it is |
|---|---|
| **Resources panel** (attachment icon) | Browse and attach documents directly. Content is injected before the model runs — no tool call, no tool-call tokens |
| **Tools** | `search`, `list_resources`, `create_resource`, and the rest, called during conversation |

Resource URIs follow the form `temper://resources/{id}`,
`temper://resources/{id}/content`, and `temper://contexts/{ref}/resources` —
where `{ref}` is `@owner/slug` or a UUID. Bare context names are not accepted.
See [contexts and refs](../concepts/contexts-and-refs.md) for the full ref
grammar.

## 2. Install the skill

The connector gives Claude *access*. The skill gives it the working discipline:
which doc types exist, how goals and tasks are structured, the session-note
shape, and the traps (a capped list read as a complete one; an abbreviated UUID
that resolves to nothing).

1. Download `temper-skill-v<version>.zip` from the
   [latest release](https://github.com/tasker-systems/temper/releases/latest)
   (if you have not already).
2. In Claude Desktop: **Customize → Skills → +**, and upload the zip.

The bundle carries a `VERSION` file at its root — that is how you tell which one
you uploaded, since the Skills list shows only the skill name.

### Verifying what you downloaded

Each release publishes a `.sha256` sidecar beside the zip, and the zip carries a
GitHub build-provenance attestation:

```bash
shasum -a 256 -c temper-skill-v<version>.zip.sha256
gh attestation verify temper-skill-v<version>.zip --repo tasker-systems/temper
```

---

## Claude Code vs Claude Desktop

The path above is for **Claude Desktop and claude.ai** — surfaces that cannot
run a binary, so they drive MCP over HTTP and want the connector URL plus the
uploaded skill zip. That reader never runs `temper init` or
`temper auth login`; they authenticate through the connector's own OAuth prompt.

**Claude Code** is a different surface: it drives the Temper CLI directly. A
Claude Code user [installs Temper](../playbooks/install-temper.md), runs
`temper init` and `temper auth login` to authenticate, then runs
`temper skill install` to teach Claude Code the working discipline. That path
speaks `temper …` commands rather than MCP tool calls. See
[using Temper from the CLI](https://temperkb.io/using-temper) for that route.

### Skills do not sync between surfaces

A skill uploaded to Claude Desktop / claude.ai is **not** available in the API
or in Claude Code, and vice versa. Each surface needs its own upload. Claude
Code users want the CLI packaging instead (`temper skill install`), which is a
different tree — it speaks `temper …` commands rather than tool calls.
