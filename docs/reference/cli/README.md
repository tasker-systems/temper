<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# CLI reference

Every `temper` command, emitted from the built binary's own `--help`. If a page
here disagrees with the binary in your hands, the page is a defect — nothing in
this tree is hand-written.

```text
Developer workflow tool for agent-assisted development

Usage: temper [OPTIONS] <COMMAND>

Commands:
  init           Initialize a new vault
  check          Check vault integrity and tool health
  status         Show vault status overview
  resource       Manage resources (tasks, goals, sessions, research, concepts, decisions)
  data-artifact  List and show data artifacts owned by a resource
  context        Manage contexts (projects)
  warmup         Context primer for new sessions — active goals, in-progress tasks, recent session pointers
  invitations    List the pending team invitations addressed to you
  skill          Manage agent skill (install for Claude Code or opencode)
  memory         Manage the Claude Code memory projection
  auth           Authenticate with temper cloud
  slack          Manage the Slack account link
  team           Manage team membership and access
  admin          Administer the instance (system settings, promote admins, review requests)
  pull           Materialize a context's resources into the local read-only projection
  config         Manage temper global config
  query          Run a composed query — declared acts, piped, answered in one round trip
  search         Search the knowledge base
  edge           Assert or mutate a relationship between resources (writes go through the cloud API)
  cogmap         Operate on cognitive maps (admin-gated content reconcile)
  invocation     Operate on agent-invocation envelopes (open / close / show / list)
  steward        Team-self-cognition steward ingest trigger (delta / advance-watermark)
  trail          Read the event trail (append-only history) of a graph element — a resource node or a relationship edge
  version        Print the CLI version, optionally with the running binary's SHA-256 or an offline (or online) manifest verdict
  update         Self-update the CLI to the latest release (curl-script installs only)
  help           Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
  -V, --version            Print version
```

## Commands

| Command | Summary |
| --- | --- |
| [`temper init`](./init.md) | Initialize a new vault |
| [`temper check`](./check.md) | Check vault integrity and tool health |
| [`temper status`](./status.md) | Show vault status overview |
| [`temper resource`](./resource.md) | Manage resources (tasks, goals, sessions, research, concepts, decisions) |
| [`temper data-artifact`](./data-artifact.md) | List and show data artifacts owned by a resource |
| [`temper context`](./context.md) | Manage contexts (projects) |
| [`temper warmup`](./warmup.md) | Context primer for new sessions — active goals, in-progress tasks, recent session pointers |
| [`temper invitations`](./invitations.md) | List the pending team invitations addressed to you |
| [`temper skill`](./skill.md) | Manage agent skill (install for Claude Code or opencode) |
| [`temper memory`](./memory.md) | Manage the Claude Code memory projection |
| [`temper auth`](./auth.md) | Authenticate with temper cloud |
| [`temper slack`](./slack.md) | Manage the Slack account link |
| [`temper team`](./team.md) | Manage team membership and access |
| [`temper admin`](./admin.md) | Administer the instance (system settings, promote admins, review requests) |
| [`temper pull`](./pull.md) | Materialize a context's resources into the local read-only projection |
| [`temper config`](./config.md) | Manage temper global config |
| [`temper query`](./query.md) | Run a composed query — declared acts, piped, answered in one round trip |
| [`temper search`](./search.md) | Search the knowledge base |
| [`temper edge`](./edge.md) | Assert or mutate a relationship between resources (writes go through the cloud API) |
| [`temper cogmap`](./cogmap.md) | Operate on cognitive maps (admin-gated content reconcile) |
| [`temper invocation`](./invocation.md) | Operate on agent-invocation envelopes (open / close / show / list) |
| [`temper steward`](./steward.md) | Team-self-cognition steward ingest trigger (delta / advance-watermark) |
| [`temper trail`](./trail.md) | Read the event trail (append-only history) of a graph element — a resource node or a relationship edge. |
| [`temper version`](./version.md) | Print the CLI version, optionally with the running binary's SHA-256 or an offline (or online) manifest verdict. |
| [`temper update`](./update.md) | Self-update the CLI to the latest release (curl-script installs only). |
