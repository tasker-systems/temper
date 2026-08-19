<!-- GENERATED — do not edit. Rendered from temper-core's `config-reference` example by scripts/emit-config-reference.py; see .github/scripts/check-config-reference-drift.sh. -->

# Configuration reference

Every field of `TemperConfig`, rendered from the type itself. Descriptions are the
doc comments on the Rust struct, and defaults are the real `TemperConfig::default()`,
so a page that disagrees with the binary is a defect — nothing here is hand-written.

Config lives at `~/.config/temper/config.toml`, or wherever `TEMPER_GLOBAL_CONFIG`
points. An absent file is not an error: every section falls back to the defaults below.

## Defaults

The starting config, serialized by the same `toml` crate that parses it at load time.
Sections whose fields are all unset appear as empty tables.

```toml
[vault]
path = "~/Documents/temper-vault"

[sync.subscriptions]
contexts = []

[skill]
output = "~/.claude/skills/temper"

[auth]
provider = "none"
providers = []

[cloud]
api_url = ""

[llm]
provider = "ollama"
url = "http://localhost:11434"
model = "llama3.2:latest"
request_timeout_secs = 300

[cli]
```

## Fields

Canonical temper config — `~/.config/temper/config.toml`. Single config file replacing the old split model (global config + vault temper.toml). Imported by temper-cli, temper-client, temper-mcp, and any future crate.

### `[auth]`

Auth configuration.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `path` | string (optional) | _unset_ | Override for the on-disk auth file path. Tilde-expanded at resolution time. When `None`, falls back to `~/.config/temper/auth.json`. Has lower precedence than the `TEMPER_AUTH_PATH` env var. |
| `provider` | string | `none` | _(undocumented — this field has no doc comment in `TemperConfig`)_ |

#### `[[auth.providers]]`

A single auth provider entry. Stored in `[[auth.providers]]` arrays in TOML.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `audience` | string **(required)** | _unset_ | _(undocumented — this field has no doc comment in `TemperConfig`)_ |
| `authorize_url` | string **(required)** | _unset_ | _(undocumented — this field has no doc comment in `TemperConfig`)_ |
| `callback_url` | string | `""` (empty) | _(undocumented — this field has no doc comment in `TemperConfig`)_ |
| `client_id` | string **(required)** | _unset_ | _(undocumented — this field has no doc comment in `TemperConfig`)_ |
| `name` | string **(required)** | _unset_ | Provider name — referenced by `auth.provider` to pick the active entry. |
| `scopes` | array of string | `[]` | _(undocumented — this field has no doc comment in `TemperConfig`)_ |
| `token_url` | string **(required)** | _unset_ | _(undocumented — this field has no doc comment in `TemperConfig`)_ |

### `[cli]`

CLI output-presentation defaults, stored under `[cli]` in config.toml. These fields supply the config-file layer of the resolution precedence chain for CLI output settings. Resolution order (highest to lowest): `flag → env → config → tty-default`. That resolution logic lives in the CLI (temper-cli); this struct is intentionally free-form (no `#[validate(...)]` constraints) so the CLI decides what constitutes a valid value and can fall through to defaults when given garbage input.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `color` | string (optional) | _unset_ | Default color choice: `"auto"` \| `"always"` \| `"never"`. `None` when unset. |
| `format` | string (optional) | _unset_ | Default output format: `"json"` \| `"toon"`. `None` when unset. |
| `warmup_goals` | integer (optional) | _unset_ | How many active goals `temper warmup` lists. `None` when unset. A cap here is never silent — the primer always reports the true active total alongside. |
| `warmup_sessions` | integer (optional) | _unset_ | How many recent sessions `temper warmup` surfaces as pointers. `None` when unset. |

### `[cloud]`

Cloud API section of the configuration.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `api_url` | string | `""` (empty) | API base URL (overridden by `TEMPER_API_URL` environment variable). Empty means "unconfigured" — set by `temper init`. |

### `[llm]`

LLM configuration section.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `api_key` | string (optional) | _unset_ | API key — read from `TEMPER_LLM_API_KEY` env var at call site when None. Only set this field if you want the key in the config file (not recommended). |
| `model` | string | `llama3.2:latest` | Model identifier (e.g. `llama3.2:latest`, `claude-sonnet-4-5`). Defaults vary by provider. |
| `provider` | `ollama` \| `claude` \| `open_ai_compatible` | `ollama` | Which LLM backend to use. |
| `request_timeout_secs` | integer | `300` | HTTP request timeout in seconds for LLM provider calls. Reasoning / large cloud models may need longer than the default. |
| `url` | string | `http://localhost:11434` | Base URL for the LLM API (e.g. `http://localhost:11434` for ollama). Defaults to `http://localhost:11434` for ollama-compatible providers. |

### `[memory]`

Claude Code memory projection. **Absent means the feature is off** — this is `Option` rather than `#[serde(default)]` precisely so "not configured" and "configured empty" stay distinguishable.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `index_path` | string **(required)** | _unset_ | Where the rendered index is written. |
| `project_contexts` | array of string | `[]` | Contexts for this project. A list — a project may legitimately span contexts. |
| `reinforced_min` | integer (optional) | _unset_ | How many distinct `open_meta.reinforced` dates a memory needs before the index renders it on its own line. Below it, the memory is **collapsed into a per-section tail line** — demoted, never dropped. **`None` is not "off by default", it is the only honest starting value, and it must stay undefaulted.** A threshold is a number that can only be set from months of real reinforcement data; picking one here would be a constant with no evidence behind it, and every machine would inherit the guess. So this is `Option` with **no** `#[serde(default)]` — deliberately unlike `stale_after_days` directly above, whose 90 is a rendering nicety rather than a claim about which memories matter. Absent means the index renders exactly what it rendered before this key existed, which `render::tests::an_absent_threshold_renders_byte_for_byte_what_it_rendered_before` asserts against the whole string. |
| `shared_contexts` | array of string | `[]` | Contexts whose memories reach EVERY project on this machine. |
| `stale_after_days` | integer | `90` | Days after which a memory's `verified` date is rendered as UNEXAMINED. |

### `[skill]`

Skill generation config.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `output` | string | `~/.claude/skills/temper` | _(undocumented — this field has no doc comment in `TemperConfig`)_ |

### `[sync]`

Sync config — which contexts are synced.

#### `[sync.subscriptions]`

Sync subscriptions — which contexts are synced.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `contexts` | array of string | `[]` | _(undocumented — this field has no doc comment in `TemperConfig`)_ |

### `[vault]`

Vault path reference in cloud config.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `path` | string **(required)** | `~/Documents/temper-vault` | Path to the local vault directory |

## Undocumented fields

9 of 27 fields carry no doc comment on the Rust struct, so this reference cannot describe them. They are listed rather than left as blank cells, because a documentation hole that renders as whitespace reads as documentation.

- `auth.provider`
- `auth.providers.audience`
- `auth.providers.authorize_url`
- `auth.providers.callback_url`
- `auth.providers.client_id`
- `auth.providers.scopes`
- `auth.providers.token_url`
- `skill.output`
- `sync.subscriptions.contexts`
