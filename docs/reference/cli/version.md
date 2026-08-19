<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper version`

Print the CLI version, optionally with the running binary's SHA-256 or an offline (or online) manifest verdict.

```text
Print the CLI version, optionally with the running binary's SHA-256 or an offline (or online) manifest verdict.

`temper --version` / `-V` (injected by clap) is the terse form. This subcommand renders a typed report through the `--format json|toon` machinery; `--checksum` folds in the running binary's own SHA-256 and resolved path (self-attestation — NOT the published archive checksum).

Usage: temper version [OPTIONS]

Options:
      --checksum
          Also compute and print the SHA-256 of the running binary

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --verify
          Verify the installed files against the release manifest beside them. Offline: detects corruption and drift, not an attacker who could replace both the binary and the manifest

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --online
          Re-fetch the published manifest for this version and host triple from GitHub, and compare against that instead of the copy installed beside the binary. Once the manifest agrees, also verifies GitHub's build-provenance attestation for the published archive against a pinned Sigstore trust root — the same check `temper update` performs before installing. Requires --verify

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```
