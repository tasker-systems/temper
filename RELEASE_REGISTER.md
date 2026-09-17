# Release-verdict register

The durable home of the declared-class gate (shared semver policy, D-S4 — spec of record:
`temper-artifacts/specs/2026-09-09-shared-semver-policy-design.md`, §4.1). Every wire-touching PR
lands its declaration row here, in the same PR. The release checklist consults this register
before any release: a release whose surface class has an open gated entry waits.

Each row reads: citation · what changed behind which unchanged shape · who observes it ·
user-visibility · release relevance. Beneath the citation line, machine fields, one per line:
`pr:` the PR number, `pre-policy`, or `goal` · `classes:` a subset of `additive`,
`shape-breaking`, `behavioral` · `surfaces:` a subset of `http`, `mcp`, `cli-stdout`, `clients`,
`schema`, `internal` · `status:` one of `open`, `signal-only`, `blocked:<release-class>`,
`satisfied`.

## Since v0.5.0 — unreleased
 
- **Self-host route/cron table regeneration — counts and tables derived from `vercel.json`**
  `docs/playbooks/self-host-temper.md`'s route table, cron table, topology
  diagram, and routing-contract counts are regenerated from the current
  `vercel.json` (22 routes, 10 crons, including the three drains the page
  previously omitted); the drains-queries pointer names its repository path.
  `/using-temper`'s doc-type statement states the fourteen-type platform
  vocabulary and names the MCP `describe_schema` tool as its authority. Copy
  only — no contract shape moves, no runtime behavior change, no generated
  artifact re-stales.
pr: self
classes: additive
surfaces: internal
status: signal-only

- **Get-started regeneration — /builders, /agents, /using-temper, README, two playbooks**
  Every command, flag, feature, and MCP tool named on the published
  get-started sequences is regenerated from the installed binary's real
  output; example outputs are pasted from real runs; one playbook's SQL block
  is replaced with the shipped CLI command. Copy only — no contract shape
  moves, no runtime behavior change, no generated artifact re-stales.
pr: self
classes: additive
surfaces: internal
status: signal-only

- **Process-document relocation — 12 tracked docs moved to temper-artifacts, prose trims, citation repoints**
  Twelve tracked process documents untrack from `internal/` and
  `design-system/docs/` (they now live in the private temper-artifacts repo);
  three prose spans trim; four citations repoint at their temper-artifacts
  homes (one under `packages/agent-workflows/`). Copy and process only — no
  contract shape moves, no runtime behavior change, no generated artifact
  re-stales.
pr: self
classes: additive
surfaces: internal
status: signal-only

- **Public-surface copy redaction — roadmap-voiced passages rewritten to present-tense**
  Eleven passages across five `docs/` pages and five temper-ui `(public)` pages
  are rewritten to state shipped behavior in present-tense. Copy only — no
  committed contract shape moves and no runtime behavior change; svelte-check
  and biome pass unchanged.
pr: self
classes: additive
surfaces: internal
status: signal-only

- **This release — the 0.5.1 fleet alignment: VERSION 0.5.0 → 0.5.1 across crates, packages, and clients**
  The release train's own wire delta is none: version fields and the generated
  cores re-stale with the bump (the D-S3 baseline — no shape movement); the
  P floor rides the additive rows already in this window. The release exists to
  restore the attestation chain (the v0.5.0 predicate names the tag-push door's
  entry) and to carry the public-registry lanes' first publishes.
pr: self
classes: additive
surfaces: http, clients
status: signal-only

- **The npm manifests' publishConfig points at the public registry**
  Completes the public-registry flip: `publishConfig.registry` still named
  `npm.pkg.github.com` in both TS manifests, so every `npm publish` inside those
  directories targeted GitHub Packages (and failed ENEEDAUTH against it) no matter
  where the operator logged in. Found by the first bootstrap publish, not by CI —
  no harness or gate reads the manifest's publish target.
pr: self
classes: additive
surfaces: clients
status: signal-only

- **The client publish lanes move to the public registries**
  rubygems.org for the gem, registry.npmjs.org for the TS packages — token-free for
  consumers (GitHub Packages gates reads even on a public repo, which walled every other
  repository's CI off from the packages). Publish-side auth is OIDC trusted publishing on
  both hosts, keyed to the chain's entry workflow; the GitHub Packages API-key machinery
  and its refusal-text classifier are retired with the lanes they served. No client code
  changes — distribution only. v0.5.0's GitHub Packages artifacts remain on their hosts,
  frozen.
pr: self
classes: additive
surfaces: clients
status: signal-only

- **The Python client's distribution becomes temperkb-py, published to pypi.org**
  Both natural names are taken on PyPI by unrelated projects (temper-py — a TEMPer
  USB-device reader; temper — an HTML DSL), so the install line changes name; the
  import package stays `temper` and no client code changes. The registry lane
  replaces wheel+sdist Release assets (the npm/RubyGems pattern): consumers on the
  0.5.1 direct-URL pin keep installing forever (the asset stays on the release),
  and picking up new versions is a one-line dependency change to `temperkb-py`.
  Publish-side auth is PyPI trusted publishing keyed to release.yml — the job's own
  workflow file, per the #901 correction; pending-publisher registration claims the
  name on first publish.
pr: 902
classes: additive
surfaces: clients
status: signal-only

- **The CLI stops reading malformed config and unreadable indexes as defaults; the mention agent runtime-checks its link-state response**
  An existing-but-malformed global config refuses naming the file instead of silently
  defaulting format/color/limits, and an unreadable memory index is refused instead of read
  as empty (migrate no longer plans writes off a fabricated reading). The mention agent
  runtime-checks the link-state response against its two arms and gives the slack narrowing
  and the mint refusal switch their missing never arms. No committed contract shape moves.
pr: self
classes: behavioral
surfaces: cli-stdout, internal
status: signal-only

- **temper-ui and temper-cloud stop presenting failure as success**
  The command palette checks `resp.ok` and renders a distinct failure state where an API
  error previously rendered "No results"; the internal search route logs its 5xx instead of
  returning a body shaped like a valid empty result. temper-cloud's token verification
  keeps an absent `email_verified` claim distinct from `false`, and validates the `/userinfo`
  body at runtime instead of casting it (latent path — no live route today). No committed
  contract shape moves.
pr: self
classes: behavioral
surfaces: internal
status: signal-only

- **Query responses refuse rather than fabricate when a stage's tally row is missing; agent and UI boundaries validate temper-api responses instead of casting them**
  Grounding ruled the tally absence a compile/execute contradiction — the planner emits one
  tally arm per stage and each arm yields a row even over an empty CTE, so `tally() == None`
  is compiler/executor disagreement, never "never ran" — and `0` is a real measured value
  (a refused stage's CTE is `WHERE false`). Where the assembler previously rendered
  `produced: 0` and derived `Extent::Complete` from it, it now fails the response; measured
  zeros still render zeros. The steward/auditor dispatch consumers, the mention mint
  outcome, and temper-cloud's JWT claims are runtime-checked at the boundary instead of
  cast; temper-ui reads required wire fields as required and renders absence as absence. No
  committed contract shape moves (behavioral only — the Option-shaped fix was ruled out by
  the grounding).
pr: self
classes: behavioral
surfaces: http, internal
status: signal-only

- **`temper update` refuses on brew-managed installs (the `--check` carve-out reports); install.sh warns when the PATH temper is brew-managed**
  The homebrew tap's authority boundary (D-H2): a `BREW-MANAGED` marker beside the binary —
  or, belt-and-braces, an exe under a brew install prefix — makes the mutating path of
  `temper update` refuse, naming the formula's own `brew upgrade` line from the marker;
  `--check` keeps reporting with a note that upgrading is brew's job. The installer's face
  is warn-only (an operator who invoked it deliberately has stated intent). Who observes:
  an operator running `temper update` on a brew-managed install — previously an attempted
  swap against a brew-owned tree, now a refusal naming the managed updater. Declared limit:
  the tap has no installed population until its first formula ships, so this guards a
  fleet, not a live defect. Behavioral — no committed shape moves; the CLI's own stdout
  gains a refusal and a note.
pr: self
classes: behavioral
surfaces: cli-stdout, internal
status: signal-only

- **`temper version --verify --online` reports the Homebrew boundary instead of comparing brew-transformed bytes**
  The homebrew formula plants a post-install manifest (computed from the actual installed
  tree), so offline `--verify` proves brew-install self-consistency; the ONLINE path cannot
  apply that comparison — brew finalizes Mach-O with per-install random re-signing
  identifiers and relocates metadata — so a brew-managed install now resolves `unverifiable`
  naming what carries provenance instead (the formula's archive digest, verified at
  download). Who observes: an operator running `--verify --online` on a brew install —
  previously a guaranteed dylib `mismatch`, now an honest `unverifiable`. Declared limit:
  the tap has no installed fleet until its first formula release; this states the boundary
  before one exists. Behavioral — the verdict JSON's shape is unchanged; its meaning for
  one population changes.
pr: self
classes: behavioral
surfaces: cli-stdout
status: signal-only

## Since v0.4.0 — unreleased

- **This release — the 0.5.0 fleet alignment: VERSION 0.4.0 → 0.5.0 across crates, packages, and clients**
  The release train's own wire delta is none: openapi.json's `info.version` re-stales with
  the bump (the D-S3 baseline — no shape movement, the jq-stripped base and head
  identical), and the generated client cores (temper-rb, temper-py) and the ts schema
  re-stale with it. The P floor is on the record through Citation 9 and the additive rows
  above.
pr: self
classes: additive
surfaces: http, clients
status: signal-only

- **The erasure record names the subject's attributed text in shared spaces**
  The attribution ruling (2026-09-13, with Pete, on the 2026-09-06 clause "the team
  remainder is named, never silent"): contributing into a shared space never carried a sole
  ownership or authorship claim — attribution is what the system provides and what
  survives, and erasure removes only what was truly private to the principal.
  `principal_erasure_survey_plan` appends one `independent_obligation`-shaped target naming
  the content-block hashes whose genesis event the subject's entity emitted, wherever the
  block's resource homes outside the governed estate (team and map homes alike) — capped at
  8 hashes with "and N more". The hashes are never admitted to
  `redacted_hashes`/`kb_erased_content`, and the redaction's reads stay governed-scoped —
  nothing in a shared home is the act's to strike. The act consumes the plan, so the
  execute door's response, the recorded `principal_erased` targets, and the survey gain the
  naming together; existing outcome shapes are unchanged.
  pr: self
  classes: behavioral
  surfaces: http
  status: signal-only

- **The erasure act's blob arm goes home-pure: every live governed-home blob row strikes with the estate**
  A guest-committed file homed in the erased principal's own context is now struck with the
  estate, whoever committed it — previously such rows were named in the record as retained
  with no release path, an obligation no act could ever discharge (the retirement of the
  row's home context had already closed every read into it). Who observes it: an operator
  reading an erasure record now sees the strike template (`erased; released=…; pathname=…`)
  for every governed-home row instead of an independent-obligation naming, the redacted set
  and the erased-content ledger include guest-committed rows' hashes, and the byte-delete
  fence releases their provider bytes when no live row holds the hash; a guest whose bytes
  die on someone else's erasure is the ruling's accepted cost, mitigated at commit-time
  disclosure (the terms-of-use line a contributor crosses) rather than in the act. No shape
  changes: the route is out of the OpenAPI contract (no schema restale), the
  `principal_erased` payload keys, the strike vocabulary and the D2 hash set are
  byte-identical, and the survey door moves with the act because both consume the one plan.
pr: self
classes: behavioral
surfaces: http
status: signal-only

- **The commit door discloses the estate line — `BlobCommitResponse.estate_scope_disclosure`**
  When a blob commits into a context governed by another profile, every committing door
  (`POST /api/blobs`, the segmented finalize, the MCP commit tool) carries a
  scope-of-engagement note in its response: bytes committed into another's context live
  and die with that estate — an erasure of its owner erases them. `None` for the caller's
  own context and for team-owned homes (the team line is the terms' other half). Additive
  field on one response class; the three generated client skins restale with the
  contract.
pr: self
classes: additive
surfaces: http, mcp, clients
status: signal-only

- **The erasure act gains a read-only survey door: `POST /api/admin/erasure/survey`**
  An operator can now ask what an erasure WOULD record before running it: the door serves
  the act's own computation (`principal_erasure_survey_plan`, which `principal_erasure_execute`
  itself consumes — one computation, whose would-strike verdicts simulate the act's own
  sequential refcount, so in the same state the preview matches the act; the strike-time
  verdict stays authoritative for writers after the survey), returning
  the predicted targets, redacted set and typed strike verdicts. Same operator-only posture
  as the execute door, mounted plain out of the OpenAPI contract (allowlisted, no schema
  restales); a non-operator gets the same silent 404 and — by ruling — NO recorded refusal:
  a survey attempt is not an erasure request. Behind unchanged shapes on the act itself: the
  refactor to consume the plan leaves the execute door's request/response and the recorded
  `principal_erased` payload byte-identical (the replay witnesses pass unmodified).
pr: self
classes: additive
surfaces: http
status: signal-only

- **Eleven MCP tool inputs inline their scalar enums instead of `$ref`ing them**
  The advertised input schemas of `context_read`, `context_manage`,
  `segmented_ingest`, `describe_schema`, `facet_set`, `facets_read`,
  `facet_retract`, `relationship`, `invocation_read`, `invocation_manage` and
  `cogmap_read` now carry each discriminator enum inline (the `oneOf`-of-string-consts
  form) rather than as `{"$ref": "#/$defs/…"}` into `$defs`, matching the blob pair and
  `ReblockTarget`, which already shipped inline. Who observes it: any client that does
  not resolve `$ref`/`$defs` — the Anthropic tool-use layer among them — previously got
  no type signal for these fields and could send explicit `null` (`-32602`); the same
  callers now see the allowed values. No request or response class changes shape: the
  tools accept and return exactly what they did, and the twelve per-tool schema-witness
  tests still hold. Release-relevant as a schema-payload change only; a new router-wide
  witness (`every_advertised_tool_input_inlines_scalar_enums`) keeps the property from
  regressing tool by tool.
pr: self
classes: behavioral
surfaces: mcp
status: signal-only

- **The erasure door attributes to the caller's claimed surface**
  `execute_erasure` and the refusal recorder take the door's `Surface` and resolve the
  `<handle>@<marker>` emitter from it, instead of hard-wiring `web` at both attributed
  sites — the shape `blob_service` was corrected out of once already. The HTTP door is the
  only caller today, so `web` was accurate; the day the act gains a second surface the
  ledger would have mis-attributed it silently, on the one act an auditor most needs to
  trust. Both arms move together: a completion and a recorded refusal each name the surface
  the call arrived on. Behind unchanged shapes — the route is out of the OpenAPI contract,
  so no schema restales. Also in the same PR, no wire relevance: the blob delete gate's
  relation-arm standing checks collapse to one query (same decision, fewer round trips
  inside the strike's lock), the custody refusal names the fold-the-edge exit, and three
  comments that called the staging reaper a hole now describe the shipped lifecycle.
pr: self
classes: behavioral
surfaces: http
status: signal-only

- **The element trail carries the act correlation id**
  Every event on a trail read (`temper trail`, MCP `element_trail`,
  `GET /api/graph/elements/{kind}/{id}/trail`) now projects `correlation_id` — the
  batch/act id the event ran under, stored since correlation threading but until now absent
  from every read surface, which left the reblock playbook's receipt-to-ledger validation
  step undeliverable. A batch act carries the batch id the receipt echoed; a self-rooted
  act carries its own event id; rows predating the column carry NULL and the field is
  absent from those responses. No existing request or response class changes shape.
pr: self
classes: additive
surfaces: http, mcp, cli-stdout, clients
status: signal-only

- **Corpus adoption's HTTP surface — `POST /api/resources/reblock`**
  One bounded, resumable re-blocking step per call: the body names a scope (one resource by id,
  one context by id, or the explicitly-named deployment-wide `all`), `dry_run`, an optional
  `limit` (a declared conservative default applies), and the `after_id` resume cursor; the
  response is the reblock receipt — one outcome row per candidate, per-class counts, the
  batch correlation id, and the continuation cursor. The route is documented in the OpenAPI
  contract (the deliberate contrast with the admin-enclosed re-embed trigger), and the three
  generated client skins gain the operation and its models. Dispatch goes to the existing
  backend command; the deployment-wide arm is SystemAdmin-gated at that seam, the
  resource/context arms ride the caller's own visibility. No existing request or response
  class changes shape — the route and its schemas are new.
pr: self
classes: additive
surfaces: http, clients
status: signal-only

- **The post-commit blob byte window closes — serialized releases, under-lock presence restore**
  Byte releases (the delete door's post-commit release and the fence drain's batched delete)
  re-derive released-ness under the hash advisory lock and hold it across the provider call,
  and the commit path's transaction re-derives byte presence under the same lock — restoring
  a missing object from the caller's own bytes before the row can go live. A commit racing a
  strike can no longer mint a live row over absent provider bytes: the state was reachable by
  two interleavings, permanent, and silent (the fence's re-derivation resolved the residue
  `done` as re-occupied). The two COMMENT statements that recorded the window as "healed on
  re-upload, watched by the fence" are re-stamped by migration `20260912000010`. No shape
  changes — openapi and client skins untouched; the presence-gate refusal now surfaces as
  `400` (was a scrubbed `500`), and a commit whose deduped row is struck mid-commit reports
  its declared content type instead of erroring.
pr: self
classes: behavioral
surfaces: http
status: signal-only

- **The blob-home exclusion — a blob homes in a context, never in a map**
  The blob home's vocabulary narrows to `kb_contexts`: a commit or segmented upload naming
  a cogmap as home is refused at the door, and the schema's `kb_blobs_home_context_only`
  CHECK refuses any direct writer — landing the delete-act goal's last named-open as
  examined-and-deliberately-excluded (a map is a distilled view over resources, not a data
  store, so no custodian is ever needed because no row can exist; the class is provably
  empty and the blob feature is pre-enterprise-rollout). `BlobUploadBeginRequest.home_table`'s
  description names the one home kind; request and response shapes are unchanged — the
  openapi diff is description text only, restaling the three client packages with it. The
  read floor's cogmap arm and the erasure sweep's `independent_obligation` outcome stay as
  defense-in-depth (frozen v1 vocabulary, byte-pinned by the fence's classifier). Ruled
  2026-09-11 with Pete; the decision lives in temper ("Blobs home in contexts only").
pr: self
classes: additive
surfaces: http, clients
status: signal-only

- **`create --sources-as-edges` qualifies each asserted edge at attribution grain**
  The authoring loop now also writes one `anchored-at` row per asserted `derived_from`
  edge per block of the created resource whose attribution names that source — carried
  (`is_carried`) rows included: exactly the grain the write surface can state, never less.
  Rows ride the edge asserts' non-atomic, warn-not-fatal posture: a failed anchor warns
  with remediation text and the committed create stands, and a retried create's anchors
  ack instead of erroring (insert-if-not-live). The flag's meaning grows; no flag is added
  and update gains none.
pr: self
classes: behavioral, additive
surfaces: cli-stdout
status: signal-only

- **`property_retracted` wired — the row-grain correction verb for edge-owned facet rows**
  A registered-since-seed event type gains its write path: HTTP
  `DELETE /api/relationships/{edge_handle}/facets/{property_id}` (act context as query
  parameters), MCP `facet_retract` (the unified `target` discriminator; `target=resource`
  refused — resource facet rows have no payload-stable ids), CLI
  `temper edge facet-retract <edge> <property-id>` (a sibling subcommand; the `edge facet`
  leaf invocation is untouched), and temper-client `FacetRetractOnEdge`. The projector is
  edge-bound (`id` + `owner` + `NOT is_folded`): a foreign, missing, or already-retracted
  id renders one indistinguishable 404 — no existence oracle over property rows. The row
  persists folded; the address is re-assertable as a fresh row. Payload is owner-shaped
  and ships permissive (no migration, no registry stamp, no schema snapshot), the
  `property_set` precedent; the element trail stays blind to property lifecycle.
  Replay re-folds the same row every time. Read shapes are unchanged.
pr: self
classes: additive
surfaces: http, mcp, cli-stdout, clients
status: signal-only

- **The edge facets read resolves `anchored-at` addresses and states the verdict**
  Every facet row now carries `address_resolution` and `verdict`; on `anchored-at` rows
  they state how the row's address resolved (the block read's own three-state contract —
  `live`, `folded` with its gated disposition envelope, `absent`) and, where the edge
  declares a direction (`derived_from` under both of its kind-shapes, source-side anchors)
  whether the anchored block's live, uncorrected attribution corroborates the
  qualification (`corroborated` / `divergent` / `unattributed`). On every other row — and
  on anchored rows outside a declared direction — both fields serialize null, never
  absent, and a payload without them parses: new readers read old writers, old readers
  read new writers. The computation runs only at this read; traversal and event surfaces
  never compute it. No existing request or response class changes shape — fields are
  added.
pr: self
classes: additive
surfaces: http, mcp, cli-stdout, clients
status: signal-only

- **The facet tools teach the `anchored-at` vocabulary — agent-caller meaning changes**
  The `facet_set` / `facets_read` MCP tool descriptions document the keyed mode and the
  resolution/verdict fields; the agent-skills `knowledge-base.md` documents the
  `facet_set` / `facets_read` / `facet_retract` set and the correction loop
  (`facet_retract` joins the writes census). Wire shapes are unchanged — what the
  descriptions MEAN for an agent caller is new.
pr: self
classes: behavioral
surfaces: mcp
status: signal-only

- **The keyed edge-owner facet write — `property_key` on the edge facet surfaces**
  `POST /api/relationships/{edge_handle}/facets`, MCP `facet_set` (`target: edge`), and
  `temper edge facet --key` grow an optional `property_key`: when set, `values` is asserted
  as ONE row under that key through the shipped `property_asserted` event, instead of the
  clustering `facet` verb (whose hardcode is untouched). Assertion is insert-if-not-live: a
  repeated assert of a live (owner, key, value) acks the existing row id instead of
  erroring. Omitted, every existing request behaves exactly as before; no existing request
  class changes shape. First consumer is the `anchored-at` span qualification.
pr: self
classes: additive
surfaces: http, mcp, cli-stdout, clients
status: signal-only

- **Structural validation for `anchored-at` writes (shared dispatch)**
  A keyed write under `anchored-at` refuses unless the value is exactly
  `{"endpoint": "source"|"target", "address": "<resource-uuid>#<block-uuid>"}` in the one
  canonical form, the named endpoint's side is a resource, and the address's resource half
  is that endpoint's id. Validation sits in the backend dispatch every surface reaches, so
  no skin bypasses it; it probes structure only — never whether the addressed block exists,
  is live, or is visible (no existence oracle; resolution is the read contract's to state).
  A keyed write under any key other than the one declared key — `facet`, a resource
  owner, a misspelling — refuses outright. Previously no
  surface could write a keyed row at all, so nothing accepted-then-written changes.
pr: self
classes: behavioral
surfaces: http, mcp, cli-stdout
status: signal-only

- **The hash-global erasure refusal grain retired (offboarding ruling)**
  The erasure act's write-path refusals are removed: a create or revise carrying a hash in
  `kb_erased_content` now lands where it previously refused, and the reconcile arm no longer
  drops erased-hash chunks before the merkle — same request/response shapes, different
  outcomes. The text redaction itself scopes to the erased subject's governed homes (a
  same-hash resource in another home keeps prose, vectors and search vectors, where the
  prior shape emptied them). `kb_erased_content` remains as the ledger-derived record; the
  embed exclusion re-keys to the wiped row (empty content + set membership). Ruled with
  Pete 2026-09-10: erasure is offboarding — the authority line is custody, never byte
  identity.
pr: self
classes: behavioral
surfaces: http, mcp, cli-stdout, schema
status: signal-only

- **The delete act's ruled door — `DELETE /api/blobs/{id}` (+ temper-client/SDK `delete_blob`)**
  A born route and verb: a custodian strikes one blob under the ruled two-arm custody gate
  (relation arm: delete standing over every live relation's resource peer; home arm: the
  home custodian when none exist — personal owner, team owner role). One `blob_deleted`
  fires; already-struck and unknown ids read the same 404; no existing request class
  changes shape. Released bytes are deleted post-commit and watched by the existing fence.
  The SDKs inherit the new operation from the contract.
pr: self
classes: additive
surfaces: http, clients
status: signal-only

- **Blob relate narrowed to `kb_resources` peers (HTTP/MCP/CLI one parse point)**
  A blob relation whose peer is a `kb_cogmaps` or `kb_blobs` anchor now refuses with the
  naming 400 (`blob_relate:` vocabulary), where it succeeded before. Ruled with the
  delete-act design: no delete standing resolves over such a peer, so the edge would pin
  its row permanently. Existing resource-peered relations are untouched; pre-narrowing
  edges persist (rendering as they always did) and their fold exit is unchanged.
pr: self
classes: behavioral
surfaces: http, mcp, cli-stdout
status: signal-only

- **The defined dangling state — the born block-addressed read (HTTP route)**
  `GET /api/resources/{id}/blocks/{block_id}` exists; no block-id-addressed read existed
  before, so no existing request class changes shape. Three states by name: 200 live, 410
  folded (state envelope: attribution history + gated successor dispositions), 404 absent —
  no redirect. Read callers observe it; the resolution contract is new.
pr: self
classes: additive
surfaces: http
status: signal-only

- **The defined dangling state — MCP block-addressed resolution (`get_block`)**
  Block-addressed resolution joins the provenance tool family, answering the same tri-state
  envelope as data. No earlier tool addressed a block id for reads.
pr: self
classes: additive
surfaces: http, clients
status: signal-only

- **Corpus adoption's MCP door — `resource_reblock`**
  The same bounded, resumable re-blocking step, one tool with a scope discriminator (the unified
  `facet_set` naming shape): `scope` names `resource`, `context`, or `all` and carries the
  per-arm ref, `dry_run` selects the survey, and `limit`/`after_id` bound and resume the walk;
  the tool result is the receipt the HTTP route returns. Dispatch goes to the existing backend
  command — the deployment-wide arm's system-admin gate and the per-resource gate train are the
  backend's, unchanged. The tool description is client-published product: it ships to Anthropic
  clients verbatim, and the input schema inlines its scope enum (`#[schemars(inline)]` — a
  `$ref`-ed enum reaches Anthropic tool-use as null).
pr: self
classes: additive, behavioral
surfaces: mcp
status: signal-only

- **Corpus adoption's CLI door — `temper admin reblock`**
  The operator walk, beside `admin reembed`: exactly one of `--resource`, `--context`, or
  `--all` (none is a refusal, not a default — the deployment-wide arm must be asked for by
  name), `--dry-run`, `--limit`, and `--after-id` resume; the receipt renders to stdout under
  the agent-first defaults. Ref resolution matches the sibling command (decorated
  `slug-<uuid>` refs; contexts through the ordinary read resolution). Goes through the typed
  client's new `reblock` method, not its own HTTP call.
pr: self
classes: additive
surfaces: cli-stdout
status: signal-only

- **The defined dangling state — temper-client `BlockRead` types + CLI `resource read-block`**
  The envelope and route types are born in temper-client and the CLI read command inherits
  them; every Rust caller gets the tri-state contract. Born surfaces — nothing moved.
pr: self
classes: additive
surfaces: clients, cli-stdout
status: signal-only

- **The defined dangling state — `ResourceReblocked` per-folded-id disposition map**
  The fold event carries where each folded incumbent's content went (absorbers = full
  chunk-hash multiset, kept AND created; carried copies; content-gone arm), captured at
  computation time. Additive payload-schema change, `serde(default)`: pre-map events replay
  identically and resolve to the defined `unrecorded` disposition. Schemars snapshot +
  `kb_event_types` re-stamp ride the same change (precedent 20260908000010, additive posture,
  `schema_version` stays 1).
pr: self
classes: additive
surfaces: schema, internal
status: signal-only

- **The defined dangling state — annotate/revise on a folded or absent block: 500-class → defined states**
  A write addressing a folded block now answers 410 Gone (`TemperError`/`ClientError::Gone`,
  named on MCP and CLI), and a not-under-this-resource block answers 404 — where both
  bridged to 500-class `internal_error` before. The discrimination is unchanged; only its
  error class and shape moved. Existing request class, error→response.
pr: self
classes: behavioral
surfaces: http, mcp, cli-stdout
status: satisfied

- **The defined dangling state — citation-audit gate: folded ≡ live → defined refusal**
  Auditing a citation whose block is folded is now refused (the gate's standing zero-rows→404
  dialect) where it silently succeeded on a gone citation before. Existing request class,
  success→refusal; the refusal face is unchanged, the resolved-to state moved.
pr: self
classes: behavioral
surfaces: http, mcp, cli-stdout
status: satisfied

- **Citation 1 — PR #867 briefing: identical body + sources preserves block identity**
  A whole-body update whose sections and sources are byte-identical now keeps their block ids,
  revision history, and provenance; before, every section re-minted fresh ids. Block-aware
  callers holding ids observe it, on every write surface. User-visible defect fix — the case
  that motivated the block-grain sequencing rule; briefed at the #867 merge. Gates the next
  release of these surfaces until it is carried and marked satisfied.
pr: 867
classes: behavioral
surfaces: http, mcp, cli-stdout
status: satisfied

- **Citation 2 — PR #867 briefing: content-gone citations stop reinforcing standing**
  A rewriting revise moves prior sources to history on the folded block; live citation
  magnitude and reinforcement reflect only what survives. Standing consumers observe it — the
  most user-visible change in the set: a rewritten finding's citation count drops to what still
  exists. Owes a release-notes signal; gates the next release of these surfaces until carried.
pr: 867
classes: behavioral
surfaces: http, mcp, cli-stdout
status: satisfied

- **Citation 3 — PR #867 briefing: redistributed carried rows become live citations**
  Absorbed and carried copies now count as uncorrected rows on live blocks; before this change
  they were invisible. Standing consumers observe it: counts on rewritten findings reflect
  redistributed copies. Owes a release-notes signal; gates the next release until carried.
pr: 867
classes: behavioral
surfaces: http, mcp, cli-stdout
status: satisfied

- **Citation 4 — PR #867 briefing: one `resource_reblocked` per whole-body update**
  The ledger grain changes: one `resource_reblocked` computed against pre-update incumbents,
  where `block_mutated` + `resource_reblocked` fired before. Ledger consumers observe it; the
  fan-out correlation doc names `block_mutated` as an update sub-event and owes its amendment.
  Gates the next release until carried.
pr: 867
classes: behavioral
surfaces: internal
status: satisfied

- **Citation 5 — PR #867 briefing: chunker-skew 500 retires to async backfill**
  A CLI↔server chunker skew no longer fails the update with a 500 — unmatched caller chunks
  fall to async embed backfill, so a failure face becomes success-with-backfill. Chunk-packing
  callers observe it. Gates the next release until carried.
pr: 867
classes: behavioral
surfaces: http, mcp, cli-stdout
status: satisfied

- **Citation 6 — PR #867 briefing: single-section identical rewrite is silent**
  A single-section identical rewrite with no sources emits no revision event, where it minted a
  fresh one before. Ledger consumers observe it. Gates the next release until carried.
pr: 867
classes: behavioral
surfaces: internal
status: satisfied

- **Citation 7 — PR #867 briefing: region content clocks advance on whole-body replaces**
  Region content clocks advance on whole-body replaces (previously via `block_mutated`;
  re-blocks advanced nothing). Internal only — region readouts and materialization freshness.
  No client signal owed; recorded for the record, no release gate.
pr: 867
classes: behavioral
surfaces: internal
status: signal-only

- **Citation 8 — PR #867 briefing: folded incumbents' roles leave live view selectively**
  On a whole-body update, folded incumbents' roles leave the live view selectively, where all
  roles were destroyed before. Block-aware callers observe it; the loss is strictly narrower.
  Gates the next release until carried.
pr: 867
classes: behavioral
surfaces: http, mcp, cli-stdout
status: satisfied

- **Citation 9 — PR #867 briefing: `is_carried` on the provenance read**
  The provenance read carries `is_carried`, letting callers distinguish carried copies from
  direct rows. Additive and serde-tolerant in both deploy-skew directions (no
  `deny_unknown_fields`; `serde(default)` on the new field). API/MCP/CLI read consumers; the
  only row of the nine that shape inspection can see. Not a gate: it forces the P floor at the
  next release through the calculator.
pr: 867
classes: additive
surfaces: http, mcp, cli-stdout
status: signal-only

- **Standing blocker — defined-dangling-state resolution (mixed-fleet sequencing rule)**
  An address into rewritten-away content resolves exactly as undefined after the #867 landing
  as before it: the defined-dangling-state work is the remaining open half of the sequencing
  rule, beside the landed redistribution and `is_carried` visibility. No release exposing
  block-grain annotations to end callers at scale may ship while this entry is open — the
  release verdict belongs to the semver arc.
  RESOLVED by the defined-dangling-state build (this PR): the three-state resolution contract
  (live/folded/absent, successor-naming via the disposition map) now exists on every read
  surface, and the write and audit faces join it.
pr: goal
classes: behavioral
surfaces: clients, http, mcp, cli-stdout
status: satisfied

- **This branch — the semver mechanism itself: version plumbing with no shape movement (spec D-S3)**
  The release spine lands (`tools/scripts/release/`), the PR compat-class declaration field and
  this register arrive, and `openapi.json` `info.version` now derives from `VERSION` at emit
  time — the live 0.1.0-vs-0.4.0 divergence closes (spec D-S3). The `openapi.json` diff is
  info.version-only: 0.1.0 → 0.4.0 with the jq-stripped base and head identical, no shape
  movement; the generated client cores (temper-rb, temper-py) re-staled with the bump, resting
  the same shapes against the same paths. Existing clients observe nothing but the version
  string.
pr: self
classes: additive
surfaces: http, clients
status: signal-only

- **This branch — the client publish lanes: hosting and scope, no client code change**
  The first publish paths for the client packages: temper-ts and temper-telemetry-ts re-scope
  to `@tasker-systems/*` for GitHub Packages npm (which requires scopes) and gain
  publishConfig + repository metadata; the gem's `allowed_push_host` moves to
  `rubygems.pkg.github.com/tasker-systems`; the temper-py wheel and sdist ride the GitHub
  Release as PEP 508-referable assets. No generated client code, no wire shape, and no
  dependency resolution changes in-repo — consumers sweep import specifiers and `file:` dep
  keys to the new names mechanically. Publishing happens only at a tagged release.
pr: self
classes: additive
surfaces: clients
status: signal-only

- **The erasure act — operator doors: `POST /api/admin/erasure` and the fence tick `/api/erasure/drain`**
  Born HTTP routes; no existing request class changes shape. The execute door takes the
  operator erasure request (subject profile + opaque request reference; authorization lives in
  `erasure_service::execute_erasure`, so the route is deliberately gate-free) and answers the
  per-target completion/refusal outcomes; the drain endpoint is the byte-delete fence's
  scheduler tick, deriving pending blob deletes from the `principal_erased` payload verdicts,
  batched and retried. Existing clients observe nothing — the paths did not exist before.
pr: self
classes: additive
surfaces: http
status: signal-only

- **The erasure act — the `request` reference rel (`RefRel::Request` + its `LedgerRefRel` mirror)**
  The ledger reference vocabulary (the `kb_events."references"` apparatus the act owns) gains a
  `request` arm naming the erasure request an event fulfils; the temper-core mirror gains the
  matching arm and the parity test compiles the two together. Nothing emits the arm yet — the
  request reference rides references, not payloads, and no ledger row carries it — so no
  existing read or write changes; the arm is forward vocabulary rendered by the admin-ledger
  read once rows exist.
pr: self
classes: additive
surfaces: http, internal
status: signal-only

- **The span address form — `BlockSuccessor.home_resource_id` on the read envelope**
  A named successor now carries its row home (`Option<Uuid>`, `serde(default)`): the gated
  envelope alone constructs the successor's `<home>#<block>` address, resolving through the
  same value the read fork keys on. Additive both skew directions — new-reader/old-writer via
  the default (`None` = the pre-field world, declared; the `is_carried` pattern),
  old-reader/new-writer via no `deny_unknown_fields`. The type doc's single-resource wire
  decision is retired as it directed.
pr: self
classes: additive
surfaces: http, mcp, cli-stdout, clients
status: signal-only

- **The span address form — CLI composed block address**
  `resource read-block` accepts `<resource>#<block-uuid>` as one declared string form
  (`splitn(2, '#')`, length-capped, block half uuid-validated); the two-argument form stays.
  Input acceptance grows; stdout shape is unchanged.
pr: self
classes: additive
surfaces: cli-stdout
status: signal-only

- **The span address form — MCP `get_block` description names the successor's home**
  The tool description taught single-resource addressing ("call this again with its
  block_id"), which after cross-resource successors exist resolves a foreign successor to a
  false `absent`. The description and input docs now say a named successor's
  `home_resource_id` is the resource to pass. Unchanged input schema; the meaning behind it
  changes for agent callers — the tool description is the agent population's discovery
  surface for the form. Declared limit: no live producer emits cross-resource successors
  today, so the corrected instruction guards a future state, not a live defect.
pr: self
classes: behavioral
surfaces: mcp
status: signal-only

## Pre-policy (classified retroactively)

- **PR #858 — wire half: graph-edge listing DTO reshaped (`peer_resource_id!` → `peer_id!`)**
  The edge-listing DTO renamed `peer_resource_id!` to `peer_id!`, added `peer_table!`, and
  relaxed `peer_title`/`peer_slug` nullability (`!` → `?`) — regenerated into the client skins
  and reaching CLI stdout (`memory fetch` consumes `peer_id`); the CLI-stdout wire break of the
  #360 class. Shipped in v0.4.0 (release PR #859) with no classification, no changelog, and no
  client signal; classified retroactively. Under the current policy it would force the M
  question plus a pre-merge client-release plan.
pr: pre-policy
classes: shape-breaking
surfaces: http, mcp, cli-stdout, clients
status: signal-only

- **PR #858 — behavioral half: blob peers become visible to kind-switching clients**
  Rendering the rows the `kb_edges` CHECK widening had already admitted means any client
  enumerating or switching on edge peer kinds now observes a new state — blob peers — behind
  the listing. Shipped un-signaled in v0.4.0; classified retroactively (a briefing-class
  register entry under the current policy).
pr: pre-policy
classes: behavioral
surfaces: http, mcp, cli-stdout, clients
status: signal-only

- **PR #851 — binary blob substrate: new surfaces on API, MCP, and CLI**
  `blob put/get/list/relate` on the API, `blob_read`/`blob_manage` on MCP, multipart commit and
  segmented upload on the CLI, plus migration `20260903000020` widening the `kb_edges` CHECK to
  admit `kb_blobs`. Additive throughout — shapes only grew, old clients untouched. Shipped in
  v0.4.0; classified retroactively (forces the P floor, which that release carried).
pr: pre-policy
classes: additive
surfaces: http, mcp, cli-stdout, schema
status: signal-only

## Since 0.5.1 — unreleased

- **This release — the 0.5.2 fleet alignment: VERSION 0.5.1 → 0.5.2 across crates, packages, and clients**
  The release train's own wire delta is none: version fields and the generated
  cores re-stale with the bump (the D-S3 baseline — no shape movement); the
  P floor rides the additive rows already in this window.
pr: self
classes: additive
surfaces: http, clients
status: signal-only
