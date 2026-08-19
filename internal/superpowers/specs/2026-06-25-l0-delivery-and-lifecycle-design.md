# L0 Delivery & Lifecycle — Design (notes for a future branch)

**Date:** 2026-06-25
**Status:** Design / notes — **not scheduled for implementation in the current branch.** Captured while the
L0 content design is fresh so the future delivery branch starts from a settled model.
**Context:** Workstream 7 (Agent surface) under goal `substrate-kernel-to-cognitive-map`.
**Companions:**
- Architecture: [2026-06-25-cognitive-map-agent-invocation-architecture-design.md](2026-06-25-cognitive-map-agent-invocation-architecture-design.md) (L0 = deterministic kernel tier; "L0 evolves via additive shipped structures, release/operator-governed; ambient wake = never")
- Content: [2026-06-25-l0-orientation-kernel-charter-design.md](2026-06-25-l0-orientation-kernel-charter-design.md) (the orientation-kernel charter + seeded landmarks; **§6 deferred this delivery + lifecycle thread**)
- Built: L0 birth migration (empty `system-default` cogmap) + the workbench seed/scenario proving the content materializes.
- Reuses: the seed/scenario DSL; `content::prepare_blocks` (bge-768 embed); the mutation SQL functions; the `kb_invocations` envelope (#156).

---

## Why this design

The content design (companion §6) deferred two threads with one honest gap: **how authored L0 content
gets onto the live `system-default` map, and how it keeps getting updated.** The workbench let us ignore
this because the scenario runner embeds + loads in the `temper_next` test namespace. Production L0 is born
empty (SQL-native), and a `sqlx::migrate!` SQL file **cannot run ONNX** — so authored content (whose chunks
carry bge-768 embeddings) cannot be a plain data migration.

**The unifying observation:** delivery and lifecycle are the *same machine*. Delivery is the *first* update
(empty → populated); lifecycle is *every subsequent* update. So there is one question — *how does L0's
content get from a committed, authored artifact onto live L0, idempotently, over and over* — and one answer:
**desired-state reconciliation**.

---

## Decisions (each settled in design dialogue)

### 1. Update model: M1 — desired-state reconciliation

A committed **manifest** is the desired state. A **server-side reconciler** diffs manifest-vs-live and
applies *additive* events. Same machine whether run once (delivery) or every release (lifecycle).

Rejected: **M2 imperative shipped-scenario deltas** (no convergence guarantee; each delta must be authored
correctly; re-apply isn't idempotent without bookkeeping). **M3 operator-directed steward only**
(non-deterministic, not reproducible) — kept *only* as the escape hatch for large judgment-driven
restructures, never the default.

### 2. Embed-capable applier, Rust-side (no migration ONNX, no inlined vectors)

The reconciler reuses the affordance that already works in `temper-next`'s loader:
`content::prepare_blocks` (chunks + embeds bge-768 via temper-ingest) + the mutation SQL functions
(`resource_create` / `block_mutated` / `facet_set` / `relationship_assert` / `relationship_fold`). That
embed+mutate logic is **lifted into a service `temper-api` can call** — *not* a parallel DB-writer (the
architecture forbids a second write path).

### 3. Reconcile semantics: O3 (additive/update-only) + O1 (provenance-tagged)

- **Identity key = `origin_uri`** (stable natural key — L0 landmarks are `temper://kernel/...`).
- **Create** — manifest entry absent in live → `resource_create` (+ facets, + edges), embedded.
- **Update** — body content-hash changed → `block_mutated` (re-embed); facet/edge changes → assert/fold the delta.
- **Removal — never fold on mere absence.** Removing kernel content requires an explicit `fold:` tombstone
  in the manifest. (This is what makes reconcile *safe by construction* — it physically cannot delete
  content it doesn't see.)
- **Provenance (O1)** — kernel content carries a `provenance: kernel` facet; reconcile manages **only the
  kernel slice**, leaving `promoted` / `operator` content untouched.
- **Idempotent** — same manifest + same live state → **zero events** (content hashes match).

This resolves the classic desired-state drift pitfall: promotion-from-maps and operator edits write
content reconcile doesn't own (different provenance) and can't remove (no fold-on-absence), so a reconcile
can never clobber them.

### 4. Surface & authz: `PUT /api/cognitive-maps/{kernel-uuid}`

- **PUT = desired-state, idempotent** — the body *is* the desired manifest; the server diffs, embeds, and
  fires the additive events. Diff + embed + event-emission all stay server-side.
- **Manifest is carried in the PUT body** (operator/CI sends it). The API stays **stateless** about repo
  layout; the manifest lives in the repo, CI just sends it.
- **Authz — structural rule:** *writes to a root-team-only cogmap require `is_system_admin`* (whitelist by
  the kernel UUID `00000000-0000-0000-0005-000000000001`). Not an ad-hoc allowlist — a property of the
  system-default cogmap.
- Dispatches a `reconcile_cognitive_map` operations command **through the backend trait** (writes route
  through the trait, per the architecture).
- **Verb caveat (documented):** strict PUT means "replace, delete-on-absence." This is a
  **reconcile-PUT-with-tombstones** (absence ≠ deletion; removal is explicit). Defensible because the body
  is the full desired manifest and the op is idempotent; PATCH is the pedantic alternative, but PUT carries
  the desired-state framing we want.
- **It's still events.** Every mutation the handler applies is a `kb_events` row (event-as-primary holds).
  The desired-state PUT is the *contract*; events are its *consequence*.

### 5. Audited via our own invocation envelope

Each reconcile run opens a **`kb_invocations`** row, `trigger_kind: admin_reconcile`,
`originating_cogmap_id` = the kernel cogmap, scoped to the system actor. On close, the **outcome** records
`{created: N, updated: M, folded: K}`. This makes a reconcile an accountable, replayable run — *using the
machinery we already shipped* — and the open invocation doubles as the **serialization mutex** (see §7).

### 6. Triggers ("when does it update")

- A **release-pipeline step** (a deploy ships a new manifest version → CI or an operator fires the PUT)
  and/or a **manual operator command** (a thin `temper` admin CLI → the PUT).
- **Not** on serverless cold-start (temperkb.io is Vercel serverless; there is no meaningful boot, and
  cold-start reconcile would be wasteful + racy).
- L0's `ambient steward wake = never` holds: this path is admin/release-governed, **firewalled from
  operational cognition** (admin-events principle).

### 7. Concurrency

- **Reconcile vs promotion** — safe by construction (O3 additive-only + O1 provenance; no clobber).
- **Reconcile vs reconcile** — serialized. The open `admin_reconcile` invocation (§5) is the natural mutex
  (or a Postgres advisory lock); a second reconcile waits or no-ops.

---

## The two update vectors (kept distinct — do not conflate)

1. **Release-shipped manifest reconcile** (this design) — temper-the-software's self-description tracking
   the software. The PUT + manifest.
2. **Promotion-from-maps** — a concept that recurs across L1/L2 maps earns its way *up* into L0's
   non-kernel slice; a single **human-gated live act** (a future `POST /api/events` admin door),
   `provenance: promoted`, **not** managed by reconcile. This is its own design (it overlaps the
   cross-map promotion-translation HITL gate already deferred in the architecture spec).

---

## Components (for the future plan)

- **L0 manifest** — committed artifact in the seed/scenario DSL shape (or a thin dedicated wrapper),
  `provenance: kernel` on every entry, `origin_uri` as identity, optional `fold:` tombstones.
- **Reconciler service** (`temper-api`) — diff(manifest, live-kernel-slice) → plan(create/update/fold) →
  apply via the embed-capable mutation path, inside one `admin_reconcile` invocation. Idempotent.
- **`PUT /api/cognitive-maps/{id}` handler** — admin-gated (root-team-cogmap rule), dispatches
  `reconcile_cognitive_map` through the backend; body = manifest.
- **Operator surface** — a thin `temper` admin command (and/or a release-pipeline step) that POSTs… i.e.
  PUTs… the committed manifest.
- **The embed+mutate lift** — extract the reusable piece of `temper-next`'s loader (prepare_blocks + fire
  mutations) into a place the API service can call, without making `temper-next`'s *test* loader a runtime
  dependency.

---

## Deferred / out of scope (named, not dropped)

- **Implementation** — this is design-notes; a future branch writes the plan.
- **Promotion-from-maps mechanism** (vector 2) — its own design; overlaps the cross-map
  promotion-translation HITL gate.
- **The `POST /api/events` admin door** — the general single-admin-event surface (one-off admin acts,
  promotion); distinct grain from the bulk reconcile PUT.
- **Embedding-model migration** — if the bge model version changes, a full re-embed is a separate
  operational concern; note that the update trigger keys on *source-text* content-hash, so embeddings
  follow text changes naturally, but a model-version bump is a global re-embed event, not a reconcile.

---

## Open risks

- **Verb semantics** — reconcile-PUT is a mild stretch of PUT (documented above). Acceptable.
- **Manifest schema drift** — the manifest reuses the seed/scenario DSL; if that DSL evolves, the L0
  manifest must stay loadable. The snapshot-tested JSON Schema for seeds/scenarios is the guard.
- **First-delivery size** — the initial reconcile embeds the whole kernel (tens of chunks); fine for an
  admin/deploy op, but the handler should stream/transact sensibly and the invocation outcome should report
  progress for a large first run.
- **`temper-next` loader lift** — the reusable embed+mutate code currently lives in the test-oriented
  `temper-next` crate; lifting it cleanly (without dragging test-only deps into the API runtime) is the main
  structural work the future plan must scope.
