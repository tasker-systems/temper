# Using temper from Claude Desktop (and claude.ai)

`[observed — 2026-08-01]`

Two independent things, in this order. Neither implies the other, and the common failure is doing
only the second and wondering why Claude cannot see anything.

1. **Connect the MCP server** — gives Claude access to your knowledge base.
2. **Install the skill** — teaches Claude how to *use* it well.

This guide lives here, not inside the skill bundle, deliberately: it is what you read *before* you
have the skill, and it is addressed to you rather than to the agent. A copy of it shipped inside the
bundle for a while, where it was both duplicated and read by an agent as if it were instructions.

---

## 1. Connect the MCP server

**Settings → Connectors → Add custom connector**, with the URL:

```
https://temperkb.io/mcp
```

Authentication is OAuth via Auth0 and happens on first connect — Claude will prompt you.

> **Do not use `claude_desktop_config.json` for this.** That file configures *local, stdio* MCP
> servers. **Claude Desktop will not connect to a remote server declared there**, so a `"url"` entry
> in `mcpServers` silently does nothing — it looks configured and never connects. This guide
> previously said to do exactly that; it was wrong.

Once connected you get:

| Surface | What it is |
|---|---|
| **Resources panel** (attachment icon) | Browse and attach documents directly. Content is injected before the model runs — no tool call, no tool-call tokens |
| **Tools** | `search`, `list_resources`, `create_resource`, and the rest, called during conversation |

Resource URIs are `temper://resources/{id}`, `temper://resources/{id}/content`, and
`temper://contexts/{ref}/resources` — where `{ref}` is `@owner/slug` or a UUID. **Bare context names
are not accepted.**

## 2. Install the skill

The connector gives Claude *access*. The skill gives it the working discipline: which doc types
exist, how goals and tasks are structured, the session-note shape, and the traps (a capped list read
as a complete one; an abbreviated UUID that resolves to nothing).

1. Download `temper-skill-v<version>.zip` from the
   [latest release](https://github.com/tasker-systems/temper/releases/latest).
2. **Customize → Skills → +**, and upload the zip.

The bundle carries a `VERSION` file at its root — that is how you tell which one you uploaded, since
the Skills list shows only the skill name.

### Verifying what you downloaded

Each release publishes a `.sha256` sidecar beside the zip, and the zip carries a GitHub build-
provenance attestation:

```bash
shasum -a 256 -c temper-skill-v<version>.zip.sha256
gh attestation verify temper-skill-v<version>.zip --repo tasker-systems/temper
```

The bundle deliberately carries **no** per-file manifest, unlike the CLI archives. Manifests exist so
`install.sh` can verify each extracted file before an atomic swap; nothing installs this bundle, so
there is nothing for one to gate. See
[the release guide](https://github.com/tasker-systems/temper/blob/main/internal/development/releasing.md) for the CLI side.

### Skills do not sync between surfaces

A skill uploaded to Claude Desktop / claude.ai is **not** available in the API or in Claude Code, and
vice versa. Each surface needs its own upload. Claude Code users want the CLI packaging instead
(`temper skill install`), which is a different tree — it speaks `temper …` commands rather than tool
calls.

---

## Where things live

| | |
|---|---|
| Skill bundle source | `agent-skills/temper-knowledge-base/` (a committed projection — see the `generated-artifacts` skill) |
| Build it locally | `cargo make skill-package` → `dist/temper-skill-v<version>.zip` |
| Tool reference | `knowledge-base.md` inside the bundle |
| Doc types + frontmatter | `references/frontmatter.md` inside the bundle (generated from the server's own schemas) |
