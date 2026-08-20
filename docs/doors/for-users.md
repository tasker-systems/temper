# Using Temper

For someone who wants Temper working alongside them and their agents — on their own machine,
against a deployment someone else runs (or their own).

If you are standing the deployment up yourself, start at
[Running Temper](./for-operators.md) and come back here.

## Start here

1. **[Install Temper](../playbooks/install-temper.md)** — the CLI and what the archive contains.
2. **[Authenticate](../playbooks/authenticate.md)** — sign in and get approved. This is a
   prerequisite for every command below; a brand-new signup is born *denied* and an admin must
   approve before any data call works.
3. **[Connect Claude Code](../playbooks/connect-claude-code.md)** — install the Temper skill
   so Claude Code can drive the CLI. Or **[Connect Claude Desktop](../playbooks/connect-claude-desktop.md)**
   — wire Temper in as an MCP server, if you use claude.ai or Claude Desktop instead.

## Working with a team

- **[Running a team](../playbooks/run-a-team.md)** — creating one, inviting people, roles, and
  what membership actually grants.
- **[Teams and roles](../concepts/teams-and-roles.md)** — what a team is, the role ladder, and
  why membership grants read but not write.

## Cognitive maps

A cognitive map is a telos-seeded region of the substrate where people and agents build shared
understanding. The conceptual walkthrough is at
[temperkb.io/cognitive-maps](https://temperkb.io/cognitive-maps); these are the hands-on pages.

- **[Building a cognitive map](../playbooks/build-a-cognitive-map.md)**
- **[Ingesting a corpus](../playbooks/ingest-a-corpus.md)** — getting a body of existing
  material into a context.
- **[Contexts and refs](../concepts/contexts-and-refs.md)** — what a context is and how to
  address one. This is the first thing that trips up new users.

## Keeping context across sessions

- **[Adopting operational memory](../playbooks/adopt-operational-memory.md)** — how durable
  memories are captured and recalled, and what makes one worth keeping.
- **[Operational memory](../concepts/operational-memory.md)** — the concept: what it is, the
  CLI memory model, and shared memory across a team.

## Every command

[The CLI reference](../reference/cli/README.md) lists every `temper` command and flag. It is
emitted from the built binary's `--help`, so it describes the CLI you actually have.
