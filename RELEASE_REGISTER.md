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

## Since v0.4.0 — unreleased

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
surfaces: mcp
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
status: open

- **The defined dangling state — citation-audit gate: folded ≡ live → defined refusal**
  Auditing a citation whose block is folded is now refused (the gate's standing zero-rows→404
  dialect) where it silently succeeded on a gone citation before. Existing request class,
  success→refusal; the refusal face is unchanged, the resolved-to state moved.
pr: self
classes: behavioral
surfaces: http, mcp, cli-stdout
status: open

- **Citation 1 — PR #867 briefing: identical body + sources preserves block identity**
  A whole-body update whose sections and sources are byte-identical now keeps their block ids,
  revision history, and provenance; before, every section re-minted fresh ids. Block-aware
  callers holding ids observe it, on every write surface. User-visible defect fix — the case
  that motivated the block-grain sequencing rule; briefed at the #867 merge. Gates the next
  release of these surfaces until it is carried and marked satisfied.
pr: 867
classes: behavioral
surfaces: http, mcp, cli-stdout
status: open

- **Citation 2 — PR #867 briefing: content-gone citations stop reinforcing standing**
  A rewriting revise moves prior sources to history on the folded block; live citation
  magnitude and reinforcement reflect only what survives. Standing consumers observe it — the
  most user-visible change in the set: a rewritten finding's citation count drops to what still
  exists. Owes a release-notes signal; gates the next release of these surfaces until carried.
pr: 867
classes: behavioral
surfaces: http, mcp, cli-stdout
status: open

- **Citation 3 — PR #867 briefing: redistributed carried rows become live citations**
  Absorbed and carried copies now count as uncorrected rows on live blocks; before this change
  they were invisible. Standing consumers observe it: counts on rewritten findings reflect
  redistributed copies. Owes a release-notes signal; gates the next release until carried.
pr: 867
classes: behavioral
surfaces: http, mcp, cli-stdout
status: open

- **Citation 4 — PR #867 briefing: one `resource_reblocked` per whole-body update**
  The ledger grain changes: one `resource_reblocked` computed against pre-update incumbents,
  where `block_mutated` + `resource_reblocked` fired before. Ledger consumers observe it; the
  fan-out correlation doc names `block_mutated` as an update sub-event and owes its amendment.
  Gates the next release until carried.
pr: 867
classes: behavioral
surfaces: internal
status: open

- **Citation 5 — PR #867 briefing: chunker-skew 500 retires to async backfill**
  A CLI↔server chunker skew no longer fails the update with a 500 — unmatched caller chunks
  fall to async embed backfill, so a failure face becomes success-with-backfill. Chunk-packing
  callers observe it. Gates the next release until carried.
pr: 867
classes: behavioral
surfaces: http, mcp, cli-stdout
status: open

- **Citation 6 — PR #867 briefing: single-section identical rewrite is silent**
  A single-section identical rewrite with no sources emits no revision event, where it minted a
  fresh one before. Ledger consumers observe it. Gates the next release until carried.
pr: 867
classes: behavioral
surfaces: internal
status: open

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
status: open

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
