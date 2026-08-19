# Using Temper

For someone who wants Temper working alongside them and their agents — on their own machine,
against a deployment someone else runs (or their own).

If you are standing the deployment up yourself, start at
[Running Temper](./for-operators.md) and come back here.

## Start here

1. **[Install Temper](../guides/install.md)** — the CLI, the binary, and what the archive
   actually contains.
2. **[Use it from Claude Desktop](../guides/claude-desktop-setup.md)** — wiring Temper in as an
   MCP server, so an agent can read and write the knowledge base directly.

That is enough to run `temper warmup`, save a session, and search across everything you have
written.

## Working with a team

- **[Working with teams](../guides/teams.md)** — creating one, inviting people, roles, and what
  membership actually grants.

## Cognitive maps

A cognitive map is a telos-seeded region of the substrate where people and agents build shared
understanding. The conceptual walkthrough is at
[temperkb.io/cognitive-maps](https://temperkb.io/cognitive-maps); these are the hands-on pages.

- **[Building a cognitive map from a large corpus](../guides/building-a-cognitive-map.md)**
- **[Ingesting a corpus into a context](../guides/corpus-ingestion.md)** — getting a body of
  existing material in.
- **[Bootstrapping a team's self-cognition map](../guides/team-self-cognition-bootstrap.md)**

## Keeping context across sessions

- **[Operational memory](../guides/operational-memory.md)** — how durable memories are captured
  and recalled, and what makes one worth keeping.

## Every command

[The CLI reference](../reference/cli/README.md) lists every `temper` command and flag. It is
emitted from the built binary's `--help`, so it describes the CLI you actually have.
