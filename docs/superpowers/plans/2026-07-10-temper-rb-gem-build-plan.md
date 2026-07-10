# temper-rb Gem Build: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `clients/temper-rb/` — a pure-Ruby SDK for the temper cloud API, built from a committed openapi-generator core plus a hand-written skin.

**Architecture:** `rake generate` runs openapi-generator (pinned, via Docker) against the repo-root `openapi.json` and writes **only** `lib/temper/generated/**` under `Temper::Generated::`. A hand-written skin at `lib/temper/*.rb` under `Temper::` wraps it: process-global connection, call-scoped credentials, a transient/permanent error split, and an `Act` value object. The skin never lives inside the generated tree, so clobbering is structurally impossible rather than ignore-file-dependent.

**Tech Stack:** Ruby 3.4.10 (dev) / `>= 3.1` (floor) · faraday 2.x + `faraday-net_http_persistent` · RSpec + WebMock · RuboCop · openapi-generator `v7.23.0` via Docker.

**Source spec:** [2026-07-09-temper-rb-gem-design.md](../specs/2026-07-09-temper-rb-gem-design.md) (decisions D8–D17).

---

## Global Constraints

- **Ruby floor `>= 3.1`.** Dev pin `3.4.10` via `.ruby-version` (read natively by both mise and rbenv).
- **Gem name is `temper-rb`.** `temper` is already taken on RubyGems (`GET /api/v1/gems/temper.json` → 200). Ruby module is `Temper`; require path is `temper`.
- **No native extension.** No `ffi`, no `ethon`, no `typhoeus`, no `onnxruntime`. One source gem, no platform matrix, no compiler on the install box. (D9, D11, D16)
- **No `dry-*` runtime dependency.** The skin returns generated model instances directly; it does not re-wrap them. (D14)
- **Never hand-edit `lib/temper/generated/**`.** `rake generate` is its only writer.
- **Never widen the skin into the generated tree.** Skin files live at `lib/temper/*.rb`, never `lib/temper/generated/*.rb`.
- **`X-Temper-Surface: sdk`** is set once in `ApiClient#default_headers`. The generator emits an `x_temper_surface` kwarg on all 79 operations; the skin never exposes it.
- **The skin classifies errors; it never auto-retries a write.** Same rule as the Rust client.
- **Generator is pinned to `openapitools/openapi-generator-cli:v7.23.0`.** `latest` resolves to `7.24.0-SNAPSHOT`, an unreleased moving build — pinning is what makes the `rake generate && git diff --exit-code` drift gate meaningful.
- **`cargo make check` applies only to tasks that touch Rust or `.github/`.** The gem is inert to cargo (`members = ["crates/*", "tests/e2e"]`) and to bun (explicit two-entry `workspaces`).

---

## Context an implementer needs

**The design spec is authoritative for *why*. This plan is authoritative for *what*, and it corrects the spec in seven places** — each correction was established by running the real generator against the real contract, not by reasoning:

1. **`ActInput` has seven keys, not six**: `confidence`, `correlation_id`, `invocation_id`, `model`, `persona`, `rationale`, `reasoning`. The spec says "the six `ActInput` keys" twice.
2. **`IngestPayload` is `allOf: [ActInput, {…}]` in the contract**, and openapi-generator **flattens it** into a single model carrying all seven act keys as plain `attr_accessor`s alongside the body fields. The skin sets them directly; there is no composition wrapper to unwrap.
3. **`gemName=temper/generated` yields D10's exact layout for free** — `lib/temper/generated/{models,api}` with correct `require 'temper/generated/...'` paths. No post-processing script, no `sed` over the emitted tree.
4. **`gemVersion=<info.version>` makes `Temper::Generated::VERSION` *be* the contract version.** D16's `CONTRACT_VERSION` needs no custom rake logic — it is an alias. Without this flag the generator writes `VERSION = '1.0.0'`, which is a lie.
5. **`faraday >= 1.0.1, < 3.0`** (the generated gemspec's constraint, quoted by D11) **cannot hold.** `faraday-net_http_persistent` 2.3.1 requires Faraday 2.x. The floor narrows to `>= 2.5, < 3.0`.
6. **Two paths the spec names do not exist.** D9 writes `POST /api/relationships/assert` and `POST /api/facets/set`. The real paths are **`POST /api/relationships`** and **`POST /api/facets`**.
7. **The gem cannot be named `temper`** — that name is taken on RubyGems. It is `temper-rb`, matching the `tasker-rb` precedent. The Ruby module stays `Temper`.

**Success status codes, because guessing them makes a spec's WebMock stubs lie:** ingest `200`, cogmap genesis `200`, `DELETE /api/resources/{id}` **`200`** (not 204), context create `201`.

**Three generated-code facts that decide skin implementation:**

- `Generated::ApiClient#build_connection` calls `conn.adapter(Faraday.default_adapter)` **before** `config.configure_connection(conn)` (`api_client.rb:224–225`). Faraday's builder lets the later `adapter` call replace the earlier one — which is the *only* reason D11's "swap the adapter via `configure_connection`" works. Do not move it to `configure_middleware`, which runs *before* the default adapter is set.
- `Generated::ApiClient#call_api` rescues `Faraday::TimeoutError` and `Faraday::ConnectionFailed` and re-raises them as `ApiError.new('Connection timed out')` / `ApiError.new('Connection failed')` (`api_client.rb:76–79`). Those carry **`code == nil`**. The error classifier must map a nil-code `ApiError` to `Temper::ConnectionError`, or every timeout becomes an unclassified `Temper::Error`.
- `Generated::ApiError` exposes `#code` (the HTTP status, Integer), `#response_headers`, `#response_body`, and `#message`.

**Contract facts:**

- The server speaks one error envelope: `{"error": {"code": String, "message": String, "details": Any}}` (`ErrorBody` → `ErrorDetail`). `details` is untyped.
- **No operation declares `422`, `429`, `500`, or `503`.** Only `200/201/204/400/401/403/404/409` appear. So `RateLimited` and `ServerError` classify off the raw HTTP status, never off the contract. Bodies for undeclared statuses still parse — the envelope is parsed from `response_body` directly, independent of what the contract declares.
- `DELETE /api/resources/{id}` takes act context as **seven query parameters**, not a body: `invocation_id`, `correlation_id`, `reasoning`, `confidence`, `rationale`, `persona`, `model`.
- All 64 paths carry the `X-Temper-Surface` header as a path-item-level parameter.
- `ContextOwnerRef` generates as a **`module` with `openapi_one_of`**, not a class. The skin hand-constructs that payload.

**Generated names, verified — do not guess these.** They are the operation ids P5 fixed, and the model names derive from the contract's schema names, not from what you would call them:

| What you want | The generated call | The body model |
| --- | --- | --- |
| Create a resource | `IngestApi#create_ingest(payload)` → **200** (not 201) | `IngestPayload` |
| Read a resource | `ResourcesApi#get_resource(id)` — **takes no `edges` option** | → `ResourceDetail` |
| Read its edges | `ResourcesApi#list_resource_edges(id)` — a *separate* call | |
| Read meta only | `MetaApi#get_meta(id)` | → `ResourceMetaResponse` |
| Update / delete | `ResourcesApi#update_resource(id, body)` / `#delete_resource(id, opts)` | `ResourceUpdateRequest` |
| Create a context | `ContextsApi#create_context(body)` | `ContextCreateRequest` — fields are **`name`** and `owner`, not `slug` |
| Create a cogmap | `CognitiveMapsApi#genesis(body)` | `CreateCogmapRequest` — `cogmap_id`, `name`, `telos`, `telos_resource_id`, `telos_title` |
| Assert an edge | `RelationshipsApi#assert(body)` | `AssertRelationshipRequest` — **`source`, `target`, `edge_kind`**, `label`, `polarity`, `weight` |
| Set a facet | `FacetsApi#set_facet(body)` | `FacetSetRequest` — **`resource`, `values`** (plural), `weight` |
| Search | `SearchApi#search(params)` | `SearchParams` — the field is **`query`**, not `q` |
| Whoami | `ProfileApi#get_profile` | `GET /api/profile` **declares no response schema**; it deserializes to a plain Hash |

`AssertRelationshipRequest` and `FacetSetRequest` both carry the seven `ActInput` keys as flattened top-level attributes, exactly as `IngestPayload` does. `CreateCogmapRequest` does **not**.

`delete_resource(id, opts)`'s `opts` keys are precisely the seven act keys as symbols — so `Act#to_h` can be passed straight through.

**Ruby-version gotcha:** the floor is 3.1, and `Fiber[]` storage arrived in 3.2. Use `Thread.current[:key]`, which is **fiber-local** in Ruby (`Thread#thread_variable_get` is the thread-local one). This satisfies D12's "fiber-local" requirement on 3.1.

**Module-definition gotcha:** every generated file opens with the compact form `module Temper::Generated`, which raises `NameError` unless `Temper` is already defined. `lib/temper.rb` must define `module Temper; end` **before** requiring anything generated.

**Local toolchain:** `mise install ruby@3.4.10` requires `libyaml` on macOS or `psych` fails to configure and the build dies late (`brew install libyaml`). The generator needs Docker.

---

## File Structure

| File | Responsibility |
| --- | --- |
| `clients/temper-rb/.ruby-version` | Dev toolchain pin (`3.4.10`) |
| `clients/temper-rb/temper-rb.gemspec` | Gem metadata, deps, `required_ruby_version` |
| `clients/temper-rb/Gemfile` | `gemspec` + dev/test groups |
| `clients/temper-rb/.rubocop.yml` | Lint config, `TargetRubyVersion: 3.1` |
| `clients/temper-rb/.openapi-generator-ignore` | Suppresses everything the generator emits outside `lib/temper/generated/**` |
| `clients/temper-rb/Rakefile` | `rake generate`, `rake spec`, `rake lint` |
| `clients/temper-rb/lib/temper.rb` | Defines `module Temper`, requires skin + generated core, `Temper.configure` |
| `clients/temper-rb/lib/temper/version.rb` | `Temper::VERSION` (independent SemVer), `Temper::CONTRACT_VERSION` |
| `clients/temper-rb/lib/temper/errors.rb` | Exception tree + `ErrorMapper` (D13) |
| `clients/temper-rb/lib/temper/credentials.rb` | `BearerToken`, `ClientCredentials` (D12) |
| `clients/temper-rb/lib/temper/connection.rb` | Process-global `ApiClient`, adapter swap, `reset_connection!` (D11/D12) |
| `clients/temper-rb/lib/temper/act.rb` | `Temper::Act` value object + constructor invariant |
| `clients/temper-rb/lib/temper/refs.rb` | `Temper.parse_ref` |
| `clients/temper-rb/lib/temper/client.rb` | Façade: credential scoping, 401 re-mint, GET/HEAD retry, `#whoami` |
| `clients/temper-rb/lib/temper/resources.rb` | Resource surface (create/show/update/delete/list) |
| `clients/temper-rb/lib/temper/contexts.rb` | Context surface |
| `clients/temper-rb/lib/temper/cognitive_maps.rb` | Incremental cogmap authoring (D9) |
| `clients/temper-rb/lib/temper/generated/**` | **Generated.** Never hand-edited. |
| `.github/workflows/test-ruby.yml` | Ruby CI: rubocop, rspec, drift gate |
| `.github/workflows/ci.yml` | Wire `test-ruby` into scope + success gate |
| `.github/scripts/detect-ci-scope.sh` | Emit `run-test-ruby` |
| `.github/scripts/test-detect-ci-scope.sh` | Unit-test the new flag |

**Task dependency order:** 1 → 2 → (3, 4 parallel) → 5 → 6 → 7 → 8 → 9 → 10 → 11.

---

### Task 1: Scaffold the gem and pin the toolchain

Nothing generated yet. This task proves `bundle install` and `rspec` run green in `clients/temper-rb/` before any complexity lands.

**Files:**
- Create: `clients/temper-rb/.ruby-version`
- Create: `clients/temper-rb/temper-rb.gemspec`
- Create: `clients/temper-rb/Gemfile`
- Create: `clients/temper-rb/.rubocop.yml`
- Create: `clients/temper-rb/.rspec`
- Create: `clients/temper-rb/.gitignore`
- Create: `clients/temper-rb/lib/temper.rb`
- Create: `clients/temper-rb/lib/temper/version.rb`
- Test: `clients/temper-rb/spec/spec_helper.rb`, `clients/temper-rb/spec/version_spec.rb`

**Interfaces:**
- Produces: `Temper::VERSION` (String, SemVer). `Temper.configure { |c| ... }` with `c.base_url`, `c.device_id`. `Temper.config` returns the memoized `Temper::Configuration`.

- [ ] **Step 1: Pin the Ruby version, twice**

**mise does not read `.ruby-version`** unless `idiomatic_version_file_enable_tools` names `ruby`, and that setting is an empty list by default (`mise settings get idiomatic_version_file_enable_tools` → `[]`). rbenv and `ruby/setup-ruby` read *only* `.ruby-version`. Neither file alone covers both, so both exist and must be kept equal.

`clients/temper-rb/.ruby-version`:
```
3.4.10
```

`clients/temper-rb/mise.toml`:
```toml
# mise does NOT read `.ruby-version` unless `idiomatic_version_file_enable_tools`
# names ruby, and that setting is empty by default. So the pin lives here for
# mise, and in `.ruby-version` for rbenv and `ruby/setup-ruby`. Keep them equal.
[tools]
ruby = "3.4.10"
```

Verify the interpreter resolves:
```bash
cd clients/temper-rb && mise trust && mise exec -- ruby -v
```
Expected: `ruby 3.4.10 (...) [arm64-darwin25]` (or the local arch).

> On macOS, `mise install ruby@3.4.10` fails **late** — after ~5 minutes of compiling — with `psych: Could not be configured` if libyaml is absent. `brew install libyaml` first.

- [ ] **Step 2: Write the gemspec**

`clients/temper-rb/temper-rb.gemspec`:
```ruby
# frozen_string_literal: true

require_relative 'lib/temper/version'

Gem::Specification.new do |spec|
  spec.name          = 'temper-rb'
  spec.version       = Temper::VERSION
  spec.authors       = ['Pete Taylor']
  spec.email         = ['pete.jc.taylor@hey.com']

  spec.summary       = 'Ruby SDK for the Temper knowledge-base API'
  spec.description   = <<~DESC
    A pure-Ruby client for the Temper cloud API: resources, contexts, ingest,
    search, graph, and incremental cognitive-map authoring. No native extension.
  DESC

  spec.homepage      = 'https://github.com/tasker-systems/temper'
  spec.license       = 'MIT'
  spec.required_ruby_version = '>= 3.1'

  spec.metadata['homepage_uri']         = spec.homepage
  spec.metadata['source_code_uri']      = 'https://github.com/tasker-systems/temper/tree/main/clients/temper-rb'
  spec.metadata['bug_tracker_uri']      = 'https://github.com/tasker-systems/temper/issues'
  spec.metadata['allowed_push_host']    = 'https://rubygems.org'
  spec.metadata['rubygems_mfa_required'] = 'true'

  spec.files = Dir['lib/**/*.rb', 'README.md', 'LICENSE'].select { |f| File.file?(f) }
  spec.require_paths = ['lib']

  # Faraday 2.x floor is forced by faraday-net_http_persistent 2.x (D11).
  spec.add_dependency 'faraday', '>= 2.5', '< 3.0'
  spec.add_dependency 'faraday-multipart', '~> 1.0'
  spec.add_dependency 'faraday-net_http_persistent', '~> 2.0'
  spec.add_dependency 'marcel', '~> 1.0'
end
```

> `faraday-multipart` and `marcel` are runtime dependencies of the **generated** `api_client.rb`, which requires them unconditionally. Both are pure Ruby.

- [ ] **Step 3: Write the Gemfile, rubocop config, and rspec config**

`clients/temper-rb/Gemfile`:
```ruby
# frozen_string_literal: true

source 'https://rubygems.org'

gemspec

group :development, :test do
  gem 'rake', '~> 13.0'
  gem 'rspec', '~> 3.13'
  gem 'rubocop', '~> 1.60'
  gem 'rubocop-rspec', '~> 3.0'
  gem 'webmock', '~> 3.23'
end
```

`clients/temper-rb/.rubocop.yml`:
```yaml
plugins:
  - rubocop-rspec

AllCops:
  TargetRubyVersion: 3.1
  NewCops: enable
  SuggestExtensions: false
  Exclude:
    - 'lib/temper/generated/**/*'
    - 'lib/temper/generated.rb'
    - 'vendor/**/*'

Style/Documentation:
  Enabled: false

Layout/LineLength:
  Max: 120

Metrics/BlockLength:
  Exclude:
    - 'spec/**/*'

RSpec/MultipleExpectations:
  Enabled: false

RSpec/ExampleLength:
  Enabled: false
```

`clients/temper-rb/.rspec`:
```
--require spec_helper
--color
--format documentation
```

`clients/temper-rb/.gitignore`:
```
/.bundle/
/pkg/
/tmp/
*.gem
Gemfile.lock
```

- [ ] **Step 4: Write the failing version spec**

`clients/temper-rb/spec/spec_helper.rb`:
```ruby
# frozen_string_literal: true

require 'temper'
require 'webmock/rspec'

RSpec.configure do |config|
  config.expect_with(:rspec) { |c| c.syntax = :expect }
  config.disable_monkey_patching!
  config.order = :random
  Kernel.srand config.seed
end
```

`clients/temper-rb/spec/version_spec.rb`:
```ruby
# frozen_string_literal: true

RSpec.describe Temper do
  it 'exposes a SemVer gem version' do
    expect(Temper::VERSION).to match(/\A\d+\.\d+\.\d+\z/)
  end

  it 'memoizes a single configuration object' do
    expect(described_class.config).to be(described_class.config)
  end

  it 'yields the configuration to configure' do
    described_class.configure { |c| c.base_url = 'https://example.test' }
    expect(described_class.config.base_url).to eq('https://example.test')
  end
end
```

- [ ] **Step 5: Run the spec to verify it fails**

```bash
cd clients/temper-rb && bundle install && bundle exec rspec spec/version_spec.rb
```
Expected: FAIL — `cannot load such file -- temper`.

- [ ] **Step 6: Write the minimal implementation**

`clients/temper-rb/lib/temper/version.rb`:
```ruby
# frozen_string_literal: true

module Temper
  VERSION = '0.1.0'
end
```

`clients/temper-rb/lib/temper.rb`:
```ruby
# frozen_string_literal: true

# `Temper` must exist before any generated file is required: every generated
# file opens with the compact form `module Temper::Generated`, which raises
# NameError if `Temper` is not already defined.
module Temper
  # Process-wide settings. Credentials are NOT here — they are per-call (D12).
  class Configuration
    attr_accessor :base_url, :device_id
  end

  class << self
    def config
      @config ||= Configuration.new
    end

    def configure
      yield(config)
      config
    end
  end
end

require 'temper/version'
```

- [ ] **Step 7: Run the spec to verify it passes**

```bash
cd clients/temper-rb && bundle exec rspec spec/version_spec.rb
```
Expected: PASS — 3 examples, 0 failures.

- [ ] **Step 8: Verify rubocop is clean**

```bash
cd clients/temper-rb && bundle exec rubocop
```
Expected: `no offenses detected`.

- [ ] **Step 9: Commit**

```bash
git add clients/temper-rb
git commit -m "temper-rb: scaffold the gem and pin the toolchain

Gem name is temper-rb; 'temper' is taken on RubyGems. Ruby floor >= 3.1,
dev pin 3.4.10. Faraday 2.x floor is forced by faraday-net_http_persistent."
```

---

### Task 2: `rake generate` and the committed generated core

**Files:**
- Create: `clients/temper-rb/.openapi-generator-ignore`
- Create: `clients/temper-rb/Rakefile`
- Modify: `clients/temper-rb/lib/temper.rb` (require the generated core)
- Modify: `clients/temper-rb/lib/temper/version.rb` (add `CONTRACT_VERSION`)
- Test: `clients/temper-rb/spec/generated_contract_spec.rb`
- Generated (committed, never hand-edited): `clients/temper-rb/lib/temper/generated/**`, `clients/temper-rb/lib/temper/generated.rb`

**Interfaces:**
- Consumes: `Temper::VERSION` from Task 1.
- Produces: `Temper::Generated::ApiClient`, `Temper::Generated::Configuration`, `Temper::Generated::ApiError`, `Temper::Generated::ResourcesApi` (and 17 sibling API classes), 152 models. `Temper::CONTRACT_VERSION` (String) — aliases `Temper::Generated::VERSION`.

- [ ] **Step 1: Write the generator ignore file**

The generator emits 355 files outside `lib/temper/generated/**` — a gemspec, a Gemfile, a Rakefile, a README, CI configs, 171 rspec stubs, and 170 markdown docs. Every one of them would clobber hand-written work or add noise. openapi-generator writes `.openapi-generator-ignore` only when absent, so authoring it first is what suppresses them.

`clients/temper-rb/.openapi-generator-ignore`:
```
# The generator's ONLY output is lib/temper/generated/** (plus its aggregator,
# lib/temper/generated.rb, and the .openapi-generator/ manifest).
#
# Everything below is hand-written and must never be overwritten. The skin lives
# OUTSIDE the generated tree, so this file is defense-in-depth, not the only
# thing standing between `rake generate` and your code.

Gemfile
Rakefile
README.md
git_push.sh
.gitignore
.rspec
.rubocop.yml
.travis.yml
.gitlab-ci.yml
temper/generated.gemspec
spec/**
docs/**
```

- [ ] **Step 2: Write the Rakefile**

`clients/temper-rb/Rakefile`:
```ruby
# frozen_string_literal: true

require 'json'
require 'rspec/core/rake_task'
require 'rubocop/rake_task'

REPO_ROOT = File.expand_path('../..', __dir__)
SPEC_PATH = File.join(REPO_ROOT, 'openapi.json')

# Pinned deliberately. `latest` resolves to 7.24.0-SNAPSHOT -- an unreleased,
# moving build. A moving generator makes `rake generate && git diff --exit-code`
# fail on days when nothing in this repo changed.
GENERATOR_IMAGE = 'openapitools/openapi-generator-cli:v7.23.0'

RSpec::Core::RakeTask.new(:spec)
RuboCop::RakeTask.new(:lint)

desc 'Regenerate lib/temper/generated/** from the repo-root openapi.json'
task :generate do
  contract_version = JSON.parse(File.read(SPEC_PATH)).fetch('info').fetch('version')

  # --user keeps the emitted files owned by the invoking user. Without it the
  # container writes as root on Linux (CI), and the drift gate cannot read them.
  sh 'docker', 'run', '--rm',
     '--user', "#{Process.uid}:#{Process.gid}",
     '-v', "#{REPO_ROOT}:/local",
     GENERATOR_IMAGE,
     'generate',
     '-i', '/local/openapi.json',
     '-g', 'ruby',
     '--library=faraday',
     '-o', '/local/clients/temper-rb',
     '--additional-properties=' \
     "gemName=temper/generated,moduleName=Temper::Generated,gemVersion=#{contract_version}"
end

desc 'Fail if the committed generated core drifts from the contract'
task drift: :generate do
  sh 'git', 'diff', '--exit-code', '--', 'lib/temper/generated', 'lib/temper/generated.rb'
end

task default: %i[lint spec]
```

> `gemName=temper/generated` is what produces D10's nested layout — `lib/temper/generated/{models,api}` with correct `require 'temper/generated/...'` paths — with no post-processing. `gemVersion=<info.version>` is what makes `Temper::Generated::VERSION` the *contract* version rather than the generator's `1.0.0` default.

- [ ] **Step 3: Run the generator**

```bash
cd clients/temper-rb && bundle exec rake generate
```
Expected: exit 0. Then verify the shape:
```bash
ls lib/temper/generated/models/*.rb | wc -l   # 152
ls lib/temper/generated/api/*.rb | wc -l      # 18
grep -c "VERSION = '0.1.0'" lib/temper/generated/version.rb  # 1
git status --porcelain | grep -vc "^?? lib/temper/generated" # 0 -- nothing else touched
```

- [ ] **Step 4: Write the failing contract spec**

`clients/temper-rb/spec/temper/generated_spec.rb` (the path RuboCop's `RSpec/SpecFilePathFormat` requires for `describe Temper::Generated`):
```ruby
# frozen_string_literal: true

require 'json'

RSpec.describe 'the generated core' do
  let(:contract) do
    JSON.parse(File.read(File.expand_path('../../../openapi.json', __dir__)))
  end

  it 'records the contract version it was generated from' do
    expect(Temper::CONTRACT_VERSION).to eq(contract.fetch('info').fetch('version'))
  end

  it 'keeps the gem version independent of the contract version' do
    expect(Temper::VERSION).not_to be_nil
    expect(Temper.singleton_class.method_defined?(:contract_version)).to be(false)
  end

  # P5 gave every operation a unique operationId. If a future contract change
  # reintroduces a collision, the generator silently emits `list_0` again.
  it 'exposes collision-free resource operations' do
    expect(Temper::Generated::ResourcesApi.instance_methods).to include(:list_resources, :list_resource_edges)
    expect(Temper::Generated::ResourcesApi.instance_methods.grep(/_\d+\z/)).to be_empty
  end

  it 'flattens the seven ActInput keys onto IngestPayload' do
    act_keys = %i[confidence correlation_id invocation_id model persona rationale reasoning]
    expect(Temper::Generated::IngestPayload.instance_methods).to include(*act_keys)
  end
end
```

- [ ] **Step 5: Run it to verify it fails**

```bash
cd clients/temper-rb && bundle exec rspec spec/generated_contract_spec.rb
```
Expected: FAIL — `uninitialized constant Temper::CONTRACT_VERSION`.

- [ ] **Step 6: Wire the generated core into the skin**

**`version.rb` must NOT reference `Generated::VERSION`.** `temper-rb.gemspec` loads it via `require_relative` to read `spec.version`, at which point nothing generated is on the load path — `gem build` would raise `NameError`. Leave `version.rb` standalone and define the alias in `lib/temper.rb`, after the generated core loads.

`clients/temper-rb/lib/temper/version.rb`:
```ruby
# frozen_string_literal: true

module Temper
  # The gem's own SemVer. Independent of the API contract by design (D16):
  # a gem version and an API version answer different questions.
  #
  # This file is loaded by temper-rb.gemspec via require_relative, so it must
  # stand alone: no reference to Temper::Generated, which is not loaded then.
  # CONTRACT_VERSION is defined in lib/temper.rb, after the generated core.
  VERSION = '0.1.0'
end
```

Append to `clients/temper-rb/lib/temper.rb`, replacing the trailing `require 'temper/version'`:
```ruby
# `module Temper` must exist before anything generated is required: every
# generated file opens with the compact form `module Temper::Generated`.
require 'temper/generated'
require 'temper/version'

# The contract this gem was generated against. `rake generate` passes
# openapi.json's info.version to the generator as `gemVersion`, so the generated
# tree already carries it -- we alias it rather than reasserting it, and callers
# never reach into Temper::Generated for it.
Temper::CONTRACT_VERSION = Temper::Generated::VERSION
```

> A constant assignment, not a reopened `module Temper` block — RuboCop's `Style/OneClassPerFile` rejects the second top-level module definition in one file.

- [ ] **Step 7: Run the spec to verify it passes**

```bash
cd clients/temper-rb && bundle exec rspec
```
Expected: PASS — 7 examples, 0 failures.

- [ ] **Step 8: Verify the drift gate is honest**

```bash
cd clients/temper-rb && bundle exec rake drift
```
Expected: exit 0, no diff.

Now prove it **bites** — a drift gate that cannot fail is decoration. The invariant is *"the **committed** generated tree equals a fresh generation from `openapi.json`."* So the tamper must be **staged**. Tampering the working tree alone proves nothing: `drift` depends on `generate`, which rewrites the file before `git diff` ever runs, and the gate correctly reports clean.

```bash
printf '\n# tamper\n' >> lib/temper/generated/version.rb
git add lib/temper/generated/version.rb        # the tamper is now "committed"
bundle exec rake drift; echo "exit=$?"          # expect non-zero, diff shows -# tamper
git add lib/temper/generated/version.rb        # regeneration already cleaned the worktree
bundle exec rake drift; echo "exit=$?"          # expect 0 again
```

> This is the same trap P5 documented: *deleting* an `operation_id` does not fail the uniqueness test, because utoipa falls back to the fn name. Falsify the invariant, don't remove the thing satisfying it.

- [ ] **Step 9: Commit**

```bash
git add clients/temper-rb
git commit -m "temper-rb: generate and commit the client core

Pinned to openapi-generator v7.23.0 -- 'latest' is a moving SNAPSHOT tag and
would break the drift gate. gemName=temper/generated yields the nested layout
directly; gemVersion=<info.version> makes Generated::VERSION the contract
version rather than the generator's 1.0.0 default."
```

---

### Task 3: The error taxonomy (D13)

The split is load-bearing, not cosmetic: Sidekiq retries a job whose exception escapes. A 409 classified transient spins forever; a 503 classified permanent is silently dropped.

**Files:**
- Create: `clients/temper-rb/lib/temper/errors.rb`
- Modify: `clients/temper-rb/lib/temper.rb` (require it)
- Test: `clients/temper-rb/spec/errors_spec.rb`

**Interfaces:**
- Consumes: `Temper::Generated::ApiError` from Task 2.
- Produces: `Temper::Error` and its subtree. `Temper::ErrorMapper.call(api_error) -> Temper::Error`. Every exception responds to `#status`, `#code`, `#message`, `#details`; `Temper::RateLimited` adds `#retry_after`.

- [ ] **Step 1: Write the failing spec**

`clients/temper-rb/spec/errors_spec.rb`:
```ruby
# frozen_string_literal: true

RSpec.describe Temper::ErrorMapper do
  def api_error(status, body)
    Temper::Generated::ApiError.new(code: status, response_headers: {}, response_body: body)
  end

  def envelope(code, message, details = nil)
    JSON.generate(error: { code: code, message: message, details: details })
  end

  it 'maps 409 to a permanent Conflict carrying the envelope' do
    err = described_class.call(api_error(409, envelope('CONFLICT', 'already exists')))
    expect(err).to be_a(Temper::Conflict)
    expect(err).to be_a(Temper::PermanentError)
    expect(err.status).to eq(409)
    expect(err.code).to eq('CONFLICT')
    expect(err.message).to eq('already exists')
  end

  it 'discriminates SystemAccessRequired off error.code, not the status' do
    err = described_class.call(api_error(403, envelope('SYSTEM_ACCESS_REQUIRED', 'grant needed')))
    expect(err).to be_a(Temper::SystemAccessRequired)
    expect(err).to be_a(Temper::Forbidden)
  end

  it 'maps a plain 403 to Forbidden, not SystemAccessRequired' do
    err = described_class.call(api_error(403, envelope('FORBIDDEN', 'nope')))
    expect(err).to be_a(Temper::Forbidden)
    expect(err).not_to be_a(Temper::SystemAccessRequired)
  end

  it 'maps 5xx to a transient ServerError' do
    err = described_class.call(api_error(503, 'upstream down'))
    expect(err).to be_a(Temper::ServerError)
    expect(err).to be_a(Temper::TransientError)
  end

  it 'maps 429 to RateLimited and surfaces Retry-After' do
    raw = Temper::Generated::ApiError.new(
      code: 429, response_headers: { 'Retry-After' => '30' }, response_body: 'slow down'
    )
    err = described_class.call(raw)
    expect(err).to be_a(Temper::RateLimited)
    expect(err.retry_after).to eq(30)
  end

  # The generated ApiClient rescues Faraday::TimeoutError / ConnectionFailed and
  # re-raises them as ApiError with a NIL code. Without this branch every
  # timeout falls through to a bare Temper::Error and Sidekiq dead-letters it.
  it 'maps a nil-code ApiError (timeout / refused) to a transient ConnectionError' do
    raw = Temper::Generated::ApiError.new('Connection timed out')
    err = described_class.call(raw)
    expect(err).to be_a(Temper::ConnectionError)
    expect(err).to be_a(Temper::TransientError)
  end

  it 'degrades to a raw-body detail when the response is not the envelope' do
    err = described_class.call(api_error(500, '<html>502 Bad Gateway</html>'))
    expect(err).to be_a(Temper::ServerError)
    expect(err.code).to be_nil
    expect(err.details).to eq('<html>502 Bad Gateway</html>')
  end

  it 'maps 422 to BadRequest even though no operation declares it' do
    err = described_class.call(api_error(422, envelope('UNPROCESSABLE', 'bad shape')))
    expect(err).to be_a(Temper::BadRequest)
  end
end
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cd clients/temper-rb && bundle exec rspec spec/errors_spec.rb
```
Expected: FAIL — `uninitialized constant Temper::ErrorMapper`.

- [ ] **Step 3: Implement the taxonomy**

`clients/temper-rb/lib/temper/errors.rb`:
```ruby
# frozen_string_literal: true

require 'json'

module Temper
  class Error < StandardError
    attr_reader :status, :code, :details

    def initialize(message = nil, status: nil, code: nil, details: nil)
      super(message)
      @status = status
      @code = code
      @details = details
    end
  end

  # Re-raise these: Sidekiq retries a job whose exception escapes.
  class TransientError < Error; end

  class RateLimited < TransientError
    attr_reader :retry_after

    def initialize(message = nil, retry_after: nil, **kwargs)
      super(message, **kwargs)
      @retry_after = retry_after
    end
  end

  class ServerError < TransientError; end
  class ConnectionError < TransientError; end

  # Rescue these: retrying will not help. Dead-letter them.
  class PermanentError < Error; end

  class Unauthorized < PermanentError; end
  class Forbidden < PermanentError; end
  class SystemAccessRequired < Forbidden; end
  class NotFound < PermanentError; end
  class Conflict < PermanentError; end
  class BadRequest < PermanentError; end

  # Translates the generated core's one flat ApiError into the tree above.
  module ErrorMapper
    SYSTEM_ACCESS_REQUIRED = 'SYSTEM_ACCESS_REQUIRED'

    module_function

    def call(api_error)
      status = api_error.code
      code, message, details = parse_envelope(api_error.response_body)
      message ||= api_error.message
      kwargs = { status: status, code: code, details: details }

      # A nil status means the generated ApiClient rescued a Faraday transport
      # failure (timeout / connection refused) and re-raised it code-less.
      return ConnectionError.new(message, **kwargs) if status.nil?

      build(status, message, code, kwargs, api_error)
    end

    def build(status, message, code, kwargs, api_error)
      case status
      when 400, 422 then BadRequest.new(message, **kwargs)
      when 401 then Unauthorized.new(message, **kwargs)
      when 403 then forbidden(code, message, kwargs)
      when 404 then NotFound.new(message, **kwargs)
      when 409 then Conflict.new(message, **kwargs)
      when 429 then RateLimited.new(message, retry_after: retry_after_of(api_error), **kwargs)
      when 500..599 then ServerError.new(message, **kwargs)
      else Error.new(message, **kwargs)
      end
    end

    def forbidden(code, message, kwargs)
      return SystemAccessRequired.new(message, **kwargs) if code == SYSTEM_ACCESS_REQUIRED

      Forbidden.new(message, **kwargs)
    end

    # The server speaks exactly one envelope: {"error":{code,message,details}}.
    # Anything else (an HTML 502 from a proxy, an undeclared 500) degrades to a
    # raw body on #details rather than raising inside the error path.
    def parse_envelope(body)
      return [nil, nil, nil] if body.nil? || body.to_s.empty?

      parsed = JSON.parse(body.to_s)
      error = parsed.is_a?(Hash) ? parsed['error'] : nil
      return [nil, nil, body] unless error.is_a?(Hash)

      [error['code'], error['message'], error['details']]
    rescue JSON::ParserError
      [nil, nil, body]
    end

    def retry_after_of(api_error)
      headers = api_error.response_headers || {}
      raw = headers['Retry-After'] || headers['retry-after']
      raw && Integer(raw, exception: false)
    end
  end
end
```

- [ ] **Step 4: Require it and run the spec**

Add `require 'temper/errors'` to `lib/temper.rb` after `require 'temper/version'`.

```bash
cd clients/temper-rb && bundle exec rspec spec/errors_spec.rb
```
Expected: PASS — 8 examples, 0 failures.

- [ ] **Step 5: Commit**

```bash
git add clients/temper-rb
git commit -m "temper-rb: transient/permanent error taxonomy

A nil-code ApiError means the generated client rescued a Faraday transport
failure; it maps to ConnectionError (transient), not a bare Error."
```

---

### Task 4: Credential strategies (D12)

**Files:**
- Create: `clients/temper-rb/lib/temper/credentials.rb`
- Modify: `clients/temper-rb/lib/temper.rb` (require it)
- Test: `clients/temper-rb/spec/credentials_spec.rb`

**Interfaces:**
- Consumes: `Temper::Unauthorized` from Task 3.
- Produces: `Temper::Credentials::BearerToken.new(token)` and `Temper::Credentials::ClientCredentials.new(token_url:, client_id:, client_secret:, audience:, clock: -> { Time.now })`. Both respond to `#token -> String` and `#refresh! -> String`.

> **Why an injectable `clock:` rather than the `timecop` gem** — the cache is keyed on an *absolute* `expires_at` plus a 60s skew, exactly as `agent/lib/temper-auth.ts::mintM2mToken` does. Injecting the clock lets the spec advance time without a global monkey-patch, and keeps `timecop` out of the Gemfile.

> **Why a mutex** — the steward's cache is a bare module global, sound only because a serverless function is single-threaded. Under Puma, every in-flight thread races to mint at expiry.

- [ ] **Step 1: Write the failing spec**

`clients/temper-rb/spec/credentials_spec.rb`:
```ruby
# frozen_string_literal: true

RSpec.describe Temper::Credentials do
  describe Temper::Credentials::BearerToken do
    it 'returns the token it was constructed with, with no I/O' do
      expect(described_class.new('abc').token).to eq('abc')
    end

    it 'rejects an empty token at construction' do
      expect { described_class.new('') }.to raise_error(ArgumentError)
    end

    it 'cannot refresh and says so' do
      expect { described_class.new('abc').refresh! }.to raise_error(Temper::Unauthorized)
    end
  end

  describe Temper::Credentials::ClientCredentials do
    let(:token_url) { 'https://auth.test/oauth/token' }
    let(:now) { Time.at(1_000_000) }
    let(:clock) { -> { @now ||= now } }

    def creds(clock_fn = clock)
      described_class.new(token_url: token_url, client_id: 'cid', client_secret: 'sec',
                          audience: 'https://api.test', clock: clock_fn)
    end

    def stub_mint(token, expires_in: 3600)
      stub_request(:post, token_url)
        .to_return(status: 200, body: JSON.generate(access_token: token, expires_in: expires_in),
                   headers: { 'Content-Type' => 'application/json' })
    end

    it 'mints a token on first use and sends the four M2M fields' do
      stub_mint('tok-1')
      expect(creds.token).to eq('tok-1')
      expect(a_request(:post, token_url).with { |req|
        body = JSON.parse(req.body)
        body['grant_type'] == 'client_credentials' && body['client_id'] == 'cid' &&
          body['client_secret'] == 'sec' && body['audience'] == 'https://api.test'
      }).to have_been_made.once
    end

    it 'caches the token across calls' do
      stub_mint('tok-1')
      c = creds
      3.times { c.token }
      expect(a_request(:post, token_url)).to have_been_made.once
    end

    it 'refreshes 60s before the absolute expiry, not at it' do
      stub_mint('tok-1', expires_in: 3600)
      t = now
      c = creds(-> { t })
      expect(c.token).to eq('tok-1')

      stub_mint('tok-2')
      t = now + 3600 - 61   # 61s of headroom: still inside the skew window
      expect(c.token).to eq('tok-1')

      t = now + 3600 - 59   # 59s of headroom: inside the 60s skew, must re-mint
      expect(c.token).to eq('tok-2')
    end

    it 'raises Unauthorized when the mint is rejected' do
      stub_request(:post, token_url).to_return(status: 401, body: 'bad client')
      expect { creds.token }.to raise_error(Temper::Unauthorized, /401/)
    end

    it 'refresh! mints unconditionally, even on a warm cache' do
      stub_mint('tok-1')
      c = creds
      c.token
      stub_mint('tok-2')
      expect(c.refresh!).to eq('tok-2')
    end

    it 'requires every M2M field' do
      expect do
        described_class.new(token_url: token_url, client_id: '', client_secret: 's', audience: 'a')
      end.to raise_error(ArgumentError, /client_id/)
    end

    it 'mints once when many threads race an expired cache' do
      stub_mint('tok-1')
      c = creds
      threads = Array.new(8) { Thread.new { c.token } }
      expect(threads.map(&:value).uniq).to eq(['tok-1'])
      expect(a_request(:post, token_url)).to have_been_made.once
    end
  end
end
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cd clients/temper-rb && bundle exec rspec spec/credentials_spec.rb
```
Expected: FAIL — `uninitialized constant Temper::Credentials`.

- [ ] **Step 3: Implement the strategies**

`clients/temper-rb/lib/temper/credentials.rb`:
```ruby
# frozen_string_literal: true

require 'faraday'
require 'json'

module Temper
  module Credentials
    # A token the caller already holds (a Puma request serving a signed-in user).
    # No I/O, no refresh.
    class BearerToken
      def initialize(token)
        raise ArgumentError, 'token must be a non-empty String' unless token.is_a?(String) && !token.empty?

        @token = token
      end

      attr_reader :token

      def refresh!
        raise Unauthorized.new('BearerToken cannot refresh; mint a new token upstream', status: 401)
      end
    end

    # An Auth0 client_credentials machine principal (a Sidekiq worker).
    #
    # Ported from packages/agent-workflows/steward/agent/lib/temper-auth.ts --
    # the machine-principal caller already running in production. Same four
    # TEMPER_M2M_* inputs, same absolute-expires_at cache, same 60s skew.
    # Two deliberate divergences: the cache is mutex-guarded (Puma is threaded,
    # a serverless function is not), and #refresh! exists so a 401 taken
    # mid-job can re-mint rather than dead-letter.
    class ClientCredentials
      SKEW_SECONDS = 60

      def initialize(token_url:, client_id:, client_secret:, audience:, clock: -> { Time.now })
        @token_url = require_value(token_url, 'token_url')
        @client_id = require_value(client_id, 'client_id')
        @client_secret = require_value(client_secret, 'client_secret')
        @audience = require_value(audience, 'audience')
        @clock = clock
        @mutex = Mutex.new
        @token = nil
        @expires_at = nil
      end

      def token
        @mutex.synchronize do
          mint! if expired?
          @token
        end
      end

      def refresh!
        @mutex.synchronize { mint! }
      end

      private

      def require_value(value, name)
        raise ArgumentError, "#{name} must be a non-empty String" unless value.is_a?(String) && !value.empty?

        value
      end

      def expired?
        @token.nil? || @clock.call.to_f >= (@expires_at - SKEW_SECONDS)
      end

      def mint!
        response = Faraday.post(@token_url) do |req|
          req.headers['Content-Type'] = 'application/json'
          req.body = JSON.generate(
            grant_type: 'client_credentials',
            client_id: @client_id,
            client_secret: @client_secret,
            audience: @audience
          )
        end

        unless response.success?
          raise Unauthorized.new("token mint failed (#{response.status})", status: response.status,
                                                                          details: response.body)
        end

        body = JSON.parse(response.body)
        @token = body.fetch('access_token')
        @expires_at = @clock.call.to_f + body.fetch('expires_in').to_i
        @token
      end
    end
  end
end
```

- [ ] **Step 4: Require it and run the spec**

Add `require 'temper/credentials'` to `lib/temper.rb`.

```bash
cd clients/temper-rb && bundle exec rspec spec/credentials_spec.rb
```
Expected: PASS — 10 examples, 0 failures.

- [ ] **Step 5: Commit**

```bash
git add clients/temper-rb
git commit -m "temper-rb: BearerToken and ClientCredentials strategies

Ports the steward's mintM2mToken contract: four TEMPER_M2M_* inputs, absolute
expires_at, 60s skew. Adds a mutex (Puma is threaded) and refresh! (a 401 taken
mid-job must re-mint, not dead-letter)."
```

---

### Task 5: The process-global connection (D11, D12)

**Files:**
- Create: `clients/temper-rb/lib/temper/connection.rb`
- Modify: `clients/temper-rb/lib/temper.rb` (require it)
- Test: `clients/temper-rb/spec/connection_spec.rb`

**Interfaces:**
- Consumes: `Temper.config` (Task 1), `Temper::Generated::ApiClient` / `::Configuration` (Task 2).
- Produces: `Temper.api_client -> Temper::Generated::ApiClient` (memoized, process-global). `Temper.reset_connection! -> nil`. `Temper.current_token` / `Temper.with_token(token) { ... }` (fiber-local).

- [ ] **Step 1: Write the failing spec**

`clients/temper-rb/spec/connection_spec.rb`:
```ruby
# frozen_string_literal: true

RSpec.describe 'Temper connection' do
  before do
    Temper.reset_connection!
    Temper.configure { |c| c.base_url = 'https://api.test'; c.device_id = nil }
  end

  after { Temper.reset_connection! }

  it 'memoizes one ApiClient per process' do
    expect(Temper.api_client).to be(Temper.api_client)
  end

  it 'reset_connection! drops the memo so a forked worker builds fresh sockets' do
    first = Temper.api_client
    Temper.reset_connection!
    expect(Temper.api_client).not_to be(first)
  end

  it 'stamps X-Temper-Surface: sdk once, on the client' do
    expect(Temper.api_client.default_headers['X-Temper-Surface']).to eq('sdk')
  end

  it 'omits the device header when unconfigured' do
    expect(Temper.api_client.default_headers).not_to have_key('X-Temper-Device-Id')
  end

  it 'sends the device header when configured' do
    Temper.reset_connection!
    Temper.configure { |c| c.device_id = 'dev-1' }
    expect(Temper.api_client.default_headers['X-Temper-Device-Id']).to eq('dev-1')
  end

  it 'derives scheme, host, and base_path from base_url' do
    Temper.reset_connection!
    Temper.configure { |c| c.base_url = 'https://api.test:8443/prefix' }
    config = Temper.api_client.config
    expect(config.scheme).to eq('https')
    expect(config.host).to eq('api.test:8443')
    expect(config.base_path).to eq('/prefix')
  end

  # This is the whole point of D12: the connection is shared, the TOKEN is not.
  it 'resolves the access token per call, from fiber-local storage' do
    getter = Temper.api_client.config.access_token_getter
    Temper.with_token('tok-a') { expect(getter.call).to eq('tok-a') }
    Temper.with_token('tok-b') { expect(getter.call).to eq('tok-b') }
    expect(getter.call).to be_nil
  end

  it 'never leaks a token between threads' do
    seen = Queue.new
    Temper.with_token('outer') do
      Thread.new { seen << Temper.current_token }.join
    end
    expect(seen.pop).to be_nil
  end

  it 'restores the previous token after a nested scope' do
    Temper.with_token('outer') do
      Temper.with_token('inner') { nil }
      expect(Temper.current_token).to eq('outer')
    end
  end

  it 'never touches the generated singletons' do
    Temper.api_client
    expect(Temper::Generated::Configuration.default.access_token).to be_nil
  end
end
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cd clients/temper-rb && bundle exec rspec spec/connection_spec.rb
```
Expected: FAIL — `undefined method 'reset_connection!' for Temper`.

- [ ] **Step 3: Implement the connection**

`clients/temper-rb/lib/temper/connection.rb`:
```ruby
# frozen_string_literal: true

require 'faraday'
require 'faraday/net_http_persistent'
require 'uri'

module Temper
  TOKEN_KEY = :temper_access_token
  private_constant :TOKEN_KEY

  class << self
    # One ApiClient per process => one Faraday connection => one
    # net-http-persistent per-thread pool. A fresh client per request would pay
    # a TLS handshake per request; that is the pooling trap D12 exists to avoid.
    def api_client
      connection_mutex.synchronize { @api_client ||= build_api_client }
    end

    # Call from Puma's on_worker_boot and Sidekiq.configure_server, so a forked
    # worker never inherits its parent's sockets.
    def reset_connection!
      connection_mutex.synchronize { @api_client = nil }
      nil
    end

    # Thread.current[] is FIBER-local in Ruby (Thread#thread_variable_get is the
    # thread-local one). Fiber[] would be cleaner but arrived in 3.2, and the
    # gem's floor is 3.1.
    def current_token
      Thread.current[TOKEN_KEY]
    end

    def with_token(token)
      previous = Thread.current[TOKEN_KEY]
      Thread.current[TOKEN_KEY] = token
      yield
    ensure
      Thread.current[TOKEN_KEY] = previous
    end

    private

    def connection_mutex
      @connection_mutex ||= Mutex.new
    end

    def build_api_client
      base = URI.parse(config.base_url || raise(ArgumentError, 'Temper.config.base_url is not set'))

      generated = Generated::Configuration.new.tap do |c|
        c.scheme = base.scheme
        c.host = base.port && base.port != base.default_port ? "#{base.host}:#{base.port}" : base.host
        c.base_path = base.path
        c.access_token_getter = -> { Temper.current_token }

        # configure_connection runs AFTER the generated build_connection sets
        # Faraday.default_adapter, so this replaces it. configure_middleware
        # runs BEFORE, and would be silently overwritten.
        c.configure_connection { |conn| conn.adapter :net_http_persistent, pool_size: 5 }
      end

      Generated::ApiClient.new(generated).tap do |client|
        client.default_headers['X-Temper-Surface'] = 'sdk'
        client.default_headers['X-Temper-Device-Id'] = config.device_id if config.device_id
      end
    end
  end
end
```

- [ ] **Step 4: Require it and run the spec**

Add `require 'temper/connection'` to `lib/temper.rb` after `require 'temper/credentials'`.

```bash
cd clients/temper-rb && bundle exec rspec spec/connection_spec.rb
```
Expected: PASS — 10 examples, 0 failures.

- [ ] **Step 5: Prove the persistent adapter is actually installed**

A silent fallback to `net_http` would cost a TLS handshake per request and nothing would fail.

```bash
cd clients/temper-rb && bundle exec ruby -e '
  require "temper"
  Temper.configure { |c| c.base_url = "https://api.test" }
  conn = Temper.api_client.send(:build_connection) { }
  puts conn.builder.adapter.name
'
```
Expected: `Faraday::Adapter::NetHttpPersistent`.

- [ ] **Step 6: Commit**

```bash
git add clients/temper-rb
git commit -m "temper-rb: process-global connection, call-scoped token

configure_connection (not configure_middleware) is where the persistent adapter
must be swapped: the generated build_connection sets Faraday.default_adapter
between the two hooks."
```

---

### Task 6: `Temper::Act` and `Temper.parse_ref`

**Files:**
- Create: `clients/temper-rb/lib/temper/act.rb`
- Create: `clients/temper-rb/lib/temper/refs.rb`
- Modify: `clients/temper-rb/lib/temper.rb` (require both)
- Test: `clients/temper-rb/spec/act_spec.rb`, `clients/temper-rb/spec/refs_spec.rb`

**Interfaces:**
- Produces: `Temper::Act.new(confidence: nil, reasoning: nil, rationale: nil, persona: nil, model: nil, correlation: nil, invocation: nil)` with `#to_h -> Hash` (symbol keys, nils omitted, `correlation` → `:correlation_id`, `invocation` → `:invocation_id`). `Temper.parse_ref(String) -> String` (a UUID), raising `ArgumentError` otherwise.

> `Act`'s constructor invariant mirrors `ActInput::into_act_context`: `AgentAuthorship.confidence` is non-`Option` in Rust, so supplying any authorship field without `confidence` earns a 400. Rejecting it locally is the parse-don't-validate answer (D14).

> `parse_ref` is a port of `temper_workflow::operations::refs.rs:94` — bare UUID, or trailing-UUID-only from the last five hyphen groups. The gem does **not** port `sluggify`; the server derives the slug from the title.

- [ ] **Step 1: Write the failing act spec**

`clients/temper-rb/spec/act_spec.rb`:
```ruby
# frozen_string_literal: true

RSpec.describe Temper::Act do
  it 'accepts confidence alone' do
    expect(described_class.new(confidence: :probable).to_h).to eq(confidence: 'probable')
  end

  it 'accepts an empty act' do
    expect(described_class.new.to_h).to eq({})
  end

  %i[reasoning rationale persona model].each do |field|
    it "rejects #{field} without confidence, locally, rather than earning a 400" do
      expect { described_class.new(field => 'x') }
        .to raise_error(ArgumentError, /confidence/)
    end

    it "accepts #{field} alongside confidence" do
      act = described_class.new(:confidence => :certain, field => 'x')
      expect(act.to_h[field]).to eq('x')
    end
  end

  it 'renames correlation and invocation to their wire keys' do
    act = described_class.new(correlation: 'c-1', invocation: 'i-1')
    expect(act.to_h).to eq(correlation_id: 'c-1', invocation_id: 'i-1')
  end

  it 'omits nils entirely so the server sees an absent key, not null' do
    expect(described_class.new(confidence: :probable).to_h.keys).to eq([:confidence])
  end

  it 'stringifies a symbol confidence' do
    expect(described_class.new(confidence: :speculative).to_h[:confidence]).to eq('speculative')
  end

  it 'permits correlation with no confidence -- correlation is provenance, not authorship' do
    expect { described_class.new(correlation: 'c-1') }.not_to raise_error
  end
end
```

- [ ] **Step 2: Write the failing refs spec**

`clients/temper-rb/spec/refs_spec.rb`:
```ruby
# frozen_string_literal: true

RSpec.describe '.parse_ref' do
  let(:uuid) { '019f4912-3f20-7fd3-814f-13a5ddbe3cd7' }

  it 'accepts a bare UUID' do
    expect(Temper.parse_ref(uuid)).to eq(uuid)
  end

  it 'accepts a decorated ref and ignores the slug half' do
    expect(Temper.parse_ref("p4-design-the-temper-rb-gem-#{uuid}")).to eq(uuid)
  end

  it 'ignores a stale slug half entirely' do
    expect(Temper.parse_ref("totally-wrong-slug-#{uuid}")).to eq(uuid)
  end

  it 'accepts a slug whose words look like hex' do
    expect(Temper.parse_ref("beef-cafe-#{uuid}")).to eq(uuid)
  end

  it 'rejects a slug with no trailing UUID' do
    expect { Temper.parse_ref('just-a-slug') }.to raise_error(ArgumentError, /not a ref/)
  end

  it 'rejects an empty string' do
    expect { Temper.parse_ref('') }.to raise_error(ArgumentError)
  end

  it 'rejects a truncated UUID' do
    expect { Temper.parse_ref('slug-019f4912-3f20-7fd3-814f') }.to raise_error(ArgumentError)
  end
end
```

- [ ] **Step 3: Run both to verify they fail**

```bash
cd clients/temper-rb && bundle exec rspec spec/act_spec.rb spec/refs_spec.rb
```
Expected: FAIL — `uninitialized constant Temper::Act`.

- [ ] **Step 4: Implement `Act`**

`clients/temper-rb/lib/temper/act.rb`:
```ruby
# frozen_string_literal: true

module Temper
  # Act context for a write. Optional on every call.
  #
  # The constructor invariant mirrors ActInput::into_act_context: Rust's
  # AgentAuthorship.confidence is non-Option, so authorship without confidence
  # is a 400. We reject it here instead of paying a round trip.
  #
  # `correlation` is provenance, never authorization, and an act with no
  # correlation self-roots to its own event id -- so it is exempt.
  class Act
    AUTHORSHIP_FIELDS = %i[reasoning rationale persona model].freeze

    def initialize(confidence: nil, reasoning: nil, rationale: nil, persona: nil, model: nil,
                   correlation: nil, invocation: nil)
      authorship = { reasoning: reasoning, rationale: rationale, persona: persona, model: model }

      if confidence.nil? && authorship.any? { |_, v| !v.nil? }
        supplied = authorship.reject { |_, v| v.nil? }.keys.join(', ')
        raise ArgumentError, "Temper::Act requires `confidence:` when supplying #{supplied}"
      end

      @fields = authorship.merge(
        confidence: confidence && confidence.to_s,
        correlation_id: correlation,
        invocation_id: invocation
      ).reject { |_, v| v.nil? }.freeze
    end

    # Symbol-keyed, nils omitted. The server distinguishes an absent key from null.
    def to_h
      @fields.dup
    end
  end
end
```

- [ ] **Step 5: Implement `parse_ref`**

`clients/temper-rb/lib/temper/refs.rb`:
```ruby
# frozen_string_literal: true

module Temper
  # Trailing-UUID-only resolution, matching temper_workflow::operations::parse_ref.
  # A ref is either a bare UUID or `<slug>-<uuid>`; the slug half is presentation
  # and a stale one is harmless. There is no by-slug lookup, so this never hits
  # the network.
  UUID_PATTERN = /\A\h{8}-\h{4}-\h{4}-\h{4}-\h{12}\z/
  private_constant :UUID_PATTERN

  def self.parse_ref(ref)
    raise ArgumentError, 'ref must be a String' unless ref.is_a?(String)

    return ref if ref.match?(UUID_PATTERN)

    # The UUID is the last five hyphen-separated groups.
    candidate = ref.split('-').last(5).join('-')
    return candidate if candidate.match?(UUID_PATTERN)

    raise ArgumentError, "not a ref (expected a UUID or `<slug>-<uuid>`): #{ref.inspect}"
  end
end
```

- [ ] **Step 6: Require both and run the specs**

Add `require 'temper/act'` and `require 'temper/refs'` to `lib/temper.rb`.

```bash
cd clients/temper-rb && bundle exec rspec spec/act_spec.rb spec/refs_spec.rb
```
Expected: PASS — 21 examples, 0 failures (`act_spec` is 14: the `each` loop over the four authorship fields expands to 8; `refs_spec` is 7).

- [ ] **Step 7: Commit**

```bash
git add clients/temper-rb
git commit -m "temper-rb: Act value object and parse_ref

Act rejects authorship-without-confidence locally rather than earning a 400.
correlation is exempt: it is provenance, never authorship."
```

---

### Task 7: The client façade — credential scoping, 401 re-mint, read retry

This is where D12's "re-mint-*on*-401, not merely refresh-before-expiry" lands. The steward proves refresh-ahead is insufficient: its schedules resolve a token once per tick, so a tick outliving its cached token takes an unrecoverable 401. A Sidekiq job holding a token across a long unit of work has exactly this bug.

**Files:**
- Create: `clients/temper-rb/lib/temper/client.rb`
- Modify: `clients/temper-rb/lib/temper.rb` (require it)
- Test: `clients/temper-rb/spec/client_spec.rb`

**Interfaces:**
- Consumes: everything from Tasks 3–6.
- Produces: `Temper::Client.new(credentials:, backoff: DEFAULT_BACKOFF)`. `Client#whoami -> Hash` (`GET /api/profile` declares no response schema, so the generated client deserializes it untyped). `Client#call(idempotent: false) { |api_client| ... }` — the internal seam every surface module goes through. `Client#resources`, `#contexts`, `#cognitive_maps` (defined in Tasks 8–9).

> **Retry policy, mirroring `should_retry` in `crates/temper-client/src/http.rs:52`:** idempotent reads (GET/HEAD) retry on 5xx and transport failure, 3 attempts at 200ms/400ms. **Writes never auto-retry.** The 401 re-mint is *not* a retry policy — it is a single credential repair, and it applies to reads and writes alike because re-authenticating is not re-submitting.

- [ ] **Step 1: Write the failing spec**

`clients/temper-rb/spec/client_spec.rb`:
```ruby
# frozen_string_literal: true

RSpec.describe Temper::Client do
  before do
    Temper.reset_connection!
    Temper.configure { |c| c.base_url = 'https://api.test' }
  end

  after { Temper.reset_connection! }

  let(:bearer) { Temper::Credentials::BearerToken.new('tok-1') }

  it 'scopes the credential token around the call' do
    stub_request(:get, 'https://api.test/api/profile')
      .with(headers: { 'Authorization' => 'Bearer tok-1', 'X-Temper-Surface' => 'sdk' })
      .to_return(status: 200, body: '{"id":"p1","handle":"dana"}',
                 headers: { 'Content-Type' => 'application/json' })

    described_class.new(credentials: bearer).whoami
    expect(a_request(:get, 'https://api.test/api/profile')).to have_been_made.once
  end

  it 'clears the fiber-local token after the call' do
    stub_request(:get, 'https://api.test/api/profile').to_return(status: 200, body: '{}')
    described_class.new(credentials: bearer).whoami
    expect(Temper.current_token).to be_nil
  end

  it 'translates a 403 SYSTEM_ACCESS_REQUIRED into the named exception' do
    stub_request(:get, 'https://api.test/api/profile').to_return(
      status: 403,
      body: JSON.generate(error: { code: 'SYSTEM_ACCESS_REQUIRED', message: 'grant the agent cogmap write' })
    )
    expect { described_class.new(credentials: bearer).whoami }
      .to raise_error(Temper::SystemAccessRequired, /grant the agent cogmap write/)
  end

  it 'raises Unauthorized immediately for a BearerToken on 401 -- it cannot refresh' do
    stub_request(:get, 'https://api.test/api/profile').to_return(status: 401, body: '{}')
    expect { described_class.new(credentials: bearer).whoami }.to raise_error(Temper::Unauthorized)
    expect(a_request(:get, 'https://api.test/api/profile')).to have_been_made.once
  end

  it 're-mints once and retries when ClientCredentials takes a 401 mid-job' do
    stub_request(:post, 'https://auth.test/token')
      .to_return(status: 200, body: JSON.generate(access_token: 'tok-a', expires_in: 3600))
      .then.to_return(status: 200, body: JSON.generate(access_token: 'tok-b', expires_in: 3600))

    stub_request(:get, 'https://api.test/api/profile')
      .with(headers: { 'Authorization' => 'Bearer tok-a' })
      .to_return(status: 401, body: '{}')
    stub_request(:get, 'https://api.test/api/profile')
      .with(headers: { 'Authorization' => 'Bearer tok-b' })
      .to_return(status: 200, body: '{"id":"p1"}', headers: { 'Content-Type' => 'application/json' })

    creds = Temper::Credentials::ClientCredentials.new(
      token_url: 'https://auth.test/token', client_id: 'c', client_secret: 's', audience: 'a'
    )
    expect { described_class.new(credentials: creds).whoami }.not_to raise_error
  end

  it 'gives up after a single re-mint, rather than looping' do
    stub_request(:post, 'https://auth.test/token')
      .to_return(status: 200, body: JSON.generate(access_token: 'tok-a', expires_in: 3600))
    stub_request(:get, 'https://api.test/api/profile').to_return(status: 401, body: '{}')

    creds = Temper::Credentials::ClientCredentials.new(
      token_url: 'https://auth.test/token', client_id: 'c', client_secret: 's', audience: 'a'
    )
    expect { described_class.new(credentials: creds).whoami }.to raise_error(Temper::Unauthorized)
    expect(a_request(:get, 'https://api.test/api/profile')).to have_been_made.twice
  end

  it 'retries an idempotent read on 5xx, three attempts' do
    stub_request(:get, 'https://api.test/api/profile')
      .to_return(status: 503, body: 'down').times(2)
      .then.to_return(status: 200, body: '{"id":"p1"}', headers: { 'Content-Type' => 'application/json' })

    client = described_class.new(credentials: bearer, backoff: ->(_attempt) { nil })
    expect { client.whoami }.not_to raise_error
    expect(a_request(:get, 'https://api.test/api/profile')).to have_been_made.times(3)
  end

  it 'raises ServerError after exhausting read retries' do
    stub_request(:get, 'https://api.test/api/profile').to_return(status: 503, body: 'down')
    client = described_class.new(credentials: bearer, backoff: ->(_attempt) { nil })
    expect { client.whoami }.to raise_error(Temper::ServerError)
    expect(a_request(:get, 'https://api.test/api/profile')).to have_been_made.times(3)
  end

  it 'never auto-retries a write, even on 503' do
    stub_request(:post, 'https://api.test/api/ingest').to_return(status: 503, body: 'down')
    client = described_class.new(credentials: bearer, backoff: ->(_attempt) { nil })
    expect do
      client.call { |api| api.call_api(:POST, '/api/ingest', return_type: nil) }
    end.to raise_error(Temper::ServerError)
    expect(a_request(:post, 'https://api.test/api/ingest')).to have_been_made.once
  end
end
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cd clients/temper-rb && bundle exec rspec spec/client_spec.rb
```
Expected: FAIL — `uninitialized constant Temper::Client`.

- [ ] **Step 3: Implement the façade**

`clients/temper-rb/lib/temper/client.rb`:
```ruby
# frozen_string_literal: true

module Temper
  # A cheap per-request façade. Holds a credential and a reference to the shared
  # process-global ApiClient. Constructing one does no I/O.
  class Client
    MAX_READ_ATTEMPTS = 3
    DEFAULT_BACKOFF = ->(attempt) { sleep(0.2 * (2**(attempt - 1))) }

    def initialize(credentials:, backoff: DEFAULT_BACKOFF)
      @credentials = credentials
      @backoff = backoff
    end

    # Assert the machine profile resolved, and report what it can reach, rather
    # than discovering it on the first write. Authentication is not authorization:
    # a minted M2M token yields a JIT-provisioned agent profile and nothing else.
    def whoami
      call(idempotent: true) { |api| Generated::ProfileApi.new(api).get_profile }
    end

    # The one seam every surface module goes through.
    #
    # - `idempotent: true`  => 5xx and transport failures retry (GET/HEAD only).
    # - `idempotent: false` => a write. Never auto-retried.
    #
    # A 401 is repaired once, for reads and writes alike: re-authenticating is
    # not re-submitting.
    def call(idempotent: false)
      attempt = 0
      reminted = false

      begin
        attempt += 1
        Temper.with_token(@credentials.token) { yield(Temper.api_client) }
      rescue Generated::ApiError => e
        error = ErrorMapper.call(e)

        if error.is_a?(Unauthorized) && !reminted
          reminted = true
          @credentials.refresh!
          retry
        end

        if idempotent && error.is_a?(TransientError) && attempt < MAX_READ_ATTEMPTS
          @backoff.call(attempt)
          retry
        end

        raise error
      end
    end
  end
end
```

> `BearerToken#refresh!` raises `Unauthorized`, so the `reminted` branch is a no-op escape for bearer callers: the raise propagates out of `refresh!` itself, and the request is made exactly once.

- [ ] **Step 4: Require it and run the spec**

Add `require 'temper/client'` to `lib/temper.rb`.

```bash
cd clients/temper-rb && bundle exec rspec spec/client_spec.rb
```
Expected: PASS — 9 examples, 0 failures.

- [ ] **Step 5: Run the whole suite and rubocop**

```bash
cd clients/temper-rb && bundle exec rake
```
Expected: rubocop clean, all specs pass.

- [ ] **Step 6: Commit**

```bash
git add clients/temper-rb
git commit -m "temper-rb: client facade with 401 re-mint and read-only retry

Re-mint-on-401 applies to writes too: re-authenticating is not re-submitting.
Retry-on-5xx does not -- writes are never auto-retried, mirroring the Rust client."
```

---

### Task 8: The resource surface

**Files:**
- Create: `clients/temper-rb/lib/temper/resources.rb`
- Modify: `clients/temper-rb/lib/temper/client.rb` (add `#resources`)
- Test: `clients/temper-rb/spec/resources_spec.rb`

**Interfaces:**
- Consumes: `Client#call`, `Act#to_h`, `Temper.parse_ref`.
- Produces: `Client#resources -> Temper::Resources`, with `#create(title:, context_ref:, doc_type_name:, content:, origin_uri: '', act: nil, **opts)`, `#show(ref, meta_only: false)`, `#edges(ref)`, `#update(ref, act: nil, **fields)`, `#delete(ref, act: nil)`, `#list(**filters)`. All return generated model instances (D14).

> **`show` has no `edges:` option**, because `GET /api/resources/{id}` has no such query parameter — its only parameter is the path `id`. Edges come from a separate operation, `list_resource_edges`. The CLI's `--edges` flag is a CLI-side composition, not a server one.

> **The act carriage rule, and the one exception.** ~30 write endpoints accept act context via `#[serde(flatten)] pub act: ActInput`, so the seven keys ride as top-level body fields — and because the contract expresses `IngestPayload` as `allOf: [ActInput, {…}]`, the generated `IngestPayload` model already carries them as plain attributes. `DELETE /api/resources/{id}` is different: it takes `Query<ActInput>`, so the same seven keys ride the **query string**. Getting this backwards yields a silent loss of provenance, not an error.

- [ ] **Step 1: Write the failing spec**

`clients/temper-rb/spec/resources_spec.rb`:
```ruby
# frozen_string_literal: true

RSpec.describe Temper::Resources do
  before do
    Temper.reset_connection!
    Temper.configure { |c| c.base_url = 'https://api.test' }
  end

  after { Temper.reset_connection! }

  let(:client) { Temper::Client.new(credentials: Temper::Credentials::BearerToken.new('tok')) }
  let(:uuid) { '019f4912-3f20-7fd3-814f-13a5ddbe3cd7' }
  let(:act) { Temper::Act.new(confidence: :probable, reasoning: 'because', correlation: 'corr-1') }

  it 'flattens the act keys into the ingest body' do
    stub_request(:post, 'https://api.test/api/ingest')
      .with { |req|
        body = JSON.parse(req.body)
        body['confidence'] == 'probable' && body['reasoning'] == 'because' &&
          body['correlation_id'] == 'corr-1' && body['title'] == 'Postmortem'
      }
      .to_return(status: 200, body: '{"id":"' + uuid + '"}', headers: { 'Content-Type' => 'application/json' })

    client.resources.create(title: 'Postmortem', context_ref: '@dana/incidents',
                            doc_type_name: 'note', content: '# hi', act: act)
    expect(a_request(:post, 'https://api.test/api/ingest')).to have_been_made.once
  end

  it 'never sends chunks_packed or content_hash -- the server computes them' do
    stub_request(:post, 'https://api.test/api/ingest')
      .with { |req| !JSON.parse(req.body).key?('chunks_packed') && !JSON.parse(req.body).key?('content_hash') }
      .to_return(status: 200, body: '{}', headers: { 'Content-Type' => 'application/json' })

    client.resources.create(title: 'T', context_ref: '@d/c', doc_type_name: 'note', content: 'x')
    expect(a_request(:post, 'https://api.test/api/ingest')).to have_been_made.once
  end

  # DELETE takes Query<ActInput>, not a body. Getting this backwards silently
  # drops provenance instead of erroring.
  it 'routes the act keys onto the query string for delete' do
    stub_request(:delete, "https://api.test/api/resources/#{uuid}")
      .with(query: hash_including('confidence' => 'probable', 'correlation_id' => 'corr-1'))
      .to_return(status: 200, body: '{}', headers: { 'Content-Type' => 'application/json' })

    client.resources.delete("some-slug-#{uuid}", act: act)
    expect(a_request(:delete, "https://api.test/api/resources/#{uuid}")).to have_been_made.once
  end

  it 'resolves a decorated ref to its trailing UUID before addressing' do
    stub_request(:get, "https://api.test/api/resources/#{uuid}")
      .to_return(status: 200, body: '{}', headers: { 'Content-Type' => 'application/json' })

    client.resources.show("stale-wrong-slug-#{uuid}")
    expect(a_request(:get, "https://api.test/api/resources/#{uuid}")).to have_been_made.once
  end

  it 'rejects a ref with no trailing UUID before making a request' do
    expect { client.resources.show('just-a-slug') }.to raise_error(ArgumentError)
    expect(a_request(:get, %r{https://api\.test/api/resources/.*})).not_to have_been_made
  end

  it 'reads the meta projection without the body' do
    stub_request(:get, "https://api.test/api/resources/#{uuid}/meta")
      .to_return(status: 200, body: '{}', headers: { 'Content-Type' => 'application/json' })

    client.resources.show(uuid, meta_only: true)
    expect(a_request(:get, "https://api.test/api/resources/#{uuid}/meta")).to have_been_made.once
  end

  # GET /api/resources/{id} has no query parameters, so edges is its own operation.
  it 'reads edges through the dedicated operation, not a flag on show' do
    stub_request(:get, "https://api.test/api/resources/#{uuid}/edges")
      .to_return(status: 200, body: '[]', headers: { 'Content-Type' => 'application/json' })

    client.resources.edges(uuid)
    expect(a_request(:get, "https://api.test/api/resources/#{uuid}/edges")).to have_been_made.once
  end
end
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cd clients/temper-rb && bundle exec rspec spec/resources_spec.rb
```
Expected: FAIL — `uninitialized constant Temper::Resources`.

- [ ] **Step 3: Implement the surface**

`clients/temper-rb/lib/temper/resources.rb`:
```ruby
# frozen_string_literal: true

module Temper
  class Resources
    def initialize(client)
      @client = client
    end

    # POST /api/ingest. The seven act keys ride as top-level body fields
    # (`#[serde(flatten)] pub act: ActInput`), which the generated IngestPayload
    # already models as plain attributes.
    #
    # chunks_packed and content_hash are deliberately omitted: both are Option on
    # IngestPayload and the server computes them. Ruby never embeds (D9).
    def create(title:, context_ref:, doc_type_name:, content:, origin_uri: '', act: nil, **opts)
      payload = Generated::IngestPayload.new(
        **{ title: title, context_ref: context_ref, doc_type_name: doc_type_name,
            content: content, origin_uri: origin_uri }.merge(opts).merge(act&.to_h || {})
      )
      @client.call { |api| Generated::IngestApi.new(api).create_ingest(payload) }
    end

    # GET /api/resources/{id} takes no query parameters at all, so `edges` is a
    # separate operation rather than a flag. `meta_only` is a different endpoint.
    def show(ref, meta_only: false)
      id = Temper.parse_ref(ref)
      @client.call(idempotent: true) do |api|
        next Generated::MetaApi.new(api).get_meta(id) if meta_only

        Generated::ResourcesApi.new(api).get_resource(id)
      end
    end

    def edges(ref)
      id = Temper.parse_ref(ref)
      @client.call(idempotent: true) { |api| Generated::ResourcesApi.new(api).list_resource_edges(id) }
    end

    def update(ref, act: nil, **fields)
      id = Temper.parse_ref(ref)
      body = Generated::ResourceUpdateRequest.new(**fields.merge(act&.to_h || {}))
      @client.call { |api| Generated::ResourcesApi.new(api).update_resource(id, body) }
    end

    # DELETE /api/resources/{id} takes Query<ActInput>: the same seven keys, on
    # the query string rather than in a body.
    def delete(ref, act: nil)
      id = Temper.parse_ref(ref)
      @client.call { |api| Generated::ResourcesApi.new(api).delete_resource(id, act&.to_h || {}) }
    end

    def list(**filters)
      @client.call(idempotent: true) { |api| Generated::ResourcesApi.new(api).list_resources(filters) }
    end
  end
end
```

Add to `clients/temper-rb/lib/temper/client.rb`, inside `class Client`:
```ruby
    def resources
      @resources ||= Resources.new(self)
    end
```

- [ ] **Step 4: Require it and run the spec**

Add `require 'temper/resources'` to `lib/temper.rb` after `require 'temper/client'`.

```bash
cd clients/temper-rb && bundle exec rspec spec/resources_spec.rb
```
Expected: PASS — 7 examples, 0 failures.

- [ ] **Step 5: Commit**

```bash
git add clients/temper-rb
git commit -m "temper-rb: resource surface

Act keys flatten into the ingest body but ride the query string on delete --
DELETE /api/resources/{id} takes Query<ActInput>."
```

---

### Task 9: Contexts, search, graph, and incremental cogmap authoring (D9)

**Files:**
- Create: `clients/temper-rb/lib/temper/contexts.rb`
- Create: `clients/temper-rb/lib/temper/cognitive_maps.rb`
- Modify: `clients/temper-rb/lib/temper/client.rb` (add `#contexts`, `#cognitive_maps`, `#search`, `#graph`)
- Test: `clients/temper-rb/spec/contexts_spec.rb`, `clients/temper-rb/spec/cognitive_maps_spec.rb`

**Interfaces:**
- Produces: `Client#contexts` (`#create(name:, owner:)`, `#list`, `#show(ref)`), `Client#cognitive_maps` (`#create(name:, telos: nil, ...)`, `#author(cogmap_ref, title:, content:, context_ref:, doc_type_name:, ...)`, `#assert_relationship(source:, target:, edge_kind:, ...)`, `#set_facet(resource:, values:, ...)`), `Client#search(query, **opts)`, `Client#graph`.

> **D9 is a hard boundary.** The gem authors into a map through **server-recompute paths only**: ingest with `home_cogmap_id`, `relationship assert`, `facet set`, and genesis with `telos: nil`. `PUT /api/cognitive-maps/{id}` (bulk reconcile) requires a pre-embedded `chunks_packed` — `ReconcileEntry.chunks_packed` is a required `String`, unpacked verbatim with "NO server-side ONNX." A Ruby client cannot physically reach it without a 768-dim BGE embedder. `Generated::CognitiveMapsApi#reconcile` stays reachable for anyone hand-packing chunks; **the skin must not surface it**, and the README says why.

> `ContextOwnerRef` generates as a `module` with `openapi_one_of` and no discriminator — the generated code calls its own `build` best-effort. The skin hand-constructs that payload as a plain Hash rather than routing through the wrapper.

- [ ] **Step 1: Write the failing cogmap spec**

`clients/temper-rb/spec/cognitive_maps_spec.rb`:
```ruby
# frozen_string_literal: true

RSpec.describe Temper::CognitiveMaps do
  before do
    Temper.reset_connection!
    Temper.configure { |c| c.base_url = 'https://api.test' }
  end

  after { Temper.reset_connection! }

  let(:client) { Temper::Client.new(credentials: Temper::Credentials::BearerToken.new('tok')) }
  let(:cogmap_id) { '00000000-0000-0000-0005-000000000001' }

  it 'creates a charter-less map -- genesis takes an Option<ReconcileTelos>' do
    stub_request(:post, 'https://api.test/api/cognitive-maps')
      .with { |req| !JSON.parse(req.body).fetch('telos', nil) }
      .to_return(status: 200, body: '{}', headers: { 'Content-Type' => 'application/json' })

    client.cognitive_maps.create(name: 'team-self')
    expect(a_request(:post, 'https://api.test/api/cognitive-maps')).to have_been_made.once
  end

  it 'authors into a map via ingest with home_cogmap_id -- the server embeds' do
    stub_request(:post, 'https://api.test/api/ingest')
      .with { |req|
        body = JSON.parse(req.body)
        body['home_cogmap_id'] == cogmap_id && !body.key?('chunks_packed')
      }
      .to_return(status: 200, body: '{}', headers: { 'Content-Type' => 'application/json' })

    client.cognitive_maps.author(cogmap_id, title: 'Node', content: '# n',
                                 context_ref: '@d/c', doc_type_name: 'note')
    expect(a_request(:post, 'https://api.test/api/ingest')).to have_been_made.once
  end

  # POST /api/relationships -- NOT /api/relationships/assert, which the design
  # spec names and which does not exist.
  it 'asserts an edge against /api/relationships with flattened act keys' do
    other = '019f4bdc-4cd2-73f2-a866-4cc29606de66'
    stub_request(:post, 'https://api.test/api/relationships')
      .with { |req|
        body = JSON.parse(req.body)
        body['source'] == cogmap_id && body['target'] == other &&
          body['edge_kind'] == 'advances' && body['confidence'] == 'probable'
      }
      .to_return(status: 200, body: '{}', headers: { 'Content-Type' => 'application/json' })

    client.cognitive_maps.assert_relationship(
      source: cogmap_id, target: other, edge_kind: 'advances',
      act: Temper::Act.new(confidence: :probable)
    )
    expect(a_request(:post, 'https://api.test/api/relationships')).to have_been_made.once
  end

  # POST /api/facets -- NOT /api/facets/set. `values` is plural.
  it 'sets a facet against /api/facets' do
    stub_request(:post, 'https://api.test/api/facets')
      .with { |req| JSON.parse(req.body)['values'] == { 'tier' => 'core' } }
      .to_return(status: 200, body: '{}', headers: { 'Content-Type' => 'application/json' })

    client.cognitive_maps.set_facet(resource: cogmap_id, values: { 'tier' => 'core' })
    expect(a_request(:post, 'https://api.test/api/facets')).to have_been_made.once
  end

  # D9: reconcile requires a client-side 768-dim BGE embedder. Ruby has none,
  # and never will. The generated method stays reachable; the skin does not
  # surface it, and this test is the guard against someone adding it.
  it 'does not surface bulk reconcile' do
    expect(client.cognitive_maps).not_to respond_to(:reconcile)
  end
end
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cd clients/temper-rb && bundle exec rspec spec/cognitive_maps_spec.rb
```
Expected: FAIL — `uninitialized constant Temper::CognitiveMaps`.

- [ ] **Step 3: Implement the cogmap surface**

`clients/temper-rb/lib/temper/cognitive_maps.rb`:
```ruby
# frozen_string_literal: true

module Temper
  # Incremental cognitive-map authoring (D9).
  #
  # Every path here is server-recompute: the server chunks and embeds. Bulk
  # reconcile (PUT /api/cognitive-maps/{id}) is deliberately absent -- its
  # ReconcileEntry.chunks_packed is a required, pre-embedded String, carried
  # verbatim with no server-side ONNX. It is a CLI operator's job, not a Rails
  # request's. Reach Generated::CognitiveMapsApi directly if you hand-pack chunks.
  class CognitiveMaps
    def initialize(client)
      @client = client
    end

    # POST /api/cognitive-maps. Genesis takes an Option<ReconcileTelos>, so a
    # charter-less map is creatable. CreateCogmapRequest carries NO act keys.
    def create(name:, telos: nil, cogmap_id: nil, telos_resource_id: nil, telos_title: nil)
      body = Generated::CreateCogmapRequest.new(
        name: name, telos: telos, cogmap_id: cogmap_id,
        telos_resource_id: telos_resource_id, telos_title: telos_title
      )
      @client.call { |api| Generated::CognitiveMapsApi.new(api).genesis(body) }
    end

    def author(cogmap_ref, title:, content:, context_ref:, doc_type_name:, act: nil, **opts)
      @client.resources.create(
        title: title, context_ref: context_ref, doc_type_name: doc_type_name,
        content: content, act: act, home_cogmap_id: Temper.parse_ref(cogmap_ref), **opts
      )
    end

    # POST /api/relationships (not /api/relationships/assert). The seven act keys
    # flatten onto AssertRelationshipRequest, as they do on IngestPayload.
    def assert_relationship(source:, target:, edge_kind:, act: nil, **opts)
      body = Generated::AssertRelationshipRequest.new(
        **{ source: Temper.parse_ref(source), target: Temper.parse_ref(target),
            edge_kind: edge_kind }.merge(opts).merge(act&.to_h || {})
      )
      @client.call { |api| Generated::RelationshipsApi.new(api).assert(body) }
    end

    # POST /api/facets (not /api/facets/set). `values` is plural.
    def set_facet(resource:, values:, weight: nil, act: nil)
      body = Generated::FacetSetRequest.new(
        **{ resource: Temper.parse_ref(resource), values: values, weight: weight }
          .compact.merge(act&.to_h || {})
      )
      @client.call { |api| Generated::FacetsApi.new(api).set_facet(body) }
    end
  end
end
```

- [ ] **Step 4: Write the failing contexts spec**

`clients/temper-rb/spec/contexts_spec.rb`:
```ruby
# frozen_string_literal: true

RSpec.describe Temper::Contexts do
  before do
    Temper.reset_connection!
    Temper.configure { |c| c.base_url = 'https://api.test' }
  end

  after { Temper.reset_connection! }

  let(:client) { Temper::Client.new(credentials: Temper::Credentials::BearerToken.new('tok')) }

  # ContextOwnerRef is a discriminator-less string-or-object oneOf; the generated
  # `build` wrapper is best-effort by its own admission. Hand-construct instead.
  it 'hand-constructs the owner ref payload for a personal context' do
    stub_request(:post, 'https://api.test/api/contexts')
      .with { |req| JSON.parse(req.body)['owner'] == 'Me' }
      .to_return(status: 201, body: '{}', headers: { 'Content-Type' => 'application/json' })

    client.contexts.create(name: 'incidents', owner: :me)
    expect(a_request(:post, 'https://api.test/api/contexts')).to have_been_made.once
  end

  it 'hand-constructs the owner ref payload for a team context' do
    stub_request(:post, 'https://api.test/api/contexts')
      .with { |req| JSON.parse(req.body)['owner'] == { 'Team' => 'acme' } }
      .to_return(status: 201, body: '{}', headers: { 'Content-Type' => 'application/json' })

    client.contexts.create(name: 'incidents', owner: { team: 'acme' })
    expect(a_request(:post, 'https://api.test/api/contexts')).to have_been_made.once
  end

  it 'lists contexts as an idempotent read' do
    stub_request(:get, 'https://api.test/api/contexts')
      .to_return(status: 200, body: '[]', headers: { 'Content-Type' => 'application/json' })
    client.contexts.list
    expect(a_request(:get, 'https://api.test/api/contexts')).to have_been_made.once
  end
end
```

- [ ] **Step 5: Implement contexts, search, and graph**

`clients/temper-rb/lib/temper/contexts.rb`:
```ruby
# frozen_string_literal: true

module Temper
  class Contexts
    def initialize(client)
      @client = client
    end

    # POST /api/contexts. ContextCreateRequest's fields are `name` and `owner`
    # (there is no `slug` on the wire -- the server derives it).
    #
    # `owner` is :me, {team: "slug"}, or {profile: "handle"}. ContextOwnerRef is
    # an externally-tagged serde enum mixing a unit variant with newtype
    # variants, so it has no discriminator and the generated oneOf wrapper is
    # approximate by its own admission. We hand-build the wire shape and let the
    # generated model carry it through: `to_hash` passes non-model values verbatim.
    def create(name:, owner:)
      body = Generated::ContextCreateRequest.new(name: name, owner: owner_ref(owner))
      @client.call { |api| Generated::ContextsApi.new(api).create_context(body) }
    end

    def list(**filters)
      @client.call(idempotent: true) { |api| Generated::ContextsApi.new(api).list_contexts(filters) }
    end

    def show(ref)
      @client.call(idempotent: true) { |api| Generated::ContextsApi.new(api).get_context(Temper.parse_ref(ref)) }
    end

    private

    def owner_ref(owner)
      case owner
      when :me, 'me' then 'Me'
      when Hash
        return { 'Team' => owner.fetch(:team) } if owner.key?(:team)
        return { 'Profile' => owner.fetch(:profile) } if owner.key?(:profile)

        raise ArgumentError, "owner Hash must carry :team or :profile, got #{owner.keys.inspect}"
      else
        raise ArgumentError, "owner must be :me, {team:}, or {profile:}, got #{owner.inspect}"
      end
    end
  end
end
```

Add to `clients/temper-rb/lib/temper/client.rb`, inside `class Client`:
```ruby
    def contexts
      @contexts ||= Contexts.new(self)
    end

    def cognitive_maps
      @cognitive_maps ||= CognitiveMaps.new(self)
    end

    # SearchParams names the field `query`, not `q`.
    def search(query, **opts)
      params = Generated::SearchParams.new(query: query, **opts)
      call(idempotent: true) { |api| Generated::SearchApi.new(api).search(params) }
    end

    def graph
      @graph ||= Generated::GraphApi.new(Temper.api_client)
    end
```

- [ ] **Step 6: Require both and run the specs**

Add `require 'temper/contexts'` and `require 'temper/cognitive_maps'` to `lib/temper.rb`.

```bash
cd clients/temper-rb && bundle exec rspec
```
Expected: PASS — full suite green.

- [ ] **Step 7: Commit**

```bash
git add clients/temper-rb
git commit -m "temper-rb: contexts, search, graph, and incremental cogmap authoring

Bulk reconcile is deliberately not surfaced: its chunks_packed is a required
pre-embedded String and Ruby has no BGE embedder (D9). A spec guards the absence."
```

---

### Task 10: CI — Ruby jobs, path scoping, and the drift gate

Currently `detect-ci-scope.sh` is binary: docs-only or full pipeline. The gem's jobs must be scoped to `clients/temper-rb/**` plus `openapi.json`, so they stay off the critical path of unrelated PRs — and `openapi.json` is in that trigger set precisely because a contract change must be *seen* to move the gem.

**Files:**
- Create: `.github/workflows/test-ruby.yml`
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/scripts/detect-ci-scope.sh`
- Modify: `.github/scripts/test-detect-ci-scope.sh`

**Interfaces:**
- Produces: `detect-scope` job output `run-test-ruby` (`"true"` / `"false"`); a reusable `test-ruby.yml` invoked via `workflow_call`.

> **Two traps.** (1) `run_test`'s assertion grep has no `|| true`, so asserting a flag the script does not yet emit **kills the harness via `set -e`** instead of reporting a FAIL — it can fail on a wrong value but not a missing one, which is exactly what a new flag needs. Fix that first or you never see an honest red. (2) The `__force_full_ci__` no-diff fallback must turn `test-ruby` **on**. A safety fallback that silently skips a job is not a safety fallback.

- [ ] **Step 0: Let the harness fail on a missing key**

In `.github/scripts/test-detect-ci-scope.sh`, inside `run_test`:
```bash
        local actual
        # `|| true`: an ABSENT key must report a FAIL with actual='', not kill the
        # harness via set -e.
        actual="$(echo "$output" | grep "^${var_name}=" | head -1 | cut -d= -f2- || true)"
```

- [ ] **Step 1: Write the failing scope-detection tests**

Append to `.github/scripts/test-detect-ci-scope.sh`, before the summary block:
```bash
run_test "ruby-only change runs test-ruby but not rust" \
    "clients/temper-rb/lib/temper/client.rb" \
    "RUN_TEST_RUBY=true" "DOCS_ONLY=false"

run_test "openapi.json change runs test-ruby (the gem must be seen to move)" \
    "openapi.json" \
    "RUN_TEST_RUBY=true"

run_test "unrelated rust change does not run test-ruby" \
    "crates/temper-api/src/handlers/resources.rs" \
    "RUN_TEST_RUBY=false" "RUN_TEST_RUST=true"

run_test "docs-only change runs nothing, including test-ruby" \
    "README.md" \
    "DOCS_ONLY=true" "RUN_TEST_RUBY=false"

run_test "the gem's own README is still docs-only" \
    "clients/temper-rb/README.md" \
    "DOCS_ONLY=true" "RUN_TEST_RUBY=false"

run_test "a self-referential change forces every job on" \
    ".github/scripts/detect-ci-scope.sh" \
    "RUN_TEST_RUBY=true" "RUN_TEST_RUST=true"
```

- [ ] **Step 2: Run them to verify they fail**

```bash
bash .github/scripts/test-detect-ci-scope.sh
```
Expected: FAIL — `RUN_TEST_RUBY: expected='true' actual=''`.

- [ ] **Step 3: Teach the scope script about Ruby**

In `.github/scripts/detect-ci-scope.sh`, after the `HAS_SELF` block (line ~107), add:
```bash
# Ruby SDK: the gem's own tree, the contract it is generated from, and its CI
# workflow. openapi.json is in this set precisely because a contract change must
# be SEEN to move the gem -- that is what the codegen drift gate proves.
#
# The no-diff safety fallback must run everything, this job included.
HAS_RUBY=false
if changes_match '^clients/temper-rb/|^openapi\.json$|^\.github/workflows/test-ruby\.yml$|^__force_full_ci__$'; then
    HAS_RUBY=true
fi
```

Replace the job-flag block (lines ~126–136) with:
```bash
if [ "$DOCS_ONLY" = "true" ]; then
    RUN_CODE_QUALITY=false
    RUN_TEST_RUST=false
    RUN_TEST_TYPESCRIPT=false
    RUN_TEST_RUBY=false
    SCOPE_SUMMARY="docs-only: skipping code-quality, test-rust, test-typescript, test-ruby"
else
    RUN_CODE_QUALITY=true
    RUN_TEST_RUST=true
    RUN_TEST_TYPESCRIPT=true
    # test-ruby is the one PATH-SCOPED job: it needs Docker for the drift gate,
    # so it stays off the critical path of PRs that cannot possibly affect it.
    # HAS_SELF forces it on, matching the script's conservative posture.
    if [ "$HAS_RUBY" = "true" ] || [ "$HAS_SELF" = "true" ]; then
        RUN_TEST_RUBY=true
    else
        RUN_TEST_RUBY=false
    fi
    SCOPE_SUMMARY="full-ci: code change detected — running full pipeline (test-ruby=${RUN_TEST_RUBY})"
fi
```

Add to both output blocks:
```bash
printf 'RUN_TEST_RUBY=%s\n' "$RUN_TEST_RUBY"
```
and
```bash
        echo "run-test-ruby=${RUN_TEST_RUBY}"
```

Also extend the `debug` line to include `HAS_RUBY=$HAS_RUBY`, update the two existing harness cases that assert `SCOPE_SUMMARY` verbatim, and change `ci-success`'s skip message from `(docs-only, correctly skipped)` to `(out of scope, correctly skipped)` — a job can now be correctly skipped for path-scope reasons.

- [ ] **Step 4: Run the scope tests to verify they pass**

```bash
bash .github/scripts/test-detect-ci-scope.sh
```
Expected: PASS — all cases, including the six new ones.

- [ ] **Step 5: Write the Ruby CI workflow**

`.github/workflows/test-ruby.yml`:
```yaml
name: Ruby SDK Tests

on:
  workflow_call:

jobs:
  test-ruby:
    name: temper-rb (rubocop, rspec, codegen drift)
    runs-on: ubuntu-latest
    timeout-minutes: 15

    defaults:
      run:
        working-directory: clients/temper-rb

    steps:
      - name: Checkout code
        uses: actions/checkout@v6

      - name: Set up Ruby
        uses: ruby/setup-ruby@v1
        with:
          ruby-version: "3.4"
          bundler-cache: true
          working-directory: clients/temper-rb

      - name: Lint
        run: bundle exec rubocop

      - name: Test
        run: bundle exec rspec

      # The generated core is committed, so contributors need no Docker to build
      # or test the gem. CI is where we prove it still matches the contract.
      #
      # This step is why the job is path-scoped -- it pulls a ~1GB image.
      - name: Codegen drift gate
        run: bundle exec rake drift

      # A gem that cannot be packaged is not shippable, and the gemspec loads
      # lib/temper/version.rb standalone -- a regression there is invisible to
      # rspec but fatal to `gem build`.
      - name: Build the gem
        run: gem build temper-rb.gemspec
```

- [ ] **Step 6: Wire it into `ci.yml`**

In `.github/workflows/ci.yml`, add to `detect-scope.outputs`:
```yaml
      run-test-ruby: ${{ steps.detect.outputs.run-test-ruby }}
```

Add a job after `test-typescript`:
```yaml
  test-ruby:
    needs: detect-scope
    if: needs.detect-scope.outputs.run-test-ruby == 'true'
    uses: ./.github/workflows/test-ruby.yml
```

Add `test-ruby` to `ci-success`'s `needs`:
```yaml
    needs: [detect-scope, code-quality, test-rust, test-typescript, test-ruby]
```

Add a `check_job` call after the `test-typescript` one:
```bash
          check_job "test-ruby" \
            "${{ needs.test-ruby.result }}" \
            "${{ needs.detect-scope.outputs.run-test-ruby }}"
```

- [ ] **Step 7: Verify the gate teeth locally**

The `ci-success` gate treats a job that is out of scope and `skipped` as a pass. Confirm the new flag flows:
```bash
echo "crates/temper-api/src/main.rs" | bash .github/scripts/detect-ci-scope.sh --stdin | grep RUN_TEST_RUBY
```
Expected: `RUN_TEST_RUBY=false`.
```bash
echo "openapi.json" | bash .github/scripts/detect-ci-scope.sh --stdin | grep RUN_TEST_RUBY
```
Expected: `RUN_TEST_RUBY=true`.

- [ ] **Step 8: Run `cargo make check`**

This task touches `.github/`, so the repo-wide gate applies.
```bash
cargo make check
```
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add .github clients/temper-rb
git commit -m "temper-rb: CI jobs, path-scoped, with a codegen drift gate

test-ruby is the first path-scoped job: it needs Docker for the drift gate, so
it stays off PRs that cannot affect it. openapi.json is in its trigger set --
a contract change must be seen to move the gem."
```

---

### Task 11: README, the onboarding cliff, and the release script

Authentication buys nothing on its own. A minted M2M token yields a JIT-provisioned agent profile and *nothing else*: going live also needs a cogmap write grant and team membership, or every call authenticates cleanly and then 403s. That is the first wall a `temper-rb` user hits, and it is invisible from the contract.

**Files:**
- Create: `clients/temper-rb/README.md`
- Create: `clients/temper-rb/LICENSE`
- Create: `tools/scripts/release/publish-ruby.sh`
- Test: `clients/temper-rb/spec/readme_spec.rb`

- [ ] **Step 1: Write the failing README spec**

A doc that drifts from the code is worse than no doc. Pin the three claims that matter.

`clients/temper-rb/spec/readme_spec.rb`:
```ruby
# frozen_string_literal: true

RSpec.describe 'README' do
  let(:readme) { File.read(File.expand_path('../README.md', __dir__)) }

  it 'documents the four TEMPER_M2M_* variables by their real names' do
    %w[TEMPER_M2M_TOKEN_URL TEMPER_M2M_CLIENT_ID TEMPER_M2M_CLIENT_SECRET TEMPER_M2M_AUDIENCE]
      .each { |var| expect(readme).to include(var) }
  end

  it 'carries a Going live section naming both authorization steps' do
    expect(readme).to match(/##\s*Going live/i)
    expect(readme).to match(/cogmap write grant/i)
    expect(readme).to match(/team membership/i)
  end

  it 'documents the fork-safety hooks' do
    expect(readme).to include('Temper.reset_connection!')
    expect(readme).to include('on_worker_boot')
    expect(readme).to include('Sidekiq.configure_server')
  end
end
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cd clients/temper-rb && bundle exec rspec spec/readme_spec.rb
```
Expected: FAIL — `No such file or directory @ rb_sysopen - .../README.md`.

- [ ] **Step 3: Write the README**

`clients/temper-rb/README.md` must contain, at minimum:

````markdown
# temper-rb

A pure-Ruby SDK for the Temper knowledge-base API. No native extension: one source
gem, no compiler on the install box.

## Install

```ruby
gem 'temper-rb'
```

## Configure

```ruby
Temper.configure do |c|
  c.base_url  = ENV.fetch('TEMPER_API_URL')
  c.device_id = ENV['TEMPER_DEVICE_ID']   # optional; sets X-Temper-Device-Id
end
```

The connection is process-global. The **token is per call**.

## Puma — a token the caller already holds

```ruby
client = Temper::Client.new(credentials: Temper::Credentials::BearerToken.new(session_token))
client.resources.create(title: 'Postmortem', context_ref: '@dana/incidents',
                        doc_type_name: 'note', content: markdown,
                        act: Temper::Act.new(confidence: :probable, reasoning: '…'))
```

## Sidekiq — a machine principal

```ruby
CREDENTIALS = Temper::Credentials::ClientCredentials.new(
  token_url:     ENV.fetch('TEMPER_M2M_TOKEN_URL'),
  client_id:     ENV.fetch('TEMPER_M2M_CLIENT_ID'),
  client_secret: ENV.fetch('TEMPER_M2M_CLIENT_SECRET'),
  audience:      ENV.fetch('TEMPER_M2M_AUDIENCE'))
```

`audience` must equal the API's configured `AUTH_AUDIENCE`, or the minted token
fails validation before it ever reaches the machine-profile resolver.

## Errors

`Temper::TransientError` (429, 5xx, timeouts) vs `Temper::PermanentError` (400,
401, 403, 404, 409, 422). The split is load-bearing: Sidekiq retries a job whose
exception escapes. Let transients escape; rescue permanents and dead-letter them.
The SDK classifies; it never auto-retries a write.

## Going live

Authentication is not authorization. A minted M2M token gets you a
JIT-provisioned agent profile and nothing else — every call will authenticate
cleanly and then 403.

1. Provision an Auth0 M2M application and a client grant for the API's audience.
2. Set `TEMPER_M2M_TOKEN_URL`, `TEMPER_M2M_CLIENT_ID`, `TEMPER_M2M_CLIENT_SECRET`,
   `TEMPER_M2M_AUDIENCE`.
3. Grant the agent profile a **cogmap write grant** on the target map.
4. Add the agent profile to the **team** whose contexts it must read.

Assert it worked at boot, rather than discovering it on the first write:

```ruby
Temper::Client.new(credentials: CREDENTIALS).whoami
```

## Forking

The connection holds sockets. A forked worker must not inherit its parent's.

```ruby
# config/puma.rb
on_worker_boot { Temper.reset_connection! }

# config/initializers/sidekiq.rb
Sidekiq.configure_server { |_| Temper.reset_connection! }
```

## Cognitive maps

The SDK authors into a map incrementally — `cognitive_maps.author` (ingest with
`home_cogmap_id`), `assert_relationship`, `set_facet` — all of which the server
chunks and embeds for you.

**Bulk reconcile is not exposed.** `PUT /api/cognitive-maps/{id}` takes a
pre-embedded desired-state manifest: `chunks_packed` is a required, client-computed
768-dimension BGE embedding, carried verbatim with no server-side fallback. That is
a CLI operator's job. If you truly need it, reach
`Temper::Generated::CognitiveMapsApi` directly and pack the chunks yourself.

## Versioning

`Temper::VERSION` is the gem's own SemVer. `Temper::CONTRACT_VERSION` names the
`openapi.json` it was generated against. They answer different questions and are
not forced to agree.
````

- [ ] **Step 4: Add the license and run the spec**

```bash
cp LICENSE clients/temper-rb/LICENSE   # from the repo root
cd clients/temper-rb && bundle exec rspec spec/readme_spec.rb
```
Expected: PASS — 3 examples, 0 failures.

- [ ] **Step 5: Write the release script**

`tools/scripts/release/publish-ruby.sh` — modeled on tasker-core's, minus the platform-gem matrix (there is no native extension, so there is nothing to cross-compile):
```bash
#!/usr/bin/env bash
# Build and publish the temper-rb source gem to RubyGems.
#
# Usage: ./tools/scripts/release/publish-ruby.sh VERSION [--dry-run]
#
# There is no native extension, so there is no platform matrix: one source gem.

set -euo pipefail

VERSION="${1:-}"
DRY_RUN=false
[[ "${2:-}" == "--dry-run" ]] && DRY_RUN=true

[[ -z "$VERSION" ]] && { echo "Usage: $0 VERSION [--dry-run]" >&2; exit 1; }

GEM_NAME="temper-rb"
REPO_ROOT="$(git rev-parse --show-toplevel)"
GEM_DIR="${REPO_ROOT}/clients/temper-rb"

if [[ "$DRY_RUN" != "true" && -z "${GITHUB_ACTIONS:-}" ]]; then
    : "${GEM_HOST_API_KEY:?GEM_HOST_API_KEY is required for RubyGems publishing}"
fi

# Idempotency guard: never attempt to re-push an existing version.
if curl -sf "https://rubygems.org/api/v1/versions/${GEM_NAME}.json" \
    | grep -q "\"number\":\"${VERSION}\""; then
    echo "${GEM_NAME} ${VERSION} already published — nothing to do."
    exit 0
fi

cd "$GEM_DIR"
gem build "${GEM_NAME}.gemspec"

if [[ "$DRY_RUN" == "true" ]]; then
    echo "[dry-run] would publish ${GEM_NAME}-${VERSION}.gem"
    gem specification "${GEM_NAME}-${VERSION}.gem" | head -20
    exit 0
fi

gem push "${GEM_NAME}-${VERSION}.gem"
```

- [ ] **Step 6: Verify the gem builds and the guard works**

```bash
chmod +x tools/scripts/release/publish-ruby.sh
./tools/scripts/release/publish-ruby.sh 0.1.0 --dry-run
```
Expected: builds `temper-rb-0.1.0.gem`, prints the spec, publishes nothing.

Confirm the gem carries no generated cruft and no spec files:
```bash
cd clients/temper-rb && gem contents --spec-file temper-rb.gemspec 2>/dev/null || \
  tar -tf temper-rb-0.1.0.gem >/dev/null && gem spec temper-rb-0.1.0.gem files | grep -c "^- spec/"
```
Expected: `0` spec files in the packaged gem; `lib/temper/generated/**` present.

- [ ] **Step 7: Commit**

```bash
git add clients/temper-rb tools/scripts/release/publish-ruby.sh
git commit -m "temper-rb: README, license, and release script

The README's 'Going live' section answers the onboarding cliff -- authentication
is not authorization -- and a spec pins it so the doc cannot drift silently."
```

---

### Task 12: Prove fork safety

Designed, not proven, in the spec. `tasker-rb` sidesteps forking entirely (its example app runs single-mode Puma and nothing hooks `Process._fork`), so there is no precedent to copy. This is the last thing standing between the gem and a v1 tag.

**Files:**
- Test: `clients/temper-rb/spec/fork_safety_spec.rb`
- Modify: `clients/temper-rb/lib/temper/connection.rb` (only if the test finds a defect)

- [ ] **Step 1: Write the failing fork spec**

`clients/temper-rb/spec/fork_safety_spec.rb`:
```ruby
# frozen_string_literal: true

RSpec.describe 'fork safety' do
  before do
    Temper.reset_connection!
    Temper.configure { |c| c.base_url = 'https://api.test' }
  end

  after { Temper.reset_connection! }

  it 'is skipped where fork is unavailable' do
    skip 'fork is unsupported on this platform' unless Process.respond_to?(:fork)
  end

  # The parent builds a connection; the child MUST NOT reuse its sockets.
  it 'gives a forked child a distinct connection after reset_connection!' do
    skip 'fork is unsupported' unless Process.respond_to?(:fork)

    parent_client = Temper.api_client
    parent_id = parent_client.object_id

    reader, writer = IO.pipe
    pid = fork do
      reader.close
      Temper.reset_connection!
      writer.write(Temper.api_client.object_id == parent_id ? 'SHARED' : 'DISTINCT')
      writer.close
      exit!(0)
    end
    writer.close
    result = reader.read
    Process.wait(pid)

    expect(result).to eq('DISTINCT')
  end

  # The real hazard: a child that does NOT reset inherits the parent's live
  # socket, and two processes interleave bytes on one TLS connection. We cannot
  # assert corruption safely, so we assert the memo is what carries the risk --
  # which is exactly what reset_connection! clears.
  it 'a child that skips reset_connection! inherits the parent memo' do
    skip 'fork is unsupported' unless Process.respond_to?(:fork)

    parent_id = Temper.api_client.object_id

    reader, writer = IO.pipe
    pid = fork do
      reader.close
      writer.write(Temper.api_client.object_id == parent_id ? 'INHERITED' : 'FRESH')
      writer.close
      exit!(0)
    end
    writer.close
    result = reader.read
    Process.wait(pid)

    expect(result).to eq('INHERITED')
  end

  it 'a forked child can complete a request after reset_connection!' do
    skip 'fork is unsupported' unless Process.respond_to?(:fork)

    WebMock.allow_net_connect!
    server = TCPServer.new('127.0.0.1', 0)
    port = server.addr[1]
    accepter = Thread.new do
      2.times do
        socket = server.accept
        socket.gets
        socket.print("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}")
        socket.close
      end
    end

    Temper.reset_connection!
    Temper.configure { |c| c.base_url = "http://127.0.0.1:#{port}" }
    client = Temper::Client.new(credentials: Temper::Credentials::BearerToken.new('tok'))
    client.whoami   # parent warms the connection

    pid = fork do
      Temper.reset_connection!
      Temper::Client.new(credentials: Temper::Credentials::BearerToken.new('tok')).whoami
      exit!(0)
    end
    _, status = Process.wait2(pid)

    accepter.kill
    server.close
    WebMock.disable_net_connect!

    expect(status.exitstatus).to eq(0)
  end
end
```

- [ ] **Step 2: Run it**

```bash
cd clients/temper-rb && bundle exec rspec spec/fork_safety_spec.rb
```
Expected: the first three PASS against the Task 5 implementation. The fourth is the real probe — it drives a live socket across a fork.

**If the fourth test fails**, the defect is real and this is what the task exists to find. Do not weaken the test — report the failure mode and fix `connection.rb`.

The library-safe fix is the one already shipped: `reset_connection!` clears the memo, and the README asks the host for two lines in `on_worker_boot` / `Sidekiq.configure_server`. Ruby ≥ 3.1 also exposes `Process._fork`, which Ruby invokes in the child and which a library *could* override to auto-reset. **Do not reach for it unless the fourth test proves the explicit hook insufficient.** A library that silently rewires its host application's fork semantics is worse than one that asks for two lines of config: the override is process-global, composes badly with every other gem that does the same, and hides the socket lifetime from the person who has to debug it.

- [ ] **Step 3: Run the full suite**

```bash
cd clients/temper-rb && bundle exec rake
```
Expected: rubocop clean, every spec green.

- [ ] **Step 4: Commit**

```bash
git add clients/temper-rb
git commit -m "temper-rb: prove fork safety with a live socket across fork

Closes the spec's last 'designed, not proven' thread. The explicit
reset_connection! hook is preferred over monkey-patching Process._fork:
a library should not silently rewire its host's fork semantics."
```

---

## Final verification

```bash
# The gem, end to end
cd clients/temper-rb
bundle exec rubocop                      # clean
bundle exec rspec                        # all green
bundle exec rake drift                   # no diff vs the contract

# The repo gates that the gem's CI wiring touches
cd ../..
bash .github/scripts/test-detect-ci-scope.sh   # all cases pass
cargo make check                                # unaffected, still green

# The contract still generates from scratch
cargo make openapi-validate                     # 0 errors
```

**Acceptance:**

1. `clients/temper-rb/` builds a source gem with no compiler and no Docker from a clean checkout.
2. `rake generate` is the only writer of `lib/temper/generated/**`, and `rake drift` proves it — verified by tampering.
3. `Temper::CONTRACT_VERSION == openapi.json`'s `info.version`.
4. A `BearerToken` call does zero token I/O; a `ClientCredentials` call mints once, caches to 60s before absolute expiry, and re-mints exactly once on a mid-job 401.
5. Reads retry 5xx three times; writes never auto-retry.
6. `Temper::Act` rejects authorship-without-confidence before any request is made.
7. The skin exposes no bulk-reconcile path.
8. A forked child completes a request after `reset_connection!`.
9. `test-ruby` runs on `clients/temper-rb/**` and `openapi.json`, and on nothing else.

---

## Open threads carried forward

- **G3 (`019f4bdc`) is the one external dependency.** It designs the same machine-principal auth path for the Rust client. There is no shared *code* — the steward is TypeScript, the gem is Ruby — but there is a shared **contract**: the four `TEMPER_M2M_*` variables, machine-identity-first precedence, the absolute-`expires_at`-plus-60s-skew cache, and re-mint-on-401. Task 4 conforms to it. **If G3 changes that contract, Task 4 moves with it.** Nothing in Tasks 1–3 or 5–12 depends on G3, and the `ClientCredentials` unit tests are fully closed under WebMock — only the *live* tier waits.
- **The live integration tier is deferred**, gated on an env var, following `tasker-rb`'s `FFI_CLIENT_TESTS` precedent. It cannot be written honestly until an M2M app exists to point it at, and it needs the cogmap write grant + team membership from the README's "Going live" section.
- **`ContextOwnerRef` stays hand-constructed** (Task 9). Fixing the serde representation properly — adjacently tagged, or a decorated string with a custom impl — has a blast radius well beyond the gem.
- **`422` and `500` are declared on no operation.** `ErrorMapper` classifies them off the raw status and degrades `#message` to the raw body on `#details`. Widening the contract to declare them is a temper-side follow-up.
- **Three schemas are registered but referenced by no operation** — `SearchResultRow`, `EmbedDispatchSummary`, `InvocationCloseAck`. `openapi-generator validate` reports them as recommendations. They emit three unused models into the gem. Harmless; worth a follow-up.
- **The faraday pin is a live conflict surface.** Rails apps carry faraday transitively, and this plan *raises* D11's floor from `>= 1.0.1` to `>= 2.5` because `faraday-net_http_persistent` 2.x requires it. If that bites a real consumer, the fallback is `httpx` (pure Ruby, persistent by default), not typhoeus.
- **`temper-py` and `temper-ts`** inherit P0–P3 and P5 for free. D9's incremental-authoring constraint applies to them identically: no SDK gets bulk reconcile without an embedder.
