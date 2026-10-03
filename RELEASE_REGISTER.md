# Release-verdict register

The durable home of the declared-class gate (shared semver policy, D-S4 — spec of record:
`temper-artifacts/specs/2026-09-09-shared-semver-policy-design.md`, §4.1, as amended by the
compat-deprecation regime,
`temper-artifacts/specs/2026-09-22-compat-semver-policy-amendment-design.md`). Every wire-touching PR
lands its declaration row here, in the same PR. The release checklist consults this register
before any release: a release whose surface class has an open gated entry waits.

Each row reads: citation · what changed behind which unchanged shape · who observes it ·
user-visibility · release relevance. Beneath the citation line, machine fields, one per line:
`pr:` the PR number, `pre-policy`, or `goal` · `classes:` a subset of `additive`,
`shape-breaking`, `behavioral` · `surfaces:` a subset of `http`, `mcp`, `cli-stdout`, `clients`,
`schema`, `internal` · `status:` one of `open`, `signal-only`, `blocked:<release-class>`,
`satisfied`.

Routing (D-C2): a `shape-breaking` row below the era level names its routing by adding
`retirement-train` to `classes:` — the movement either converts to the deprecation path (re-declare
the row `additive` + `behavioral`; the re-declaration is the record, so `converted` needs no token)
or waits for the retirement release train. A bare `shape-breaking` row fails the declared-class
gate on a main-bound PR. Deprecation rows (the D-C3 records) carry the retirement horizon — the
era release the record names. Historical and pre-policy rows read as history: only new rows carry
the routing vocabulary (the #858 pre-policy row's present-tense law claim is grandfathered).

## Since v0.5.4 — unreleased
- **Two new admin doors: `POST /api/admin/resources/block-history-scrub` and its read-only `/survey`**
  The block history scrub (resource erasure D11) empties the history of named blocks of a resource
  that is not erased, behind two new operation ids (`admin_scrub_block_history`,
  `admin_survey_block_history_scrub`) under the `Admin` tag. No existing shape moves: the request,
  execute response and survey schemas are new, and a refused scrub is recorded on the existing
  `resource_erasure_refused` event with its new optional `act` and `blocks`. A list naming an id
  that is not a block of the resource is a 400 that records nothing, whatever the resource's
  state, so a recorded refusal names only real blocks of it. The migration re-registers the
  `block_history_scrubbed` and `resource_erasure_refused` payload schemas with optional
  properties only, adds the scrub's DB functions, and re-creates `resource_erasure_execute` with
  the same signature: its `kb_resources.ingest_state` target line is written only for an
  in-progress ingest, so an erasure after a scrub cancelled the ingest does not claim to have
  ended it. Who observes: system admins and SDK clients that call the new operations; a non-admin
  gets the erasure doors' 404. User-visible: operators only. Release relevance: additive.
pr: self
classes: additive
surfaces: http, clients, schema
status: signal-only
- **`ResourceView` gains `ingest_ended` — a resource whose ingest ended before its body was whole (`cancelled` by the block history scrub; `abandoned` is reserved for an abandoned-ingest reaper, and nothing sets it yet)**
  `ingest_state` keeps its two wire values, and an ended ingest reads `in_progress` there (it is
  not whole, and it stays hidden from list and search as an in-progress one is), with the new
  optional `ingest_ended` (`cancelled` | `abandoned`, skipped when absent) naming the reason. The
  DB column gains the two terminal states (migration `20261003000110`); a finalize or an append on
  an ended ingest answers 409 not-resumable (SQLSTATE TF004) under a new error code,
  `INGEST_ENDED` (additive: the other 409s keep `CONFLICT`), where it previously would have
  continued. The CLI's segmented upload, meeting that code, removes its resume record and says
  so, so the next run starts a fresh upload; its JSON error payload carries the same code. A
  re-block of a resource addressed directly declines an ended ingest under the existing
  `byteless` class, which now also covers an upload that ended before its body was whole; a
  context or deployment walk skips ended ingests. Who observes: API/SDK/CLI/MCP readers of `show`
  on a resource whose ingest a scrub cancelled, and a client resuming such an upload; only the
  scrub sets a terminal state. User-visible: yes, on such resources only. Release relevance:
  additive.
pr: self
classes: additive, behavioral
surfaces: http, mcp, cli-stdout, clients, schema
status: signal-only
- **MCP teardown: the context-ref anchor relays to `GET /api/contexts/resolve`; the last in-process gate and the unwired tool modules are deleted**
  The context orientation tools (`context_read`'s shape/metrics/analytics views,
  `context_materialize`) and `resource_reblock`'s `scope=context` resolve their context ref
  through the route-first resolve route instead of reading the pool after an in-process
  Level 1 + 2 gate, behind the same local parse. Every refusal face carries byte-exact —
  `invalid context ref: …`, `context not found: {the resolver's sentence}`, and the `+<team>`
  non-member's `context not found: Forbidden` — pinned against the in-process resolver
  before the swap. `ensure_profile_from_parts` (with the in-process `AuthzError` mapping only
  it used) and the never-wired `tools/admin_ledger.rs` / `tools/profiles.rs` are deleted; tool
  names, schemas and descriptions are byte-identical (the declarations fixture is unchanged).
  The source gate now requires every `#[tool]` to send a relay or sit on the named
  pure-compute allowlist (`describe_schema`). Who observes: an MCP-calling agent, whose one
  visible change is the declared delta — a fault behind the resolver (a database error) now
  renders `internal_error` where it rendered `invalid_params` under the `context not found: `
  prefix. User-visible: only on that fault path. Release relevance: signal-only.
pr: self
classes: behavioral
surfaces: mcp
status: signal-only
- **The CLI's context-ref reads resolve through `GET /api/contexts/resolve` — refusal sentences change**
  `resolve_context_id_for_read` (behind `temper context transfer|rename|delete|shape|
  region-metrics|analytics|materialize|materialize-delta`, `graph … --in`, the data-artifact
  shape commands' `--context` and the admin commands' context filters) stops listing every
  visible context and filtering client-side; it parses the ref with the shared `parse_context_ref`
  and lets the server resolve it, the same resolver every ref-accepting route uses. The warmup
  staleness pre-flight resolves the same way, which makes its `@me` match exact (it previously
  matched any `@`-owned context by slug, to avoid a profile round trip). A bare UUID still passes
  straight through. What a CLI reader sees changes; the error kind does not, with one exception
  named below. A context the caller cannot
  read, or that does not exist, now reads `context not found or not readable (ref "<ref>")` (the
  `@me/<slug>` form names the slug) where it read `context '<ref>' not found among the contexts
  you can see` — still `api`; a malformed ref is refused with the parser's own sentence — still
  `bad_request`, and the exception: a sigil-less `name/slug`, which was reported missing (`api`),
  is now refused there (`bad_request`);
  and a `+<team>/<slug>` from a non-member now reads `context ref "<ref>": you are not a member of
  that team` where it read "not found". Who observes: CLI users and agents reading CLI errors.
  User-visible: yes, in refusal wording only. Release relevance: signal-only.
pr: self
classes: behavioral
surfaces: cli-stdout
status: signal-only
- **`desktop_client_id` on `AuthProvider` — the deployment names the desktop's own OAuth client; `login()` resolves the device id from the store it is handed**
  The provider entry grows an optional `desktop_client_id` (`serde(default)`, skipped when
  absent): where the deployment registers the desktop's own public client — an Auth0
  application for the hosted instance, an `AS_CLIENTS` entry for self-hosted. The CLI never
  reads it and there is no fallback to `client_id`: absence stays observable so a desktop
  sign-in can refuse rather than present the CLI's client registration to a redirect that
  was never registered for it. Alongside, `login()`'s device-id resolution moves off the
  free no-arg disk read (`load_auth()`: env, then the global CLI auth file) onto the
  `TokenStore` it is already handed — an empty `MemoryTokenStore` mints a fresh UUIDv7 and
  persists it through that store, a populated one keeps its id, and a caller holding a
  non-disk store never consults the global auth path. The CLI is unchanged: the no-arg
  helper now delegates to the default `DiskTokenStore`, whose load carries the same
  env-then-disk precedence. Who observes: nobody at runtime today — the field has no CLI
  reader and the CLI's custody is the disk either way; the desktop consumer
  (temper-contrib) reads the field and inherits clean device-id custody when it lands.
  `docs/reference/config` re-renders with the field.
pr: self
classes: additive
surfaces: schema, clients
status: signal-only
- **`GET /api/contexts/resolve` — a context ref resolved to its id, for the caller (the route-first half of the network door's teardown)**
  A new route, nothing changed beside it: `GET /api/contexts/resolve?context_ref=<ref>` answers
  `{ "context_id": <uuid> }` (`ContextResolution`) for `@me/<slug>`, `@<handle>/<slug>`,
  `+<team>/<slug>` or a bare UUID. It is `context_service::resolve_context_ref` behind a route —
  the resolver every ref-accepting route already uses — so each ref form keeps exactly the answer
  it has elsewhere: a context the caller cannot read answers as one that does not exist (uniform
  404 on the UUID and `@<handle>` arms, an unknown handle included), a malformed ref answers 400
  with the shared parser's sentence, and the `+<team>` arm keeps its existing non-member 403. The
  ref grammar is the one `parse_context_ref`; no server-side dialect. Typed client method
  `ContextClient::resolve` (temperkb-client), with the ref in the query string, not the logged
  path. `openapi.json` gains the path and the schema (86 lines added, none removed); the three
  SDKs and the ts-rs `context.ts` regenerate with the addition only. Why: the MCP
  `context_anchor` resolver (`cognitive_maps.rs`, `reblock.rs`) is the last database read in a
  tool module; this route is what it relays to in teardown, one call per anchored tool. Who
  observes: API and SDK callers, who gain a route and a client method; no existing request or
  response shape changes. User-visible: no. Release relevance: additive.
pr: self
classes: additive
surfaces: http,clients
status: signal-only
- **Beat 5: the steward pair crosses the network door — `steward_ingest_delta` and `steward_advance_watermark` forward to `/api/steward`; no MCP tool executes on the direct binding**
  The last two direct handlers stop executing in-process and forward to
  `GET /api/steward/{cogmap}/delta` and `POST /api/steward/{cogmap}/watermark` as
  temper-client relays (`StewardClient`) — caller's bearer re-issued, service credential +
  `mcp` carrier as default headers, refusals mapped through the one `AcrossAuth` idiom. The
  routes make the identical calls the direct binding made, so the read gate and the
  auth-before-write gate are unchanged and now run behind the API's Level 1 + 2. Tool names,
  schemas, descriptions byte-identical (the declaration witness holds; the steward skill recipe
  test untouched). The cogmap ref parse stays MCP-local and pure (no read). ONE DECLARED PARITY
  DELTA (the G3c format; pinned green against the direct binding first, flipped in the swap
  commit, named in the tool module's and the parity suite's headers): the NotFound prefix
  (`steward_ingest_delta: ` / `steward_advance_watermark: `) drops for the server's bare
  sentence on three faces — the delta's unreadable/absent cogmap, the advance's cogmap exit,
  and its ingest-window exit; kind and gate identical, the two advance exits still
  distinguishable, unreadable still indistinguishable from absent. NOT a delta, pinned
  unchanged: the disclosing 403 keeps its prefix, sentence and kind. Attribution: neither act
  writes a ledger row (the advance moves two cursor columns and completes the workflow job;
  its command's `origin` is unread), so the family's witness pins the trusted path (both acts'
  carriers honored — bite-proven by refusing the carrier in `relay_trust`) and that an advance
  emits no `kb_events` row. A dead relay base URL reddens every wire-reaching face — the door
  is the only path. Who observes: an MCP-calling agent, chiefly the deployed steward runtime —
  three not-found messages lose their tool-name prefix; nothing else changes. User-visible:
  no. Release relevance: signal-only.
pr: self
classes: behavioral
surfaces: mcp
status: signal-only
- **Auth0-fronted instances advertise themselves as the RFC 8414 `issuer`**
  `GET /.well-known/oauth-authorization-server` on an instance without `AS_ISSUER` answers
  `issuer: "<MCP_BASE_URL>/"` where it answered the Auth0 tenant domain. The document's shape and
  every endpoint in it are unchanged; the issuer now equals the `authorization_servers` entry the
  protected-resource metadata names, as RFC 8414 §3.3 requires, so MCP clients that validate it
  complete discovery instead of aborting with an issuer mismatch. Access and ID tokens still carry
  the Auth0 `iss`, and the API's token validation is unchanged. `MCP_BASE_URL` is now read with
  trailing slashes trimmed on the MCP side too, so the two documents agree for that shape as well.
  Who observes: MCP clients and anything else reading the Auth0-arm metadata document; SAML/AS
  instances are unaffected. User-visible: yes
  (fresh MCP authorization succeeds on strict clients). Release relevance: signal-only.
pr: self
classes: behavioral
surfaces: http
status: signal-only
- **Resource erasure 2c: write doors refuse an erased resource with `410 RESOURCE_ERASED`, checked inside the write**
  The resource write doors (`PATCH /api/resources/{id}`, `PUT /api/resources/{id}/meta`,
  `PUT /api/ingest/{id}`, `DELETE /api/resources/{id}`, `POST /api/resources/{id}/provenance`,
  `POST /api/resources/{id}/artifacts`, `POST /api/facets` on a resource,
  `POST /api/resources/{id}/blocks`, `POST /api/resources/{id}/finalize`, the ingestion record of a
  segmented `POST /api/ingest`, each candidate of `POST /api/resources/reblock`, and the source of
  `POST /api/relationships`) check the caller's right to modify inside the write's own transaction,
  under a row lock, so a write racing an erasure or a soft delete either lands before it or is
  refused after it. A caller who holds standing on an erased resource (`resource_husk_held_by`, the
  read side's population) gets `410 RESOURCE_ERASED` where it got `403`; every other caller keeps
  `403`, and a soft-deleted resource keeps `403`. A refused caller takes no row lock on `DELETE`.
  A `PATCH` that sets a goal the caller may not link is refused as a whole: the title and body no
  longer land without the goal. Reblock addressed at an erased id answers a holder `410` (everyone
  else keeps `404`); a candidate erased under a running batch is a `denied` row. A segmented ingest
  replaying an idempotency key onto a since-deleted resource gets `403` where it got `404`. A
  principal with no emitter to resolve (a read-only machine client) is refused by the gate (`403`)
  on create and on these doors, where it got `500`. The relationship doors (`.../retype`,
  `.../reweight`, `.../fold`, `POST` and `DELETE` on `.../facets`) check the source resource the
  same way; an erased or deleted target keeps its `404`, and an edge that touched a since-erased
  resource was folded by the erasure and keeps answering `404`. `POST /api/resources/{id}/reassign`
  refuses a deleted or erased resource (`403`, or `410` to the owner of an erased one) where it moved
  it, and decides authority before anything about the id: an unknown id answers `403` where it
  answered `404`, and a cognitive-map-homed resource's `400` (now declared) answers only its owner,
  where every caller got it; `POST /api/teams/{id}/reassign` and `DELETE /api/teams/{id}/members/{profile_id}`'s
  `residual_owned` count and move live resources only. `POST`/`DELETE /api/resources/{id}/grants`
  refuse grant administration on a deleted or erased resource from every caller, its owner and a
  system admin included (`403`, or `410` to a holder of an erased one). A relationship assert into a
  target, a blob relation onto a resource peer, and a grant or revoke each take the resource's row
  lock inside the write, so one racing an erasure is refused (`404`, or `410`/`403` on the grant
  doors) instead of answering `500` or landing on it. `GET /api/resources/{id}/blocks/{block_id}`
  answers a holder of an erased resource `410 RESOURCE_ERASED` (everyone else keeps `404`; a folded
  block keeps its `410` `BlockRead`). `PUT /api/cognitive-maps/{id}` (reconcile) requires
  authorship of the map; the L0 kernel and maps joined to the gating team keep requiring a system
  admin. The `principal_erased` and `resource_erased` ledger payloads, and their registered payload
  schemas, no longer carry `propagated_to_clients` (`20261002000020`); no event carrying it exists.
  Every door that can answer `410 RESOURCE_ERASED` declares it in the contract, and
  `POST /api/resources/{id}/finalize` declares the `409` and `422` it answers; the SDKs regenerate.
  `PUT /api/cognitive-maps/{id}` declares its `400` and `401`, and append and finalize their `401`.
  Every authored write refuses an over-long authorship field with `400`, naming the field and the
  limit: `reasoning` and `rationale` at most 16384 bytes, `persona` and `model` at most 256.
  temper-client reads a `410` carrying `RESOURCE_ERASED` as a typed erasure, a block read included;
  every other `410` stays gone. The CLI reports the erasure under code `RESOURCE_ERASED`, and
  `resource delete`, `update` and `annotate` remove the local projected copy of the resource they
  addressed when they meet it; `resource show` reports it and leaves the vault alone. MCP tools and
  MCP resource reads report an erased resource as `invalid_params` naming the erasure, and an
  unknown or deleted id as `invalid_params` (`-32602`) where it was `internal_error` (`-32603`).
  Who observes: the owner or a grant holder of an erased resource; a caller whose goal link is
  refused; a team admin reassigning or removing a departing member; the owner or a system admin
  administering grants on a deleted resource; a caller reconciling a cognitive map; MCP and CLI
  users meeting an erased or unknown id; a caller reassigning an id it has no authority over; an
  agent sending an over-long authorship field. User-visible: yes. Release relevance: signal-only.
pr: self
classes: additive,behavioral
surfaces: http,mcp,clients,cli-stdout,schema
status: signal-only
- **The per-row machine-client, connection and subscription routes refuse a caller without authority as `404`, indistinguishable from a missing id**
  `GET`/`DELETE /api/machine-clients/{id}` and `POST …/rotate-secret`; `GET`/`DELETE
  /api/connections/{id}`, `POST …/credential`, `…/webhook-events`, `…/tool-manifest` and
  `POST`/`DELETE …/reach`; `GET`/`DELETE /api/subscriptions/{id}`. A caller who is neither a
  system admin nor the owner of the owning team (for subscriptions, an owner or maintainer of the
  authoring team) was answered `403` for an existing id and `404` for a missing one, so any
  approved bearer could probe which ids exist. Both now answer the same `404`, same body
  (`… not found or not readable`). The gates admit exactly whom they admitted before; only the
  refusal voice moved. Unchanged `403`s: `SYSTEM_ACCESS_REQUIRED`; `POST …/reach` from a caller who
  controls the connection but does not manage the receiving team (they can already read the
  connection, so nothing is disclosed); and the team-scoped creates (provision, issue, subscription
  create), where no row's existence is at stake. The machine-client missing-row message gains
  "or not readable", matching the other two families. openapi.json changes descriptions only on the
  twelve operations. Who observes: a caller without authority over a row, who now sees `404` where
  they saw `403` — over HTTP, in the SDKs, and in the `temper admin machine|connection|subscription`
  JSON error code (`forbidden` → `not_found`). User-visible: no. Release relevance: signal-only.
pr: self
classes: additive,behavioral
surfaces: http,clients,cli-stdout
status: signal-only
- **The scoped operator routes enter the OpenAPI contract: admin ledger, machine clients, connections, subscriptions**
  The routes gated `is_system_admin OR <a scoped role>` — `GET /api/admin/ledger`, `/api/machine-clients`
  (list, provision, get, revoke, issue, rotate-secret), `/api/connections` (list, provision, get,
  revoke, credential, webhook-events, tool-manifest, reach grant/revoke) and `/api/subscriptions`
  (list, create, get, revoke) — gain `#[utoipa::path]` documentation under four new tags
  (`Admin Ledger`, `Machine Clients`, `Connections`, `Subscriptions`) and leave the
  out-of-contract allowlist. They stay in `gated_routes`: a team owner, a team manager or the
  actor themself reaches them, so they are not admin-only. Every existing operation and schema is
  byte-identical; the contract only grows (20 new operations, their request/response schemas, four
  new API classes in each generated SDK). The temper-core `Subscription` row publishes as
  `ConnectionSubscription`, since the contract already carries the vault-config `Subscription`, and
  `Connection` publishes as `RemoteConnection`, apart from the SDKs' own HTTP connection. No
  route's path, method, gate, status codes or wire bytes change. Who observes: OpenAPI/SDK
  consumers, who can now call these families with a bearer that passes their gate. User-visible:
  no. Release relevance: signal-only.
pr: self
classes: additive
surfaces: http,clients
status: signal-only
- **The system-admin surface enters the OpenAPI contract, isolated as its own route group**
  The routes whose only authorization is the `&SystemAdmin` proof — `/api/access/admin/*` (the
  join-request and reconsideration queues and their counts, full settings, promote/demote, the
  four standing acts, auto-join reconcile, the profile directory), `POST /api/admin/erasure`,
  `POST /api/admin/resources/erasure` and their `/survey` doors, `POST /api/embed/admin/reembed`
  and `POST /api/machine-clients/{id}/rebind` — gain `#[utoipa::path]` documentation under a new
  `Admin` tag and move from `gated_routes` to a new `admin_routes` group at the same gated tier.
  `POST /api/admin/slack/links/disconnect` (already documented, `Slack Link` tag) moves with them
  unchanged. Every existing path's operation is byte-identical; the contract only grows (23 new
  operations, their request/response schemas, an `AdminApi` in each generated SDK). No route's
  path, method, gate, status codes or wire bytes change: `GET /api/access/admin/profiles` now
  returns an untagged enum that serializes exactly as the page or card it returned before. One
  request relaxation rides along: `RebindMachineRequest`'s `from_machine_client_id` (always
  overwritten by the path `{id}`) and `keep_old_active` (documented default `false`) gain
  `#[serde(default)]`, so a body may omit them; bodies that send them are read exactly as before.
  Who observes: OpenAPI/SDK consumers, who can now call the operator surface with an admin bearer.
  User-visible: no. Release relevance: signal-only.
pr: self
classes: additive
surfaces: http,clients
status: signal-only
- **Resource erasure 2b PR 2: an erased resource reads as `410 RESOURCE_ERASED` to a caller with standing; the operator erasure doors**
  `GET /api/resources/{id}`, `/content` and `/meta` (which composes from the same read) gain a
  `410` under the new code `RESOURCE_ERASED`, with a fixed message
  naming only the id. It reaches only a caller who holds standing on the erased resource — its
  owner, or a direct or team read grant (`resource_husk_held_by`, `20260930000060`). Every other
  caller, including a member of the resource's context with no grant, keeps the `404` an unknown
  id gets, byte for byte; a soft-deleted resource keeps its `404`. The three operations' contracts
  grow the `410`. Three write paths also change a response: a create's idempotent replay, update
  and annotate read the resource back through the same `show_view_select`, so a holder now gets
  `410` from them where it got `404`. Their declaration in the contract and their enumeration
  belong to build order 2c; their OpenAPI is unchanged here. An older client sees a
  generic `410` — temper-client's 410 arm reads it as `Gone` carrying the message, so MCP and the
  CLI receive a gone error where they received not-found; keying on the code, and the CLI and MCP
  rendering, are build order 2c. Two operator-only doors land out of the OpenAPI contract
  (`POST /api/admin/resources/erasure` and `/survey`, gated by `is_system_admin` — no tenant axis
  exists); a non-operator gets `404`. A non-operator at either erasure door pair, the principal
  pair included, is now rejected before dispatch with no ledger event: `POST /api/admin/erasure`
  no longer records an `unauthorized` `principal_erasure_refused`, and the `unauthorized` reason
  in both refusal vocabularies is retired (still registered). Who observes: the owner or a grant
  holder of an erased resource, and the instance operator. User-visible: yes. Release
  relevance: signal-only.
pr: self
classes: additive,behavioral
surfaces: http,clients,mcp,cli-stdout
status: signal-only
- **MCP list/read results carry `ttlMs` and `cacheScope`; edge trails carry edge-property events; lineage walks answer past depth 1**
  `tools/list`, `resources/list`, `resources/templates/list`, `resources/read` and the empty
  `prompts/list` gain the two fields MCP 2026-07-28 requires (SEP-2549) — additive keys on
  unchanged result shapes: `Public` / 5 min for the deployment surface (tools, templates,
  prompts), `Private` / 0 for caller data (resources). Who observes: MCP clients; a client
  enforcing 2026-07-28 (Claude Code) listed no tools before, older-protocol clients ignore the
  keys. Behind unchanged shapes, two ledger readers change what they return: `element_trail` for
  an edge now includes that edge's own property events (facet asserts, `anchored-at`,
  retractions), which it silently omitted; and `resource_lineage` walks breadth-first, visiting
  each node once, where it previously exceeded the function time limit (HTTP 502 / MCP gateway
  error) past depth 1 — it returns the same nodes at the same depths (a seed with a self-loop is
  no longer listed as its own lineage), now reports a live edge before a folded one when a node is
  reached over both, and walks nothing for an unknown direction (no caller passes one). Both are
  declared in their migrations (`20260930000010`, `20260930000020`, `20260930000030`;
  `20260930000040` refreshes one COMMENT). User-visible: yes, as fixes.
  Release relevance: signal-only.
pr: self
classes: additive,behavioral
surfaces: mcp,http,schema
status: signal-only
- **Single-ingress follow-on: HTTP-constructed `DbBackend` sites pass the proof they already hold — signatures, not behavior**
  The seam's constructor splits by what the caller genuinely holds: `DbBackend::with_proof`
  takes the surface's middleware-minted `AuthenticatedProfile` (the HTTP handlers, the
  citation-audit service signature, and the MCP steward tool construct through it), while
  `DbBackend::new` stays the CLI/operator Class E spelling — no middleware exists above that
  frame, so no proof exists to pass. The seam's three Principal-consuming gate sites
  (`record_citation_audit`'s `authorize::<AuditAuthority>`, `auditor_dispatch_tick`'s
  `require_machine_principal`, `complete_auditor_job`'s `authorize::<AuditorJobAuthority>`)
  dispatch through `DbBackend::principal`, which routes `Proof` for proof-holding callers and
  `Bare` for the CLI path — the same gate definitions, one shared spelling, the caller's own
  arm. `citation_audit_service::record_citation_audit`'s signature takes the resolved proof
  rather than a bare id. No SQL moved, no predicate reordered, no probe widened: for any
  caller the gate binds the same profile id into the same predicates it always did —
  `Bare` admits exactly what it admitted before, and a caller that cannot hold the proof
  never compiles the `Proof` path. The routing pin (`db_backend_principal_routing_test`)
  drives the same caller through BOTH spellings and demands the identical admission.
  Who observes: nobody — same gate, same predicates, same profile id; the proofs the gates
  receive are now the ones the surfaces minted.
  Review amendment (2026-09-29, RG-1/RG-2 passes on 9326cf09): the "all HTTP handlers
  construct through it" claim overshot by one site — the `genesis` handler still built
  `DbBackend::new` (behaviorally inert — `create_cognitive_map`'s admin probe reads
  `self.profile_id` directly, not through `principal()` — but falsifying the coverage
  claim). Fixed in the review-fixes commit: genesis now constructs `with_proof`, and the
  routing pin gained a second witness on the `complete_auditor_job` door.
pr: self
classes: behavioral
surfaces: http,mcp
status: signal-only
- **PR 3 of the single-ingress refactor: routes.rs collapses to a declarative route table — the audit scripts assert the table**
  A pure route-wiring refactor, declared because the wire-touched path moved: the twelve
  sub-router functions in `routes.rs` (1,226 lines) become per-group files under
  `crates/temper-api/src/routes/`, one table in `mod.rs` maps every group key to its
  auth tier, the tier's middleware stack is applied in exactly one place (`apply_tier`),
  and both app builders consume the same table (the duplicated internal-stack wiring in
  `create_app`/`create_internal_app` dies with it). The audit scripts re-point at the
  table — `audit-route-auth.sh` now pins each group's exact `(group, tier)` row (stronger
  than the per-builder layer greps it replaces) and `check-openapi-routes.sh` scans the
  module directory. Who observes: nobody — the group set, the middleware addition order,
  and every route's posture are byte-identical; openapi.json is byte-stable
  (check-openapi-routes + check-openapi-pin untouched and green); no signature, refusal
  dialect, or gate changes. The one comment corrected with evidence: the pre-table
  "INNERMOST" relay-trust comment mis-stated axum's layer ordering (last-added layer is
  outermost, per axum 0.8.9 `Endpoint::layer`/`PathRouter::layer`); the table's row
  comments state the true execution order — no behavior rides the correction.
pr: self
classes: additive
surfaces: http
status: signal-only
- **PR 2 of the single-ingress refactor: the Class F conditional write-gates and the Bare-site ledger consume the typed principal — signatures, not behavior**
  Every site in PR 1's disclosed residual ledger flips from `Principal::Bare` to
  `Principal::Proof` end-to-end: the thirteen production sites (connection_service
  get_for_caller/provision/revoke/authorize_live/grant_reach/revoke_reach,
  machine_client_service get_for_caller/revoke/rotate_secret, machine_authz
  authorize_registration/contain_reach, cogmap_service bind_team/unbind_team),
  with signatures cascading to the HTTP handlers, the sibling services
  (machine_registration provision/issue, team create_team), and the test call
  sites (proofs minted through `test_support::authenticated_profile_for`;
  assertions, seeding, and expect-messages unmodified). The three Class F
  conditional/composed gates keep their internal probes — `require_cogmap_write_admin`
  and `create_team`'s auto_join arm take `&AuthenticatedProfile` with their
  conditional shapes (the OR-arm, the field-conditional admin gate) preserved;
  bind/unbind run the same `TwoSidedAuthority`. Class E: `reblock_resources`'s
  deployment-wide arm now calls the new `auth::require_system_admin_by_id` — the
  gate DEFINITION, with `require_system_admin` (the surface) and the seam its two
  callers; genesis's positive admin question keeps calling the `is_system_admin`
  owner directly (a refusal wrapper would swallow a DB error into "not an
  admin"), with the seam parity named in a comment. `require_machine_principal`
  consumes the `Principal` enum; the db_backend seam passes `Bare` — the bare-id
  DB probe retained there, the one provenance re-derivation outside the seam now
  typed. No `Principal::Bare` construction remains on an HTTP-reachable path
  except the db_backend seam. The 5b.4 grant-axis escalation guard citation
  (`authz/grant.rs`) is untouched. Every refusal dialect, probe ordering, probe
  cost, and wire shape byte-identical; no openapi.json movement. Who observes:
  nobody — a caller that cannot hold the proof cannot compile the call, which is
  the point.
pr: self
classes: behavioral
surfaces: internal
status: signal-only
- **PR 1 of the single-ingress refactor: the 13 Class A+B authz ladders and read-visibility services consume `&AuthenticatedProfile` — signatures, not behavior**
  The seven Class A `ScopedAuthority::resolve` ladder impls (authz grant, machine,
  read_gates ×2, two_sided, context_admin, subscription) and the six Class B
  read-visibility sites (machine-client/subscription/connection `list`,
  context `list_retired_administered`/`get_retired_administered`, admin-ledger
  `readable_event_types`) take the Level-1 proof at their signatures; the internal
  probes stay, with their orderings (membership-first, self-read-first,
  object-side-first) untouched and their asserting tests' assertions unmodified. The
  trait carries the caller as the new crate-internal `Principal` enum
  (`Proof(&AuthenticatedProfile)` / `Bare(ProfileId)`) — the sealed-proof boundary
  at the service signatures, with `Bare` the db_backend seam's door (Class E, the
  CLI path has no middleware above it) and the Class F conditional gates' spelling
  until their own PR. Direct human-path callers (grant/revoke capability, team
  detail, context share/unshare/reassign/rename/retire/restore, subscription
  create/revoke/get, ledger reads, delivery reads) take the proof and handlers pass
  `auth.0` through; MCP's `ensure_profile_from_parts` hands its tools the proof it
  already held instead of discarding it. The two `AuditAuthority`/`AuditorJobAuthority`
  impls migrated to the trait's new `Principal` signature to keep it compiling (their
  only production caller is the Class E seam, still `Bare`); the test call sites were
  mechanically adapted (proofs minted through the real Level-1 gate, `Principal::Bare`
  at the ladder-level tests) — assertions, seeding, and expect-messages unmodified.
  DISCLOSED RESIDUAL: besides the two Class E/F seams above, ten further
  conditional-gate sites on HTTP-reachable paths run on `Bare` with a proof available
  one frame upstream (connection_service get/provision/revoke/authorize_live/
  grant_reach/revoke_reach, machine_client_service get_for_caller/revoke/
  rotate_secret, machine_authz authorize_registration/contain_reach) — zero behavior
  change at each (SQL byte-identical), but they are the sites a future Proof-only arm
  would silently not reach; they ride PR 2's ledger. Every wire shape, tool
  declaration, refusal dialect, and probe cost byte-identical; no openapi.json
  movement. Who observes: nobody — a caller that cannot hold the proof cannot compile
  the call, which is the point.
pr: self
classes: behavioral
surfaces: internal
status: signal-only

- **Beat G4: the last mixed direct cluster crosses the network door — reblock, blobs, segmented ingest, data_artifacts(+shapes) forward as relays; the direct binding is steward-and-nothing-else**
  The twelve direct handlers (reblock 1, blobs 2, the consolidated segmented-ingest
  tool's 4 actions, data_artifacts 3, shapes 3) stop executing in-process and forward
  to their deployed routes as temper-client relays — caller's bearer re-issued,
  service credential + `mcp` carrier as default headers, refusals mapped arm-for-arm
  from the preserved bodies through the one `AcrossAuth` idiom. Tool names, schemas,
  descriptions byte-identical (the declaration witness holds). The retained resolver
  (the G3d pinned pattern, adopted per Pete's 2026-09-27 ruling for reblock's
  context scope): the `@me/…`-grammar ref resolves in-process, visibility-gated, from
  the one validated decode — resolver and forwarded act cannot disagree on identity.
  `build_create_command` + `provenance_body` die whole with their last caller, and
  temper-mcp's `temper-substrate` dependency dies with them.
  SIX DECLARED PARITY DELTAS (the G3c delta format; pinned green against the direct
  binding first, flipped in the swap commit, named in the tool modules' headers and
  the parity suite's header — these three sites agree): (1) finalize's
  expectation-mismatch Conflicts (`expected_blocks` / `expected_body_hash`) — the
  direct catch-all's `internal_error` renders the wire 409's `invalid_params` with
  the server's own sentence, prefix and `Conflict: ` label stripped; (2)
  `get_data_artifact`'s absent face — the direct 200-text posture ("Artifact not
  found or not visible to you.") renders the flat route's 404 `invalid_params` with
  the server's sentence; (3) `get_data_artifact_shape`'s absent face — same flip
  ("shape not found"); (4) the blob read/commit/relate not-found prefixes
  (`blob_commit: ` / `blob_relate: ` / `blob_read: `) drop for the server's bare
  sentence (kind and gate identical); (5) the blob read result's `content_hash` is
  now the collected bytes' own sha256 — the wire read carries no hash header, the
  proof a whole read is what was committed moves from row attestation to
  self-attestation; (6) the consolidated `segmented_ingest`'s `begin` becomes
  USABLE — the flattened-create collision (serde binding the outer `content` slot
  over `create.content`) refused every begin-by-the-advertised-shape call, the
  dispatcher now reshapes explicitly (the in-band repair row below). The append
  occupied-seq face is NOT a delta: both sides refuse `internal_error` (the append
  route bridges the raise generically) — named at the pin. The blob read ceiling
  stays byte-stable (the tool's own sentence and numbers). Accepted residuals
  (accept-and-name, per the G3b convention): the relay's non-idempotent-retry
  posture can double-assert a blob relation on a timed-out-but-landed write (the
  caller's own redrive has the same property — design-ruled); the shapes-declare
  route's act-envelope drop (both doors, filed
  [01a0e2f0-5bdc-7b00-94d2-0fbf9d141df0](https://github.com/tasker-systems/temper/issues/));
  the source gate's shape heuristic and the 400-label sweep's coverage are
  unchanged. Who observes: an MCP-calling agent — every refusal face above renders
  the server's or the tool's own sentence with kind and gate identical; the
  ledger's `<handle>@mcp` attribution is witnessed on a real commit through the
  door (the family's own witness test, bite-proven from the relay-client builder:
  stripping the carrier reddens it with `@web`; a dead relay base URL reddens the
  cluster — the door is the only path).
pr: self
classes: behavioral
surfaces: mcp
status: signal-only

- **Two MCP wire-shape defects repaired in-band ahead of beat G4's swap — the consolidated segmented-ingest `begin` becomes usable, and blob commits accept whitespace-wrapped base64**
  The beat G4 parity ground pass probed both red. (1) The consolidated
  `segmented_ingest` tool's `begin` was unreachable on the wire: the outer `content`
  field (the append step's) shares the JSON key with the flattened `create.content`,
  and serde's outer-first binding left `create.content` `None` for every
  begin-by-the-advertised-shape call, which the surface-side integrity check then
  refused with "ingest_begin requires content". The dispatcher now reshapes the wire
  input explicitly (outer `content`/`sources` honor as begin's segment text and
  sources when the flattened side is absent — an explicitly nested field is never
  overwritten). Tool declaration byte-identical; the repair is behavior, not
  declaration. (2) `blob_manage` commit's base64 `content` refused any whitespace
  (encoders wrap at 76 columns; an agent hands the tool one long wrapped string) —
  the decode now strips ASCII whitespace first while keeping the alphabet strict
  (URL-safe symbols and missing padding still refuse). Ruled 2026-09-27, Pete: both
  fixed in-band with the parity work rather than preserved into the swap —
  preserved-defect parity would have pinned a caller-visible refusal nobody defends.
  Who observes: an MCP-calling agent, whose usable-begin and tolerant-decode are the
  point.
pr: self
classes: behavioral
surfaces: mcp
status: signal-only

- **The shapes cogmap-home pair and the flat artifact read cross the wire — route-first for the beat G4 MCP door crossing**
  Three additive routes close the two per-method coverage gaps the beat G4 grounding
  found between the direct MCP tools and the wire: `GET/POST /api/cognitive-maps/{id}/shapes`
  (the cogmap-home twin of the context shapes pair — the MCP
  `list_data_artifact_shape`s / `declare_data_artifact_shape` tools admit a cogmap
  `home_type`, the substrate read/write are home-generic, and until now only the
  context arm had a door) and `GET /api/data-artifacts/{artifact_id}` (the flat
  visibility-gated read — the MCP `get_data_artifact` tool takes only the artifact id
  and answers folded rows, a posture the nested resource-scoped route cannot carry and
  the tool's declaration cannot grow a `resource_id` field for). All three mirror their
  context/nested twins' gates arm-for-arm (visibility-gated reads, authoring-authority
  403 on the declare, uniform 404 not-visible posture on the flat read) and ride
  temper-client as `list_cogmap_shapes` / `declare_cogmap_shape` / `get_by_id`. One
  behavior-neutral amend rides with them: both declare routes now resolve the emitter
  from the request's resolved surface (`surface.marker()`) instead of the hard-coded
  `web` literal — byte-identical for every existing caller (web degrades to `web`),
  and the correct `@mcp` attribution once the MCP tools forward their declares through
  the door at the beat G4 swap (the direct tool's in-process `resolve_emitter(…, "mcp")`
  is what this preserves). Who observes: a REST/SDK caller gains capability (additive);
  no existing caller-visible face changes.
pr: self
classes: additive
surfaces: http, mcp
status: signal-only

- **The cognitive_maps + contexts families cross the network door — the register's largest cluster executes as real relayed calls**
  Ten tool groups stop executing in-process and forward to their deployed routes
  (`/api/cognitive-maps/…` read/genesis/materialize/bind/grant routes, the
  `/api/contexts/…` read/manage/materialize routes, `/api/invocations` and its
  read/close routes; describe_schema is pure compute and swaps shape-only) as
  temper-client calls, exactly as the four families before them: the caller's
  bearer re-issued, the service credential and the `mcp` carrier set by the relay,
  refusals mapped arm-for-arm from the preserved bodies, and the direct methods'
  `ensure_profile_from_parts` gates deleted whole — Level 1 + 2 execute at the API.
  The `Surface::Mcp` origin fields the direct commands carried die to the carrier:
  the ledger's `<handle>@mcp` emitter attribution is witnessed on a real open
  through the door (the family's own witness test). The ONE retained in-process
  read is the context-ref resolver (`@me/<slug>` is MCP-local input shaping whose
  `@me` only exists at this surface — the resources family's retained-resolver
  precedent), and `cogmap_read_charter` crosses by PROJECTION: the door's charter
  view is the show route plus a field projection, no additive route. Tool names,
  wire schemas, and tool descriptions are byte-identical. Who observes: an
  MCP-calling agent, whose visible changes are the declared parity deltas named in
  the parity suite's header — an unreadable-anchor materialize_delta, a re-closed
  invocation (409), and a team-owned create by a non-manager now render
  `invalid_params` with the server's/tool's own sentence (previously the direct
  catch-alls' `internal_error`); not-found and conflict refusals drop the direct
  maps' `{action}: `/`{context}: ` prefix and the `Conflict: `/`Bad request: `
  status labels (kind and gate identical on every previously-pinned face; a few
  unpinned catch-all faces changed kind to `invalid_params` with the door, named
  in the tool files' headers); an unreadable map's charter view flips from the
  direct 200-empty to the show route's 404 sentence (deny-is-an-error travels
  with the projection); an outsider's invocation `show` flips from the direct
  deny-with-null to the route's uniform 404 sentence (the `list` read keeps its
  deny-with-data posture); and MCP `get` of a retired-but-administered context
  now answers the row — the route's restore-reachability fallback behind the
  door, where the direct `get_visible`-only read refused (the caller's own
  administered data, identical to the CLI's read). The disclosure-dialect arms
  (the detailed/terse authorship 403s, the share/rename requirement sentences)
  are byte-stable, and describe_schema runs no gate beyond the MCP edge's JWT
  validation — static product vocabulary, never tenant data (recorded at the
  swap's review round).
pr: self
classes: behavioral
surfaces: mcp
status: signal-only

- **The resources and search families' 400 faces drop the status label — the `api_error_cause` strip reaches the door's first two families**
  The seven relayed 400 arms (create/list/update/annotate/update_resource_meta/
  delete on resources; search's caller-error arm) now speak the server's own
  sentence bare via the shared `api_error_cause` strip, exactly as the
  ledger/graph families have since G3c. The relayed 400 body carries the API's
  rendered Display (`Bad request: …`), a label the direct bindings never
  rendered — they destructured the variant — so the label was a
  door-introduced artifact: the latent direct-parity gap Pete ruled swept
  (goal Amendment 2026-09-26, ruling 2). Search's arm also drops its
  `search: ` context prefix, so no caller reads a label stacked on a label.
  Query contributes no face: its 400 arrives typed as `PlanRefused` with its
  own rendering, untouched. Both parity suites declare the delta in their
  headers and their swept faces' pins flipped in the same commit. Who
  observes: an MCP-calling agent, whose visible change is the dropped
  label/prefix prose — refusal KIND and gate identical (`invalid_params`
  either way).
pr: self
classes: behavioral
surfaces: mcp
status: signal-only

- **The temper-mcp service's shared profile cache is torn down — per-request identity, no auth state on the service**
  An internal identity-plumbing refactor of the MCP service, declared because the
  wire-touched path moved: the service struct's last auth state (the `profile`
  slot and its lock) is deleted; `ensure_profile_from_parts` stops caching and
  RETURNS the resolved profile, and every still-direct tool function receives it
  as a parameter instead of reading the shared slot. On a stateful deployment the
  slot was a cross-request wrong-identity TOCTOU (request A's gate fills it,
  request B's gate overwrites it, A's tool fn reads B's profile); with the field
  gone, identity is a per-request value and the compiler makes cross-request
  bleed unrepresentable. Who observes: nobody — behavior-neutral per request
  (the same Level 1 + 2 gate runs at the top of every direct method), and no wire
  shape moves: tool names, inputs, outputs, and refusal sentences are byte-stable.
pr: self
classes: additive
surfaces: mcp, internal
status: signal-only

- **The ledger/graph MCP tools execute through the network door — element_trail, facets, relationships, and the citation audit cross as real relayed calls**
  The four remaining ledger/graph tool families stop executing in-process and forward
  to their deployed routes (`GET /api/graph/elements/{kind}/{id}/trail`,
  `POST /api/facets`, the `/api/relationships/…` set/read/retract routes, and the
  block-addressed `POST /api/citation-audits`) as temper-client calls, exactly as the
  resources, search, and query families have since the door opened: the caller's
  bearer re-issued, the service credential and the `mcp` carrier set by the relay,
  refusals mapped arm-for-arm from the preserved bodies, and the direct methods'
  `ensure_profile_from_parts` gates deleted whole — Level 1 + 2 execute at the API.
  The MCP input's act envelope maps straight through into each request body (the
  audit's `BlockCitationAuditRequest` is the tool input 1:1), so authorship and
  correlation ride the wire act, and the audit's three-cause 404 sentence stays the
  tool's own fixed string — the one-string-by-construction property survives the
  wire. Tool names, wire schemas, and tool descriptions are byte-identical. Who
  observes: an MCP-calling agent, whose visible changes are the two declared parity
  deltas named in the parity suite's header — a closed-invocation 409 now renders
  `invalid_params` with the server's own sentence (previously `internal_error` — no
  `Conflict` arm in the direct map), and not-found refusals drop the direct map's
  `{action}: ` prefix (the door carries the server's own sentence; kind and gate
  identical). The audit's fixed sentence and the terse/detailed 403 faces are
  unchanged.
pr: self
classes: behavioral
surfaces: mcp
status: signal-only

## Shipped in v0.5.4
- **This release — the 0.5.4 fleet alignment: VERSION 0.5.3 → 0.5.4 across crates, packages, and clients**
  The release train's own wire delta is none: version fields and the generated
  cores re-stale with the bump (the D-S3 baseline — no shape movement); the
  P floor rides the additive rows already in this window.
pr: self
classes: additive
surfaces: http, clients
status: signal-only

- **temper-telemetry-ts aligns at 0.5.4 — the npm pair co-releases, and the lane's guard enforces it**
  A version-site change, not a wire change: `publish-npm.sh` releases
  `temper-ts` and `temper-telemetry-ts` as a pair at one requested version,
  and its version-agreement guard refused the v0.5.4 lane when
  `temper-ts` moved and `temper-telemetry-ts` (whose generated types did not
  change this window) stayed at 0.5.3. The manifest aligns so the pair ships
  together; the package content is unchanged from 0.5.3. Who observes it: the
  npm registry listing only.
pr: self
classes: additive
surfaces: clients
status: signal-only

- **The citation audit gains a block-addressed write — POST /api/citation-audits**
  The audit write's second route: the body names only the `(block, source)` citation
  pair plus the act envelope, and the server derives the authorization subject from
  the block exactly as the direct path always has — no finding is ever caller-named,
  so nothing can transpose. Who observes it: any bearer-authenticated client; the
  same `AuditAuthority` gate answers through both routes (one `authorize` call, the
  same three-cause 404 sentence, byte-identical by the new route-level equivalence
  test), and the request type carries the act fields the finding-addressed body
  type predates, so a write here never silently drops authorship or correlation.
  Because the act envelope is caller-carried here for the first time on an HTTP
  audit write, the route additionally answers the act gate's own arms — a 404
  naming an unknown invocation and a 409 for a closed run — which the
  finding-addressed route's always-empty act keeps unreachable; both sit after
  `authorize`. No existing shape moves: the finding-addressed route, its request
  type, and every response are untouched; the temper-client method
  (`record_citation_audit_for_block`) and the generated SDK skins grow additively
  around it. CLI-observable text only: the `CloudBackend::record_citation_audit`
  stub's refusal message rewrites to point at the new route (the structural
  impossibility it recorded no longer holds).
  Release-relevant as additive surface only — a client that never calls the new
  route observes nothing but the CLI stub's wording.
pr: self
classes: additive
surfaces: http, clients, cli-stdout
status: signal-only

- **The eight remaining `$ref`-carrying MCP tool declarations go fully inline — no `$ref`, no `$defs` on the wire**
  The advertised input schemas of `create_resource`, `segmented_ingest`,
  `update_resource`, `update_resource_meta`, `commit_data_artifact`,
  `declare_data_artifact_shape`, `context_manage` and `run_query` now inline the
  four temper-core type clusters they referenced — `ManagedMeta`, `KindOwnerInput`,
  `ContextOwnerRef`, and the 21-type composition cluster — instead of carrying a
  `$defs` block plus `$ref`s into it, completing the rule the scalar-enum row began:
  every one of the 38 served declarations is self-contained, enforced by a new
  router-wide guard (`every_served_tool_input_schema_is_self_contained`) rather than
  per-tool witnesses alone. Who observes it: any client that does not resolve
  `$ref`/`$defs` now sees the concrete object shape — `type: object` with named
  properties — where it previously saw a reference it could not resolve (the
  client-side string-encoding failure the client matrix's row 1 filed). No request or
  response class changes shape: the tools accept and return exactly what they did,
  and the inlined schema is JSON-Schema-equivalent to what it replaced.
  Release-relevant as a schema-payload change only; the byte-witness fixture was
  regenerated AS this declaration change and the query-ceilings test follows the
  inlined pointers.
pr: self
classes: behavioral
surfaces: mcp
status: signal-only

- **rmcp 1.8 → 3.4.1 — the SDK behind the MCP surface jumps two majors; 2026-07-28 enters the advertisement; the wire is held byte-identical**
  The dependency behind every MCP tool, resource, and initialize response moves from
  rmcp 1.8 (Jun 2026) to 3.4.1 (Sep 2026, MCP spec 2026-07-28). What changes behind an
  unchanged shape: `with_stateful_mode` → `with_legacy_session_mode`, `Content` →
  `ContentBlock`, `ServerInfo`/`ClientInfo` → `ServerConfig`/`ClientConfig`, resource
  raw-wrappers flattened, and `supported_protocol_versions()` overridden to the explicit
  pair `["2025-11-25", "2026-07-28"]` — not the SDK default (every version the SDK knows,
  back to 2024-11-05) and not a 2026-07-28-only list (which would strand every legacy
  client: negotiation echoes a legacy client's request only when the server supports it,
  and rejects when it supports no initialize-handshake version). Who observes: a legacy
  client, nothing — the byte-stability witness pins the full 38-tool `tools/list` response
  byte-identical across the jump and the initialize negotiation observable (legacy,
  ancient, and modern requested versions all answered `2025-11-25`) is held; a modern
  client may now negotiate the 2026-07-28 lifecycle (per-request metadata instead of the
  handshake), which is the beat's deliberate flip and the only new wire capability.
  Release relevance: none beyond the additive capability — no shape moves, no schema
  moves, the dogfood probe against the deployed 3.x build completes the beat's evidence
  after merge.
pr: self
classes: additive
surfaces: mcp
status: signal-only

- **The MCP search and query tools execute through the network door; a relayed composition is measured once, by the door it arrived on**
  The `search` and `run_query` MCP tools stop calling the service-direct read path and
  forward to `POST /api/search` and `POST /api/query` as `temper-client` calls, exactly
  as the resources family has since the door opened: the caller's bearer re-issued, the
  service credential and the `mcp` carrier set by the relay, refusals mapped
  arm-for-arm from the preserved bodies (`PLAN_REFUSED` reconstructed into the tool's
  every-refusal rendering; the direct methods' `ensure_profile_from_parts` gate deleted
  whole — Level 1 + 2 execute at the API). Tool names, wire schemas, and tool
  descriptions are byte-identical. Who observes: an MCP-calling agent, whose one
  visible change is the degenerate-embedding search refusal — previously wrapped as an
  internal error, now `invalid_params` carrying the server's own sentence (a declared
  parity delta, named in the parity suite's header; expired-in-flight, machine-gate,
  and deactivation arms are unchanged, witnessed through the preserved 401 bodies).
  Alongside, `/api/query`'s composition-shape measurement skips its `door=http` event
  when the request arrived relayed (`RelayedSurface` planted by `relay_trust`, which
  requires the service credential AND the honored carrier): the MCP edge records the
  same composition as `door=mcp` pre-relay, so one act is measured once by the door it
  arrived on, and a forged carrier without the credential degrades to measurement, not
  suppression. openapi.json does not move: no schema, route, or status changes; the new
  extractor reads a server-side extension, not the wire.
pr: self
classes: behavioral
surfaces: http, mcp
status: signal-only

- **The MCP skill projection moves to `skills/temper-knowledge-base/` — the temper-mcp gate's fixture path follows**
  A distribution change, not a wire change: the committed MCP packaging moves to the
  skills.sh scanner's priority container dir so `npx skills add tasker-systems/temper`
  surfaces it (previously the scanner's priority pass satisfied itself on the
  repo-internal `.claude/skills/` trees and the skill was unreachable). The
  `every_tool_the_shipped_skill_names_exists_in_the_router` temper-mcp test reads the tree
  by path; the path moved, the walked file set is byte-identical (git rename at 100%), and
  the router is untouched. No route, field, schema, or tool surface moves; openapi.json is
  static.
pr: self
classes: behavioral
surfaces: mcp
status: signal-only

- **Coverage floors — per-crate thresholds on the nightly report; the PR path stays coverage-free**
  A hygiene change, not a wire change: coverage-thresholds.json floors every crate against the
  lcov the nightly emits (check-coverage-floors.py, its mutation harness in guard-tests), so one
  crate's drift is named, not averaged away. A temper-api test file lost a suppression the lint
  no longer fires, which is where this path appears; no route, field, schema, or tool surface
  moves.
pr: self
classes: behavioral
surfaces: http
status: signal-only

- **Auto-join enrollment materializes at the standing committer; operator reconcile verb**
  A team flagged `auto_join_role` is an always-complete "everyone pool"
  (migration `20260629000002`'s header), but since the enrollment trigger was dropped
  (`20260722000100`) the pool only grew through the request-review door: `admin access
  approve`, admin promotion, and reactivation conferred standing while enrolling nowhere,
  so approved principals silently drifted out of `everyone` (and any other auto-join team).
  The standing committer (`principal_standing_apply`) now materializes the enrollment in
  the standing transaction — enroll while `has_system_access` — so every approval door
  inherits it. The pool is append-only, decided rather than inherited: no standing
  transition removes an auto-join membership, because post-D11 those rows are owned by
  other authorities (D14 machine hygiene enrolls born-`denied` machines, D17/D11
  revocation deliberately preserves grants and memberships, IdP-source rows are owned by
  SAML reconcile); the invariant reads one-directional — every standing-approved profile
  is a member — and enrollment defers to existing rows (`DO NOTHING`) rather than
  rewriting roles. Alongside, an operator repair
  verb exists for instances that drifted before this change: `POST
  /api/access/admin/auto-join/reconcile` and `temper admin access reconcile-auto-join`
  converge every auto-join team to the standing-approved population, report each
  (team, profile) pair added, and name any touched team that also carries SAML group
  mappings — those teams' new native rows permanently pre-empt IdP role assertions for
  the pairs written (native-wins-skip), and the verb warns so the operator sees the
  conversion the repair is making. Who observes: approving a principal by any door now puts
  them in every auto-join team at the team's `auto_join_role`, atomically; revocation and
  deactivation leave rosters exactly as they were (stale rows are harmless under D18 —
  membership confers no access, and admission is denied at every surface); an operator of
  a drifted instance gets a
  one-command convergence that names what it added. The route is on the operator-only
  undocumented surface (no openapi movement); the ts-rs `access.ts` tree gains
  `AutoJoinReconcileRow`. Server-to-server and library callers see the standing committer's
  return value unchanged.
pr: self
classes: additive, behavioral
surfaces: http, cli-stdout, clients
status: signal-only
- **The network door — the MCP resources family executes through the deployed API as a real HTTP relay; embedding readiness rides the gated reads**
  The MCP function stops hosting any part of the API: it authenticates the caller at
  its own edge (unchanged JWT verification, `list_tools` included), then forwards
  every tool act over HTTPS to the API function as a `temper-client` HTTP call —
  re-issuing the caller's bearer and presenting a NEW dedicated service-to-service
  secret (`TEMPER_MCP_SERVICE_SECRET`, never shared with the embed-dispatch secret).
  At the API, a relay-trust middleware validates that secret; beside a valid one, the
  `X-Temper-Relayed-Surface: mcp` carrier inserts a server-side extension that
  attributes relayed acts `@mcp` — trusted only with the credential, allowlisted to
  the single value `mcp`, and never an authorization input. Who observes: an
  MCP-calling agent, whose tool names, wire schemas, and response shapes are
  unchanged; refusal kinds held arm-for-arm across the hop, pinned by a parity suite
  re-harnessed across a real wire (the relay side of the MCP function is stateless —
  no router, per-request bearer from the request parts; it STILL holds a small DB
  pool for the not-yet-migrated direct families and the edge's profile resolution,
  so "the MCP function" today carries both shapes — the pool's blast radius belongs
  to those families' migration, not the door). Alongside, two refusal-voice
  widenings, message strings only: the API's machine-credential 401 body names the
  refusal it carries instead of the generic token sentence, and `temper-client` gains
  a body-preserving 401 variant so the MCP surface can map post-edge refusals to
  their own sentences (the CLI keeps its current 401 rendering). And the
  embedding-status enrichment rides the gated resource reads themselves (B1): the
  route, its client method, and its allowlist entry come OUT; `ResourceView` carries
  `embedding_status` — absent unless the `embedding-status` section is requested —
  and `GET /api/resources/{id}` gains an additive `?sections=` whose no-query answer
  is byte-identical, with the MCP `EnrichedResource` wrapper dissolved into the view
  that carries the field. Per-route body limits on the tool-carrying API endpoints
  rise to the MCP edge's 25 MB contract, so the network door does not 413 work the
  direct binding performed; the edge's 25 MB stays the one user-visible ceiling.
  openapi.json moves additively only: a new `EmbeddingStatus` schema, a fourth
  `ResourceSection` value, one optional view field, one optional query parameter; no
  rename, no removal, no type change on any existing shape. The arc-boundary review
  pass (RG-1 + RG-2) tightened four voices inside those declared surfaces, no new
  shape: `delete`'s post-edge 401s speak the arm sentences (previously an internal
  fault with a CLI login hint), the JWKS-outage 401 maps to a retryable internal
  voice (previously the catch-all's terminal framing), the relay pool refuses
  redirects (reqwest's default would replay the service credential and bearer to any
  3xx target), and the CLI `show --with embedding-status` actually requests the
  section (previously parsed, documented, silently dropped).
pr: self
classes: additive, behavioral
surfaces: http, mcp, clients, cli-stdout
status: signal-only
- **The data-artifact refusal surface — declined writes travel as typed 400s, explicit `kind_owner` reaches the SQL layer, CLI `--kind-owner`**
  A data-artifact write the system declines for reasons the caller can act on — the shape
  registry's SQL refusals and the enforcing-shape commit verdict — was mapped onto the
  internal-error class, so the refusal's vocabulary never reached any caller: HTTP answered
  500 with the generic body, and the MCP commit tool spliced raw database text into an
  internal error. Those writes now travel as `400` under a new wire code,
  `DATA_ARTIFACT_REFUSAL`, carrying the refusal's own words; `temper-client` parses the code
  into a typed error (rendered bare at the CLI), and the MCP tools render it as a caller
  error. Alongside, an explicitly named `kind_owner` was silently dropped before the SQL
  defaulting arm on both the shape-declare and artifact-commit wires — the SQL read
  flattened keys the Rust layer never wrote — so an empty context was undeclarable with an
  explicit owner; the flatten now rides both wires. The CLI gains `--kind-owner`
  (`kb_profiles:<uuid>` | `kb_teams:<uuid>`) on `data-artifact commit` and `schema declare`,
  the parameter the other two doors already accepted. Who observes: callers of the
  data-artifact write routes — a refusal that was an opaque 500 now arrives as a 400 naming
  what failed (an old temper-client still sees the text, under its generic non-2xx
  rendering); a caller naming an explicit namespace has it honored instead of silently
  defaulted; a CLI user can declare against an empty context. Genuine server faults on these
  paths still answer 500 — classification keys on the refusal's SQLSTATE + message prefix
  and the typed substrate error, never on the envelope. No route, field, or schema shape
  moves; openapi.json and the ts-rs trees are static.
pr: self
classes: additive, behavioral
surfaces: http, mcp, cli-stdout, clients
status: signal-only

- **The in-process door — temper-client transport, audience-acceptance parity, surface-through-the-door attribution (one-seam goal, beat G1)**
  The MCP door's migration onto the API's own execution path begins: temper-client
  gains an in-process transport (`Router::oneshot`, no socket) whose requests carry
  the caller's surface as a trusted request extension — the one channel a remote
  caller cannot write — and the API's `require_auth` accepts the same audience set
  the MCP middleware accepts, from one shared definition
  (`AuthConfig::accepted_audiences`), so an `mcp_audience` token authorizes
  identically at either door. Who observes: a caller holding an mcp_audience
  bearer, previously 401'd by the HTTP door, now authenticates there; every other
  acceptance is unchanged (unknown audiences still refused; the `X-Temper-Surface`
  header allowlist untouched — `mcp` stays untrusted as a wire claim). No wire
  shape moves: no route, field, or schema changes; openapi.json static; the
  header channel and its degradation rules are byte-identical.
pr: self
classes: behavioral
surfaces: http, mcp, clients
status: signal-only

- **The honest-absence signals — `Scoring.score_present` beside the deprecated zero rendering, `temper-stage-present` beside the empty string**
  A hit whose row carried no quantity, and a task carrying no stage, render
  absence as values on the wire today (`score: 0.0`; `temper-stage: ""`). Both
  renderings now carry the fact beside them: the wire keeps each legacy
  rendering byte-for-byte — documented in the contract as the deprecated
  rendering of absence, retired only at the reserved break level — while one
  additive sibling per shape states which values are measurements
  (`Scoring.score_present`, always emitted by current servers, optional in the
  contract; `temper-stage-present` on task-shaped CLI stdout). Who observes it:
  API consumers and the three generated skins reading `/query` responses, and
  external parsers of task-shaped CLI stdout — old readers parse everything
  unchanged, new readers can distinguish a measured zero from no quantity.
  Additive on the wire (one new optional property per shape; nothing moved,
  omitted, or nulled) + behavioral (the legacy renderings are now documented
  deprecated in the contract — a meaning change behind an unchanged shape).
pr: self
classes: additive, behavioral
surfaces: http, cli-stdout, clients
status: signal-only

- **The client closure becomes registry-consumable — the six crates publish as `temperkb-*` from CI**
  A distribution change, not a wire change: the published-side package names of
  `temper-core`, `-client`, `-principal`, `-workflow`, `-auth`, and `-telemetry`
  move to the `temperkb-` prefix (crates.io's `temper` and `temper-core` are
  unrelated projects), while every LIBRARY keeps its `temper_*` name via an
  explicit `[lib] name`, so no `use temper_core::…` — in this workspace or in a
  downstream consumer — moves. The internal manifests gain no dependency edge:
  path dependencies are rewritten to the new keys with the same paths, feature
  forwards follow, and all 16 members inherit the workspace lockstep version
  (one line per release bump). Server-side internals are locked out of the
  registry by an explicit `publish = false` rather than by the absence of a
  publish step. Who observes it: a Rust consumer outside this monorepo — first,
  an external Tauri app — resolves `temperkb-client` and its closure from
  crates.io at semver, the Rust counterpart of the existing PyPI/npm/RubyGems
  lanes; nothing that talks to a deployed temper changes a byte.
  Release-relevant as the enabling change for the crates.io lane in the release
  workflow.
pr: self
classes: additive
surfaces: clients, internal
status: signal-only

## Shipped in v0.5.3

- **This release — the 0.5.3 fleet alignment: VERSION 0.5.2 → 0.5.3 across crates, packages, and clients**
  The release train's own wire delta is none: version fields and the generated
  cores re-stale with the bump (the D-S3 baseline — no shape movement); the
  P floor rides the additive rows already in this window.
pr: self
classes: additive
surfaces: http, clients
status: signal-only

- **The open_meta key delete verb — an explicit `null` value on an update's `open_meta` deletes the key**
  A resource's open_meta key could be set but never removed: an explicit `null`
  was silently dropped by every write surface — the update exited ok and the
  key survived — so a metadata correction meant delete-and-recreate the whole
  resource (ids, provenance, embeddings all move). An update PATCH (CLI
  `--open-meta`, HTTP `open_meta`, MCP `update_resource`/`update_resource_meta`)
  now treats `{"key": null}` as key deletion, folding the key's live property
  rows via the new `property_unset` ledger event; on CREATE a null-valued key
  is refused 400 naming the key instead of the silent drop. Callers observe it
  on every write surface: same request class, success-changes-stored-state.
  The openapi movement is description-only (`open_meta`'s field description
  names the verb); no shape change.
pr: self
classes: additive, behavioral
surfaces: http, mcp, cli-stdout, clients
status: satisfied

- **The admin skill set behind `temper skill install --include-admin`**
  The admin surface gains installable skill packaging, gated: a default
  install ships no admin content at all — and stops carrying what it already
  did, because the generated `reference.md` previously included every admin
  command row (~40 of them) in a default install. The default skill's content
  therefore changes behind unchanged file shapes: same filenames, fewer rows.
  `--include-admin` restores the rows and ships `admin.md`; a later default
  install retracts installer-shipped admin files (byte-matched; user-edited
  files stay). Who observes: an operator running `temper skill generate` or
  `temper skill install` — the preview/install output loses admin rows unless
  they pass the flag, and the install report gains a retraction line when it
  removes one. MCP emit untouched (config-free by design; skills-drift green).
  Additive (new optional flag on install/generate) + behavioral (default
  render content); no HTTP/MCP/schema movement.
pr: 938
classes: additive, behavioral
surfaces: cli-stdout
status: signal-only

- **The Level-3 ordering pin — approved non-admin × nonexistent UUID (operator-directory follow-up, review finding F-B2)**
  Test-only. The directory's uniform-403 ordering pin existed only for the
  born-Denied class, whose request the router's `require_system_access`
  refuses before any handler code runs — a handler that 404s before its own
  gate still passed it. The new witness provisions an APPROVED non-admin (the
  only refused class that reaches the handler body) and pins the show door's
  `require_system_admin` gate-before-existence: a nonexistent profile UUID
  gets the same plain 403. Bite-probed per the consolidated pass's mutation.
  No production file, no wire shape, no behavior changed; no openapi.json
  movement. The row exists because the crosscheck matches the whole crate
  directory, tests included — the vocabulary's shape-unchanged residual is
  the closest class (the walk-link row's precedent).
pr: self
classes: behavioral
surfaces: internal
status: signal-only

- **The walk link — the resource rail offers walk-from-here as a deep link into the traversal door (UI-only)**
  The vault resource view's rail offered nothing that starts a question from
  the piece of work in front of the reader: reaching the traversal door meant
  opening the graph surface and re-describing the resource being looked at.
  The rail bar now carries "Walk out from here", a deep link to
  `/graph/@me?from=<this resource's uuid>` — the existing traversal door under
  its existing grammar, composed by the existing `graphHref` builder and
  addressed at the reader's own door. Depth is not emitted (grammar-only,
  ruled 2026-08-21 — its default lives in the read); no endpoint, no grammar
  change, every wire shape untouched, and the traversal read keeps exactly one
  consumer: the link carries the question, the graph screen answers it (task
  `01a0b626-1182-7011-89d9-67e3a87880dd`).
pr: 932
classes: behavioral
surfaces: internal
status: signal-only

- **The operator directory — the admin surface's read-only principal inventory (operator-directory PR-1)**
  Two new admin reads behind the sealed `&SystemAdmin` service gate:
  `GET /api/access/admin/profiles` (human principals, default filter `needs-access` =
  NOT-approved-including-absence, `email_contains` literal matching, team filter,
  50/200 limit and 0..10000 offset clamps, `total` in the envelope) and
  `GET /api/access/admin/profiles/{profile_id}` (the state card: admission,
  governance, auth links with the default-link fallback, memberships, pending
  invitations projected token-free from `vw_invitee_invitations`' attribution
  rule, open queue state, existing-command hints). `?email=` resolves an address
  EXACTLY (case-insensitive, verified) into the single matching card, refusing
  ambiguity with a 404 that names the collision. New wire types
  `AdminDirectoryEntry` / `AdminDirectoryListResponse` / `AdminProfileCard` and
  MCP inputs in temper-core; the generated `admin.ts` re-stales additively. No
  existing shape changes; both routes ride the existing plain-`.route()`
  operator-only posture and its allowlist.
pr: self
classes: additive
surfaces: http, clients
status: signal-only

- **The connections read states its bound — a bounded sibling of the /edges listing**
  `GET /api/resources/{id}/connections` answers
  `ResourceConnections { rows, total, limit, returned, truncated }` — the same
  visibility gate and the same incident-edge rows as the incumbent listing,
  under a server-side limit (default 50, clamped 1..=200), plus one gated
  COUNT over the same `edges_visible_to` predicate for the filtered total: an
  edge the caller cannot see is absent from `rows` and from `total` alike.
  `truncated` derives from the page in the envelope constructor, as the
  resource-list envelope derives it. The incumbent `/edges` array endpoint, its
  shape and every client of it are untouched; the vault rail's Connections
  region consumes the sibling and states the bound chrome in its heading.
  Additive only: one new response type, one new route, the generated SDK skins
  re-staled with the commit.
pr: 929
classes: additive
surfaces: http, clients
status: signal-only

- **The follow-affordance panel reads in the reader's terms, states its bound, and closes (UI-only)**
  The vault resource rail's connection rows read `label || edge_kind`, so an
  edge whose label was empty (the wire `COALESCE`s the column to `''`) rendered
  the system's structural name — 'near', 'contains' — as if the reader had
  written it, with asserted weight and polarity appended to every row. Rows
  now state the reader's own label verbatim or state nothing, and weight and
  polarity leave the row entirely. The event region's silent 50-row render
  slice is now stated chrome — "The most recent 50 of 60 events." — derived
  from the same `rows` the heading counts, so heading, statement and slice
  cannot disagree; the read stays whole. And the rail closes: the closed state
  names History and Connections as the regions it withholds rather than
  rendering as their absence, and the main column reflows not at all. Every
  wire shape is untouched — render-only behavior behind unchanged
  `GraphEdgeRow`/`EventTrail` shapes (task
  `01a0b56b-95fe-7af0-abf2-87fc1c23f249`).
pr: 927
classes: behavioral
surfaces: internal
status: signal-only

- **Palette search rows read the home the row actually carries — cogmap-homed rows stop rendering an empty home**
  The command palette's result sub-line read `context_name` alone, so a search
  hit homed in a cognitive map — which carries `cogmap_name` and no
  `context_name` key at all (`skip_serializing_if`) — rendered an empty home
  before the separator ("· concept"). The sub-line now reads
  `cogmap_name ?? context_name` and renders no home segment at all when a row
  carries neither. `ResourceView` and every wire shape are untouched: render-only
  behavior behind unchanged shapes, observed live from `POST /api/search`
  (task `01a0b4d5-9c40-7c03-add9-0a498e49b31b`).
pr: 925
classes: behavioral
surfaces: internal
status: signal-only

- **The palette asks the real door — one arm at a time on screen (UI-only)**
  The header-search palette's read moves from the title-`ILIKE` list door to
  `POST /api/search` through a new UI-server proxy (`/_internal/search`
  GET→POST), and the command palette renders one arm of the answer at a time —
  wide by default, an Exact switch selecting the other — with each arm's
  disposition rendered in place and the `/vault/search` hand-off dropped for a
  stated bound. Requests carry `{query, limit}` and nothing else: no `arms`
  param, no embedding, byte-identical to any current client's request — the
  arm selection never reaches the wire. The parked `arms` selector on
  `jct/search-arms-selector` is the wire half, aligned to the 0.6.0 chain.
pr: self
classes: additive
surfaces: internal
status: signal-only

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

- **Copy refresh — public-surface spans regenerated from their named authorities**
  Six public-surface files refreshed at span level, each derived from its
  authority: the /builders landing page's frontmatter demo now shows the
  canonical vault frontmatter (`temper-*` keys, `relates_to`) matching the
  decision schema; README's core-command descriptions match the CLI's
  `--help` text; temper-rb.md names the spec suite and its CI workflow as the
  pin behind its patterns; install-temper states the CLI's local-embedding
  default (`embed`+`extract` features; ingestion posts pre-embedded chunks);
  slack-mentions describes the linked-mention behavior as shipped (a model
  turn dispatched under the linked user's identity) and its Verify step names
  the observable reply classes; self-host-temper's `ENABLE_SWAGGER` row states
  the real Swagger UI path (`/api-docs/ui`) and that the flag carries no
  environment gate. Copy only — no contract shape moves, no runtime behavior
  change, no generated artifact re-stales.
pr: self
classes: additive
surfaces: internal
status: signal-only

- **Copy refresh — nine public-surface spans restated from their shipped authorities**
  Nine spans across the cognitive-maps and operating pages now state the
  shipped shape at each span: the materialize wake (a map re-materializes when
  formation events since its last materialization clear the default threshold
  of five, with the steward verbs to run it), the onboarding seed fixture
  (`onboarding-cogmap.yaml`) named as the worked scenario, the access
  scenarios named as YAML fixtures with S4 scoped to the charter-gating read,
  the emitter entity described as the seed declares it, and the admin ledger
  scoped to the grant family (team lifecycle stated as provisioning). Copy
  only — no contract shape moves, no runtime behavior change, no generated
  artifact re-stales.
pr: self
classes: additive
surfaces: internal
status: signal-only

- **Skill-copy refresh — the agent-skill surfaces' door names restated from their shipped authorities**
  The shared skill templates behind the `agent-skills/temper-knowledge-base`
  projection state each surface's doors as shipped: the data-artifacts surface
  section's heading is surface-parameterized (the MCP render shows the MCP
  tools, the CLI render the CLI door); working-a-goal's traversal paragraph
  names the MCP `run_query` composition door — the `follow-from` act with its
  `bound`/`seed` relations — beside the CLI's `temper graph traverse`, and its
  advance-phase workflow pointer is surface-parameterized (the MCP packaging
  ships no per-cell workflow files, as its SKILL.md states). The hand-written
  `knowledge-base.md` names `get_profile` as the REST route `GET /api/profile`
  and scopes `reassign` to the resource and team reassign doors. Copy only —
  no contract shape moves, no runtime behavior change; the only generated
  re-stale is the skill emit itself.
pr: self
classes: additive
surfaces: internal
status: signal-only

- **The MCP system-access denial renders the full remediation payload (operator-directory PR-3)**
  A denied MCP caller now receives the same remediation-bearing denial a
  browser caller does: the error's structured `data` carries the full
  `SystemAccessDetails` — email, display_name, typed refusal, request_url,
  cli_command — built by one shared constructor (`SystemAccessDetails::for_profile`
  in temper-core) that temper-api's 403 middleware also renders, so the two
  surfaces cannot disagree on the remediation. The message names the denied
  identity, so an agent operating under a credential it does not read can tell
  the human WHICH account needs approving. The previous `data` carried only
  `{ refusal }` and dropped the identity; the `refusal` key and its serialized
  shape are unchanged, so existing readers keep working. No tool inventory
  change; the request URL is hoisted beside `REQUEST_ACCESS_COMMAND` as the
  shared `REQUEST_ACCESS_URL` constant.
pr: self
classes: additive, behavioral
surfaces: mcp
status: signal-only

## Shipped in v0.5.2

- **This release — the 0.5.2 fleet alignment: VERSION 0.5.1 → 0.5.2 across crates, packages, and clients**
  The release train's own wire delta is none: version fields and the generated
  cores re-stale with the bump (the D-S3 baseline — no shape movement); the
  P floor rides the additive rows already in this window.
pr: self
classes: additive
surfaces: http, clients
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

## Shipped in v0.5.1

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

## Shipped in v0.5.0

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
surfaces: http, clients
status: signal-only

- **The operator directory reaches the operator — `admin profiles {list,show}` (operator-directory PR-2)**
  The CLI group over PR-1's reads: `admin profiles list` (filters pass
  through verbatim: `--standing`, literal `--email-contains`, `--team`,
  clamped `--limit/--offset`) and `admin profiles show` by UUID or `--email`
  (exactly one; ambiguity is refused server-side — the CLI never narrows
  locally). Three typed AdminClient methods (`list_profiles`,
  `profile_card_by_email`, `show_profile`) over the unchanged routes — the
  card arm is its own method because its response SHAPE differs from the
  page's. No existing shape changes; card hints render verbatim and name only
  existing commands AND are machine-legal for the standing class they target —
  the approve hint is gated to denied/requested/revoked (it refused from
  standing-row absence and deactivated), deactivated cards advertise
  `reactivate` instead, and absence advertises no access act; token-absence is
  asserted by e2e against the real binary's rendered stdout in both json and
  toon.
pr: self
classes: additive, behavioral
surfaces: cli-stdout, clients
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
