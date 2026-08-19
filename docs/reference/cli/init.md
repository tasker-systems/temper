<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper init`

Initialize a new vault

```text
Initialize a new vault

Usage: temper init [OPTIONS] [PATH]

Arguments:
  [PATH]
          Path for the new vault (default: current directory)

Options:
      --no-interactive
          Skip interactive prompts

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --instance-url <INSTANCE_URL>
          Self-host: instance base URL (e.g. <https://temper.acme.com>)
          
          For Auth0/Okta this requires `--auth-domain`, `--auth-client-id`, and `--auth-audience` (validated at run time); `--idp temper-as` needs only this flag.

      --auth-domain <AUTH_DOMAIN>
          Self-host: OAuth provider domain (e.g. acme.us.auth0.com or acme.okta.com)

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --auth-client-id <AUTH_CLIENT_ID>
          Self-host: CLI application client_id

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

      --auth-audience <AUTH_AUDIENCE>
          Self-host: API audience (e.g. <https://temper.acme.com/api>)

      --idp <IDP>
          Self-host: identity provider URL shape (default: auth0)
          
          [default: auth0]

      --auth-server-id <AUTH_SERVER_ID>
          Self-host: Okta authorization server ID (required with --idp okta)

  -h, --help
          Print help (see a summary with '-h')
```
