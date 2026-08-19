<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper skill`

Manage agent skill (install for Claude Code or opencode)

```text
Manage agent skill (install for Claude Code or opencode)

Usage: temper skill [OPTIONS] <COMMAND>

Commands:
  generate  Generate skill content (preview to stdout)
  install   Install skill directory and command wrapper
  check     Check skill status
  emit      Emit the MCP (`agent-skills/`) projection into a directory
  help      Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper skill generate`

```text
Generate skill content (preview to stdout)

Usage: temper skill generate [OPTIONS]

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper skill install`

```text
Install skill directory and command wrapper

Usage: temper skill install [OPTIONS]

Options:
      --target <TARGET>
          Which agent to install for. Determines the default skill directory and command wrapper location: `claude` writes to `~/.claude/skills/temper/` + `~/.claude/commands/temper.md`; `opencode` writes to `~/.config/opencode/skills/temper/` + `~/.config/opencode/command/temper.md`. Defaults to `claude` for back-compat

          Possible values:
          - claude:   Claude Code — skill to `~/.claude/skills/temper/`, wrapper to `~/.claude/commands/temper.md`
          - opencode: opencode — skill to `~/.config/opencode/skills/temper/`, wrapper to `~/.config/opencode/command/temper.md`
          
          [default: claude]

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --path <PATH>
          Override the skill install directory (overrides the target's default)

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

### `temper skill check`

```text
Check skill status

Usage: temper skill check [OPTIONS]

Options:
      --target <TARGET>
          Which agent to check for. Determines the expected command-wrapper location. Defaults to `claude`

          Possible values:
          - claude:   Claude Code — skill to `~/.claude/skills/temper/`, wrapper to `~/.claude/commands/temper.md`
          - opencode: opencode — skill to `~/.config/opencode/skills/temper/`, wrapper to `~/.config/opencode/command/temper.md`
          
          [default: claude]

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

### `temper skill emit`

```text
Emit the MCP (`agent-skills/`) projection into a directory.

Config-free by construction — unlike `install`, this reads no config and bakes in no per-user state, which is what makes the emitted tree committable and pinnable by a drift gate. Writes only the generated files; hand-written siblings in the directory are untouched.

Usage: temper skill emit [OPTIONS] --path <PATH>

Options:
      --path <PATH>
          Directory to write the tree into (e.g. `agent-skills`)

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```
