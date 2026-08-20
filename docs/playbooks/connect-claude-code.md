# Connect Claude Code

**For individual users** — someone who wants to drive Temper from Claude Code (Anthropic's CLI
agent). This is the only Claude Code path, and it is distinct from the Claude Desktop path.

## Outcome

By the end of this page, Claude Code will have the Temper skill installed and will be able to
run `/temper` commands to manage your knowledge base.

## Prerequisites

- **Temper installed** — see [Install Temper](./install-temper.md).
- **Temper initialized** — run `temper init` first. The skill installer needs a config file on
  disk.
- **Authentication is NOT required for this step.** The skill installer makes no network call
  and builds no client. You will need to authenticate separately (see
  [Authenticate](./authenticate.md)) before running Temper commands through the skill.

## Install the skill

```bash
temper skill install --target claude
```

This writes approximately 20 files to `~/.claude/skills/temper/` plus
`~/.claude/commands/temper.md`. It installs the Temper skill that teaches Claude Code the
session lifecycle, grounding, and outcome registers.

> **`temper init` is required, authentication is not.** The skill installer needs a config file
> to know which instance to target, but it does not authenticate or make any network call.
> You can install the skill before or after authenticating — the skill itself is inert until
> you run a `/temper` command through Claude Code.

## The two Claude paths — and why they are different

**Claude Code** drives the CLI. It wants `temper skill install --target claude`. Claude Code
can run a binary, so it shells out to `temper` directly. Authenticate with
`temper auth login` and Claude Code inherits your cached token.

**Claude Desktop / claude.ai** cannot run a binary. It drives MCP over HTTP and wants the
connector URL plus the uploaded skill zip. That reader never runs `temper init` or
`temper auth login` — they authenticate through the connector's own OAuth prompt. See
[Connect Claude Desktop](./connect-claude-desktop.md) for that path.

## Further reading

- **Authenticate after installing the skill:** [Authenticate](./authenticate.md).
- **The trust boundary your CLI session crosses:**
  [The Trust Boundary](../concepts/trust-boundary.md).
- **Using Temper from the CLI:** [temperkb.io/using-temper](https://temperkb.io/using-temper).
