<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper update`

Self-update the CLI to the latest release (curl-script installs only).

```text
Self-update the CLI to the latest release (curl-script installs only).

Resolves the latest published release and compares it against the running binary's compiled version. When newer (or with `--force`), downloads that release's archive and manifest, verifies GitHub's build-provenance attestation for that exact downloaded archive against a pinned Sigstore trust root (mandatory — there is no bypass flag), verifies every manifest file against the same archive's contents, and only then hands the already-verified archive to the embedded installer to atomically replace the whole install directory (binary + bundled `lib/libonnxruntime.*`), re-pointing the on-PATH symlink. Refuses on `cargo install` builds (no archive provenance). `--check` reports current-vs-latest, mutating nothing. Unix-first; Windows self-update is a follow-up.

Usage: temper update [OPTIONS]

Options:
      --check
          Report current-vs-latest and exit without mutating anything (dry run)

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --version <VERSION>
          Pin a specific release tag to install (e.g. v0.3.0), bypassing the latest-release lookup and the already-current no-op

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --force
          Reinstall even when already on the latest version (repair path)

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```
