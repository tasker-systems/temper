<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper memory`

Manage the Claude Code memory projection

```text
Manage the Claude Code memory projection

Usage: temper memory [OPTIONS] <COMMAND>

Commands:
  status   Report this machine's memory state — works whether or not you have opted in
  emit     Render the index from Temper and write it
  migrate  Move local memory files into Temper, reconciling rather than blind-creating
  harvest  Copy each curated title out of the hand-written index into the file it names
  check    Check whether the on-disk index matches a fresh render — the LOCAL drift gate
  help     Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper memory status`

```text
Report this machine's memory state — works whether or not you have opted in

Usage: temper memory status [OPTIONS]

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper memory emit`

```text
Render the index from Temper and write it

Usage: temper memory emit [OPTIONS]

Options:
      --path <PATH>        Override the configured index_path (for a machine mid-adoption)
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper memory migrate`

```text
Move local memory files into Temper, reconciling rather than blind-creating.

The reconciliation is by `source_file`: a file some memory already names is skipped, which is what makes re-running safe. It does NOT detect near-duplicates — nothing is compared against what the target context already holds, so adjudicating overlap is a step to take before running this, not something the command does for you. Interactive by default, where it confirms the whole batch once (count, target context, cohort) before the first write; with no terminal attached it refuses to write unless `--unattended` explicitly authorizes an unconfirmed run. `--dry-run` is always permitted and writes nothing.

Titles come from the link text in the hand-written `MEMORY.md`, which is the only place a human-readable title for each memory exists. A file no link names is skipped, never given a title invented from its filename.

Usage: temper memory migrate [OPTIONS]

Options:
      --cohort <COHORT>
          Which cohort to migrate, by the files' frontmatter `type` (e.g. `feedback`)
          
          [default: feedback]

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --context <CONTEXT>
          Target context ref. Defaults to the first `shared_contexts` entry for the cross-project cohort, else the first `project_contexts` entry

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --dry-run
          Plan and print; write nothing

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

      --unattended
          Authorize a write run with no terminal attached. The batch is written without the confirmation prompt — the flag is the authorization

  -h, --help
          Print help (see a summary with '-h')
```

### `temper memory harvest`

```text
Copy each curated title out of the hand-written index into the file it names.

Run this BEFORE letting `emit` take the index over. A memory's human-readable title exists in exactly one place — the link text in `MEMORY.md` — so the takeover destroys it, and with it `migrate`'s ability to move the remaining files at all. A title is never invented from a filename: measured on a real corpus, that loses the hook on nearly half the files.

Idempotent: a file that already carries a `title:` is skipped, so a second run changes nothing. Each stamp also pins `metadata.modified` to the file's pre-write mtime, so the write's own mtime bump cannot re-date a claim nobody re-checked.

Usage: temper memory harvest [OPTIONS]

Options:
      --dry-run
          Plan and print; write nothing

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --unattended
          Authorize a write run with no terminal attached

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

### `temper memory check`

```text
Check whether the on-disk index matches a fresh render — the LOCAL drift gate.

The index lives outside the repo (per-machine, under `~/.claude/`), so nothing in git can diff it — this is the command a person or a hook runs instead, gating on the exit code. Exits non-zero when the on-disk index has drifted from Temper.

Usage: temper memory check [OPTIONS]

Options:
      --path <PATH>
          Override the configured index_path — must match whatever `emit --path` last wrote, or this checks a different file than the one that was written

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
