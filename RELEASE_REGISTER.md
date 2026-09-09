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
pr: goal
classes: behavioral
surfaces: clients, http, mcp, cli-stdout
status: blocked:first block-aware client release at scale

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
