# Trunk-Change Awareness — a SCIP Code Graph Teams and Stewards Can Watch

**Date:** 2026-07-12 (reframed same-day — §0; revised after spikes S1–S3 — §0.5)
**Status:** Design proposed. **Spikes S1, S2, S3 complete — measured, and three load-bearing claims
were DISPROVED.** S4/S5/S6 pending. Not yet approved for implementation.
**Scope:** A native, event-sourced code-intelligence graph in Temper, sourced from SCIP, built as a
**sibling projection family** on the substrate kernel — kept architecturally distinct from the curated
resource/edge/cogmap graph and joined to it **only by symbol-string citation**.

> **Companion research:** [`docs/research/2026-07-12-scip-code-graph-integration-research.md`](../../research/2026-07-12-scip-code-graph-integration-research.md).
> Read it for the **SCIP substance** — the wire format, the symbol grammar. Its *framing* is superseded
> by this document, and **its `kb_code_symbols` DDL block contains an outright bug** (§0.5, T3).
>
> **Goal** (Temper): *Trunk-change awareness: a SCIP code graph teams and stewards can watch*
> (`@me/temper`, ref `…-019f56e1-49f0-78e0-a843-75a96274ce1e`). Six spikes `advance` it.

---

## 0. What changed, and why — read this first

The first cut of this spec answered *"how do we build a SCIP projection that doesn't corrupt the
curated graph?"* It answered that well. But it never asked **who is asking, and what for** — so every
spike's acceptance criterion validated the *construction* and none validated an outward claim about
what an agent could then **do**.

The operational intention, now stated:

- **Temper does not compete with local tooling.** rust-analyzer, LSP, and ripgrep own the working
  tree, the IDE, and the in-flight agent session. **Interactive navigation of a dirty tree is out of
  scope by fiat** — which removes the single hardest problem in the space.
- **The consumer is a steward agent watching trunk.** A team's context or cogmap declares a code
  region it owns; when `main` moves, an agent judges whether what shipped **materially changes the
  plans in flight**.
- **The primary read is therefore not `code_definition`.** It is:

  > *"What landed on main, in the part of the code I care about, since I last looked — and does it
  > touch anything my plans depend on?"*

---

## 0.5 What the spikes MEASURED — including where this spec was wrong

Everything below comes from real indexes of this repo (`rust-analyzer scip`, `scip` CLI v0.9.0),
real commits, and surgical probes. **Where a claim in the previous revision failed measurement, it is
marked ❌ and corrected in place.** The failures are the most valuable output the spikes produced.

### The thesis: CONFIRMED, from two independent directions

**On a real commit** (`a80131e6 refactor(config)`, 7 files / 78 insertions / 60 deletions), S1 resolved
the `surfaces` region and diffed the symbol sets:

> **4 files changed, 24 lines changed — symbol diff `+0 / −0`. The pure-SCIP feed reported SILENCE.**

S2 confirmed it surgically: changing `<=` → `<` (this spec's own example) and a `0` → `1` each
classified as **`body-changed` and nothing else** — same signature, same kind, same line, and
**decisively no change in the occurrence set** (rust-analyzer emits no occurrence for comparison
operators or integer literals). **The content hash was the only discriminator.**

> **The per-definition content hash is NOT "the one departure from pure SCIP." It is the mechanism.**
> Without it a subscription is a feed that reports *"nothing happened"* while the code it watches is
> rewritten — the failure §8 rates as *worse than reading nothing at all*. `body-changed` blindness is
> **not an edge case: it is the modal case for a refactor**, which is the modal commit on a mature trunk.

### ❌ D1 — "A file move is provably not a change to the region." FALSE AS WRITTEN.

Both halves measured, honestly:

| Experiment | git | symbol diff |
|---|---|---|
| `services/mod.rs` → `services.rs` (**module path preserved**) | `R100` | **`+0 / −0`** ✅ the payoff is real |
| +40 blank lines injected (pure line-shift) | `+40` | **`+0 / −0`** ✅ `moved-only` confirmed |
| `services/lineage_service.rs` → `src/lineage_service.rs` (**hoisted one level, ZERO logic changed**) | `R100`, **0 lines** | **`+11 / −11`** ❌ |

**In Rust the descriptor IS the module path, and the file path determines the module path.** So git —
which has rename detection — reports the hoist as *less* of a change than SCIP does. **Most real file
moves DO churn symbols.**

> **The invariant that actually holds, and the only one we may claim:**
> **A symbol's identity is independent of its position within a file, and of a filename that is not
> part of its module path.** Strictly weaker than "file moves are free." Still worth having — it is a
> **25× noise reduction on real data** (§0.5/M2) — but the strong form will not survive review.

**Unmeasured and load-bearing:** if `scip-typescript`'s descriptors embed the **file path**, then *no*
TS file move is symbol-neutral and this payoff does not exist on half this repo. **S3 did not measure
it. Nobody has.** → §7, S6.

### ❌ D2 — "SCIP carries the four `Relationship` booleans." TRUE OF THE FORMAT, FALSE OF THE INDEXER.

**rust-analyzer populates ZERO of 21,095 symbols.** `is_implementation` / `is_reference` /
`is_type_definition` never appear.

- **`code_implementations` — one of the four reads in §2, and S6's headline — has no data behind it on
  Rust.** It is not a schema question; there is nothing to read.
- The ***implements* half** of `inbound-edge-gained` does not exist. *"Someone new implements a trait in
  my region"* is **invisible**.
- The ***calls* half survives** — recoverable by reversing the occurrence-containment map (205 rows in a
  real trunk step).

Whether `scip-typescript` populates relationships is **unmeasured**. Relationship support is a
**per-indexer capability**, not a SCIP guarantee, and the schema must treat it as such.

### ❌ D3 — "Reuse the CAS blob store wholesale." THERE IS NO CAS. This is net-new work.

- `kb_chunk_content.content` is **`TEXT`**. Postgres `TEXT` **cannot store a NUL byte**; `index.scip`
  contains **11,711** and is not valid UTF-8. That path is a *markdown* pipeline (chunk → bge-768 →
  hnsw). **A `.scip` cannot ride it.**
- `kb_blob_files` is a Vercel Blob **URL** + status enum — no hash, no bytes, no idempotency — and it is
  **dead** (one reference repo-wide; `@vercel/blob` has zero call sites).

**A binary content-addressed blob store is net-new and was in nobody's estimate.** What *is* reusable
is `block_append`'s **idempotency contract** (identical re-append = silent no-op; same-seq-different-
content = `RAISE`). S3 built and proved a segmented CAS against real Postgres: 5×3 MB segments in
292 ms, idempotent re-upload, resume, **corrupted and conflicting segments rejected**, readback
byte-identical. Segmentation is forced by **Vercel's 4.5 MB request-body cap**, not by the database.

### ❌ D4 — `enclosing_range` belongs to `Occurrence`, not `SymbolInformation`.

Found independently by S2 and S3. `SymbolInformation` declares only
`{symbol, documentation, relationships, kind, display_name, signature_documentation, enclosing_symbol}`.
The content hash keys off the **definition occurrence**. It works — **100% populated (20,915/20,915)**
for rust-analyzer — but **whether an indexer populates it at all is a per-indexer property** that
nobody had checked.

### Traps neither the spec nor the research doc saw

**T1 — An index is a view from a commit AND A BUILD CONFIGURATION.** The 69 `tests/e2e/` documents carry
`#![cfg(feature = "test-db")]` and collapse to **ONE symbol** under default features. `UNIQUE(repository_id,
commit_sha, tool_name)` **cannot distinguish `rust-analyzer scip .` from `… --all-features`** — same
commit, same tool, thousands of symbols apart. Under that key they **collide and silently overwrite**,
and every subscriber sees a phantom mass add/remove. **The key must carry the indexer invocation.**

**T2 — `symbol → content_hash` is NOT a function.** `temper-e2e 0.0.0 crate/` is defined **69 times**,
once per integration-test file (17 crate-root symbols have multiple definitions). A hash table keyed
`UNIQUE(index_id, symbol_id)` **fails on a constraint violation the first time it ingests this repo's
own index.** The key must include `document_id`.

**T3 — The research doc's `kb_code_symbols` DDL is the local-symbol bug, verbatim.** Its prose says
locals are per-document (correct); the DDL block above it says `symbol_string TEXT NOT NULL UNIQUE` +
`is_local BOOLEAN`. Measured: **508** distinct local strings but **12,938** true `(document, local)`
pairs; **89% collide**; `local 0` appears in **274 documents**; 23.5% of all occurrences reference a
local. A global unique dictionary **silently collapses 12,938 symbols into 508 rows.**

**T4 — 63% of SCIP "definitions" are `local N` bindings that RENUMBER on any edit to the file.** Include
them in a change-set and 1,036 rows become 2,475 that are **51% noise** — including **777 phantom
`signature-changed`**. Computing change-sets over global symbols only is a **correctness precondition**,
not a risk to manage.

**T5 — NEVER re-encode a `.scip`.** prost **silently drops unknown fields**: rust-analyzer emits a
`Signature` field that `scip.proto` v0.9.0 **`reserved`s**, so decode→encode loses 42,330 bytes without
a word. Also, rust-analyzer uses the **deprecated packed range form exclusively** (0 of 139,280 use
`typed_range` — a typed-only decoder reads *nothing*), in **both 3- and 4-element** flavours.
**The CAS stores and hashes bytes exactly as received.**

**T6 — `scip lint` is not a pass/fail oracle.** It emits **56,181 diagnostics on the untouched index**
(rust-analyzer emits empty `external_symbols`). A golden test asserting `lint` succeeds fails on day one.

### Measurements that held (M)

**M1 — Retention: VALIDATED.** At this repo's real trunk rate (2.69 Rust-touching merges/day):

| strategy | occurrence rows | storage/yr |
|---|---|---|
| keep every full index | 136.7M/yr, growing | **11.2 GB/yr** |
| tip-full + change-set chain (interned) | **139,297 — CONSTANT** | **86 MB/yr — 130×** |
| …material-only | **139,297 — CONSTANT** | **29 MB/yr — 386×** |

**Correction to the previous wording:** *"the big table is bounded by ONE index, not by history"* is
true **only of the occurrence table**. The change-set chain is a **second table that grows linearly**
(~1M rows/yr). The decision stands overwhelmingly; the sentence overclaimed.

**M2 — `moved-only` is 76% of every change-set.** On a real step, a `temper-cli` watcher sees **487
rows, of which 468 (96%) are pure line-shift** — a **25× noise reduction**. This is the SCIP payoff,
and it is larger than claimed.

**M3 — `signature_documentation` is AST-rendered, hence FORMAT-IMMUNE.** A rustfmt reflow that broke 8
signatures across lines produced **zero** `signature-changed` rows. It is the most trustworthy signal in
the system and belongs in the change-set row as its highest-value payload.

**M4 — Authz falls out for free, including the subtleties.** Proven against the live shipped predicates:
a `watcher` reads the repo but **cannot** subscribe; the repo's owning team **cannot** subscribe on
another team's behalf; an outsider gets **zero rows, not a 403**; `temper context share` (already
shipped) is the entire cross-team mechanism. **One** new function is needed:
`anchor_authorable_by_profile` — a five-line `CASE`, the write-side twin of a read-side function that
already ships. **New authz semantics: zero.**

**M5 — Decode is cheap and correct.** 13.6 MB → structured data in **18 ms**, zero range-validation
errors, counts matching `scip stats` **exactly** (493 / 20,915 / 139,280). Symbol parser: 10,113 distinct
real symbols, **0 parse failures, 0 round-trip mismatches**. Content hash over all 20,915 definitions:
**74 ms** (noise against a 34 s index run).

**M6 — Frontier depth = 1 hop.** Depth 0→1 **more than doubles** material signal (23→55 rows) for a 3×
set expansion, and is exactly the *"the migration function I depend on changed its signature"* wake.
Depth 2 costs **1,687 more watched symbols to surface 14 more rows** and drags the watched set to
**44.5% of the codebase** — a watcher whose region is half the repo is watching nothing.

### Questions the spikes CLOSED

| Question | Answer |
|---|---|
| Rustfmt phantom `body-changed` — normalize? | **NO. Hash raw bytes.** No whitespace normalizer is complete (rustfmt inserts trailing commas *and* closure braces — both token changes), and worse, **normalizers produce FALSE NEGATIVES**: they cannot see string-literal boundaries, so they silently absorb a changed log message, a changed user-facing string, **a reflowed SQL query**. §1.3 puts a judging agent between signal and attention — which makes a false positive **cheap** and a false negative **catastrophic**. Carry `formatting_suspect` as an **advisory** flag. (And the trigger is rare: `cargo fmt` is pre-commit-enforced, so trunk is always fmt-clean.) |
| Frontier depth | **1 hop** (M6). |
| Partitioning | **Premature.** Threshold: partition when a single index exceeds **~1M occurrences AND** daily prune volume exceeds **~10M rows** — ~30× this repo's churn. **Trap:** partitioning *by* `index_id` means **DDL on every merge** (`ACCESS EXCLUSIVE` locks, violates "sqlx owns migrations", bad citizen on serverless Neon). The natural partition key is the one you cannot cheaply partition on. |
| prost vs alternatives | **prost 0.14, codegen via `protox`** (pure-Rust, verified byte-identical to `protoc`; removes a system toolchain dep from CI). No protobuf crate exists in the workspace today. |
| New crate or a module in `temper-ingest`? | **New crate, `temper-scip`. Not close.** Feature flags **do not protect you** under workspace unification — this repo has been burned twice (substrate → ort everywhere; temper-cloud → `ingest-pipeline` on temper-api). A `scip` feature would unify **on** and drag prost + a `build.rs` into both Vercel bundles. **The crate boundary is load-bearing; the feature gate is not.** |
| Where is the content hash computed? | **The CLI, at upload time.** Computing it **requires decoding SCIP** (packed-vs-typed ranges, 3-vs-4 element, position encoding) — a shell wrapper would have to reimplement the decoder. One CLI, N indexers; the indexer invocation stays stock. Accept plainly: the hash is **producer-asserted** (the server has no source) — which is exactly what machine-principal attribution is for. **Hard constraint: ingest can only run where the source is.** |
| Ownership drift | **Its own change class, never suppressed, never churn.** Measured: moving **one line** of CODEOWNERS with **zero bytes of code changed** produces **1,902 phantom symbols** — against a real commit's 5-symbol delta. A **~380:1** noise ratio. It emits **counts, not per-symbol rows**: a symbol that entered a region because the selector moved **was not added to the codebase**, and calling it `added` is a lie the schema should not be able to tell. |
| Never-woken watermark | **NULL ⇒ `baselined`, not "everything is new."** The first tick resolves and persists the region, sets the watermark, and emits a `baselined` change-set carrying **counts and the unresolved-path residual, and no per-symbol rows.** A new subscriber has not had 598 symbols *added* to it — it has merely **arrived**. This also gives S4's chain-gap problem its vocabulary: a watermark pointing at a pruned index degrades to *"re-baseline, and say so"* rather than *"nothing changed."* |

---

## 1. Load-bearing premises

### 1.1 The two graphs are different in kind

A SCIP index is a **mechanically-generated, commit-pinned, closed-ontology projection of source at one
commit**. Temper's resource graph is **curated, assigned-identity, open-vocabulary, ledger-as-truth**.

- **Do NOT** put code symbols in `kb_resources`, code edges in `kb_edges` (never widen `edge_kind` with
  `calls`/`imports`), or code structure through the region/lens/salience producer.
- **DO** reuse the substrate *kernel* — `kb_events` ledger, append-then-project, contexts-as-home +
  team-DAG authz, machine principals, replay/drop-rebuild, span-locator provenance, and the **agent
  invocation envelope**. (**Not** the CAS — see §0.5/D3. It does not exist.)

### 1.2 The membrane — stated properly

> **The code graph is the unjudged record. All judgment lives in the curated graph and cites into the
> code graph.**

- **Code graph → steward: mechanical facts only.** Structure, signatures, content hashes,
  commit-pinned, attributed to the CI indexer's machine principal. No opinion, ever.
- **Steward → curated graph: judgment.** An authored event under an invocation envelope carrying
  `reasoning` + `confidence` + `rationale`, **citing the symbol string**.
- **Citation flows one way** (curated → code, by string). **Judgment never flows back.**

*"Code edges never enter `kb_edges`"* is a **consequence**, not the principle. The deepest reason the
graphs must stay separate is that **the code graph is regenerated wholesale every commit** — anything
written onto it would be destroyed.

This half of the system **already exists**: `crates/temper-agents/src/envelope.rs` carries
`AgentAuthorship`, `ConfidenceBand` (`Tentative`/`Probable`/`Confident`), `Disposition`,
`InvocationClosed`. We are not designing agent judgment — we are **pointing existing agent judgment at
a new class of fact.**

### 1.3 Materiality is reported, not decided, by the schema

The feed emits **classified change facts**. The **agent judges**, because *"does this change my upcoming
plans"* is a judgment only something holding the cogmap can make. Encoding materiality as a SQL
predicate is how this becomes a notification firehose nobody reads.

**This is load-bearing, not a preference.** It is what makes a noisy signal affordable — and it is the
entire argument that settles the rustfmt-normalization question (§0.5): with a judging agent in the
loop, a false positive is cheap and a **false negative is catastrophic**.

---

## 2. Architecture at a glance

```
CI (has source — HARD CONSTRAINT)   substrate kernel              code-graph projection family
─────────────────────────────────   ────────────────              ───────────────────────────
rust-analyzer scip  ─┐              kb_events (truth)             kb_code_repositories   (registration)
scip-typescript      ├─ .scip ─┐    code_index_ingested           kb_code_subscriptions  (registration)
                     ┘         │      (payload = metadata          kb_code_subscription_regions
temper CLI decodes + hashes ───┤       + 2 blob hashes)            kb_code_indexes        (the vantage)
  (hash is producer-asserted)  │            │                      kb_code_symbols   (NEVER pruned)
                               └──► CAS ◄───┘                      kb_code_documents
                                  ** NET-NEW ** (§0.5/D3)          kb_code_occurrences    (tip only)
                                  bytes stored AS RECEIVED         kb_code_change_sets    (the chain)
                                  (never re-encode — T5)

primary read:  code_changes_since(subscription) → region-filtered, classified, since-watermark
support reads: code_definition / code_references / code_hover @ trunk tip
               code_implementations — ⚠️ NO DATA on Rust (§0.5/D2)

membrane: steward reads facts ──judges──► authored event (reasoning/confidence) ──cites symbol──►
```

**Authz seam:** `kb_code_repositories.context_id → kb_contexts` ⇒ code facts inherit
`contexts_readable_by` / `context_authorable_by_profile`. **Deny = zero rows, never a 403.** Proven
(§0.5/M4). Cross-team watching is `context share`, already shipped. **No new authz surface.**

---

## 3. The watching model (S1 — measured)

| Table | Kind | Notes |
|---|---|---|
| `kb_code_repositories` | **registration** | homed in a context. One context may home several repos. |
| `kb_code_subscriptions` | **registration** | `(context \| cogmap) → (repository, selector, watermark)` |
| `kb_code_subscription_regions` | **projection** | the **resolved symbol set** per `(subscription, index)` |

> **A grounded distinction the spec previously missed.** `kb_contexts`, `kb_teams`, `kb_machine_clients`
> carry **no event lineage** — they are *registrations*. `kb_edges` carries `asserted_by_event_id` — it
> is a *projection*. **Repositories and subscriptions are registrations** (declared by a human;
> destroying them on replay would be wrong). **Indexes and everything downstream are projections.**
> Getting this backwards puts a team's watermark at the mercy of a projection rebuild.

**`kb_code_indexes` must be keyed `UNIQUE(repository_id, commit_sha, tool_name, tool_config)`** — the
3-column key of the previous revision is **unsound** (§0.5/T1).

### The region selector — paths in, symbols out, **with an honest residual**

Declared as CODEOWNERS-shaped globs, **resolved per index into a SCIP symbol set**, and the symbol set is
what we persist and diff. But path globs alone are **insufficient — refuted three ways, all measured**:

1. **`/migrations/` resolves to ZERO symbols.** CODEOWNERS itself calls it *"the kernel everything else
   is built on… the widest blast radius in the repo."* **SQL is in no SCIP index we will ever produce.**
   Same for `/openapi.json`, `/.github/`, `/.sqlx/`, `/docs/`.
2. **`/tests/e2e/` → 69 documents, ONE symbol** (build config, §0.5/T1). A steward subscribed to it would
   see nothing, forever, **and never know why.**
3. **Unit tests live INSIDE production files** in Rust (`#[cfg(test)] mod tests`). **No path glob can
   exclude them** — and on a real commit, **4 of the 5 signal symbols were renamed test functions.**

**Therefore the selector is `path:` rules AND `symbol:` descriptor-prefix rules**, ordered, with
negation (union-with-negation — *not* CODEOWNERS last-match-wins; a subscription is one watcher's
selection, not a partition of the repo). Surprising and measured: **descriptor selectors are MORE
stable than path selectors** — a `symbol:` selector on the package would have survived the module hoist
(§0.5/D1) that destroyed the path-based region.

**The unresolved-path residual is a FIRST-CLASS OUTPUT, not an error.** Resolution yields
`(symbol_set, unresolved_paths)`. **Silently resolving `/migrations/` to the empty set is the "reports
silence while the code changes" failure, aimed at the highest-blast-radius region we have.** Do not
fabricate symbols for SQL. **Report the blind spot.**

---

## 4. Change-sets & materiality classes (S2 — measured)

Computed over **global symbols only** (§0.5/T4 — a correctness precondition), between consecutive trunk
indexes, restricted to a watched symbol set **S plus its 1-hop dependency frontier** (§0.5/M6).

| Class | Status |
|---|---|
| `added` / `removed` | ✅ Confirmed |
| `signature-changed` | ✅ **Format-immune** (§0.5/M3) — the most trustworthy signal |
| `moved-only` | ✅ **76% of every change-set — a 25× noise reduction** (§0.5/M2) |
| `outbound-edge-changed` | ✅ Confirmed (occurrence-containment scan) |
| `inbound-edge-gained` / `inbound-edge-lost` | ⚠️ **Calls half only.** The *implements* half **does not exist** (§0.5/D2). `-lost` is free from the same map and answers *"is my dependency now dead code?"* |
| `body-changed` | ✅ **Confirmed invisible to SCIP.** The content hash is the only discriminator. |
| `doc-changed` | ✅ Independent of signature |
| `ownership-shifted` | Counts, **never per-symbol rows** (§0.5, ~380:1 noise) |
| `baselined` | Counts + residual, **never per-symbol rows** |

**The content hash:** SHA-256 of the **raw bytes** of the definition occurrence's `enclosing_range`
(§0.5/D4). **No normalization** (§0.5). Advisory `formatting_suspect` flag. Computed by the **CLI at
upload time** — it requires a real decoder — and is therefore **producer-asserted**.

**Container cascade:** a module symbol's `enclosing_range` is the **whole file**, so any edit anywhere
fires a content-hash change on it (**6.1% of rows**). Carry `kind` + `enclosing_symbol_id` so the
**consumer** collapses it. Do **not** filter server-side — that would be deciding materiality (§1.3).

**Row shape:** `index_id`, `symbol_id`, `change_class`, `kind`, `display_name`, `enclosing_symbol_id`,
`document_id_before/after`, `line_before/after`, **`signature_before/after`** (highest-value payload —
a steward reading `fn f(a: A) -> R` → `fn f(a: A, b: B) -> R` needs nothing else),
`content_hash_before/after`, `formatting_suspect`, `outbound_added/removed`, `inbound_added/removed`.

**It deliberately carries no body and no diff of the body.** The membrane says mechanical facts, never
judgment — and a body excerpt is where *"helpfully summarize the change"* starts.

---

## 5. Event model (additive)

| Event type | Payload | Projector effect |
|---|---|---|
| `code_index_ingested` | `{repository_id, commit_sha, ref_name, tool_name, tool_version, tool_config, project_root, text_encoding, scip_blob_hash, hash_sidecar_blob_hash, counts}` | read CAS blobs → expand into `kb_code_*` **and derive the change-set against the prior trunk index — one txn** |
| `code_index_superseded` | `{index_id}` | flip `is_superseded` |
| `code_index_pruned` | `{index_id}` | **projection-only GC** — drop that index's *occurrences*; **keep change-sets, `kb_code_symbols`, and the CAS blobs** |

**Two blobs, not one** (§0.5): the **untouched `.scip`** (so `blob_hash` is a pure function of indexer
output and CI re-runs dedup perfectly) plus a Temper-authored **content-hash sidecar** (256 KB–1.16 MB —
far too big for `kb_events.payload`, which carries manifests and hashes, never bulk).

**`kb_code_symbols` must NEVER be pruned** — a change-set row references a `removed` symbol *by id*;
prune the dictionary with the index and the chain dangles. It is cheap: **+36 symbols across a real
trunk step** (~35K/yr).

**Do not intern locals at all** (§0.5/T3). `local 0` is 7 bytes — **shorter than the 8-byte FK pointing
at it.** An occurrence carries `symbol_id` (global) **XOR** `local_id TEXT` (document-scoped), with a
`CHECK` — making the corruption **structurally unrepresentable**.

**Replay invariant:** drop `kb_code_*` → replay `code_index_ingested` (re-reading CAS) → rebuild
byte-identically **across a multi-step trunk chain**, including derived change-sets.

---

## 6. Retention (S2 — validated, §0.5/M1)

**Tip-full + a persisted change-set per trunk step.** The **occurrence table is constant at ~139K rows,
forever**; the change-set chain grows linearly (~1M rows/yr) and is **130–386× cheaper** than keeping
indexes. **Do not partition** (threshold in §0.5). Persist `moved-only` as a **count, not rows** — 76%
of every change-set, zero material value, fully recomputable from CAS by replay.

**Blob growth:** ~3.2 MB compressed per index per commit, kept forever by design → **~28 GB/yr** at one
trunk commit/hour. Does not change the design. Somebody should see the number first.

---

## 7. The spikes

| # | Spike | Status |
|---|---|---|
| **S1** | The watching model | ✅ **Complete.** Disproved D1; found T1, T2; proved M4. |
| **S2** | Change-set & materiality | ✅ **Complete.** Disproved D2, D4; found T4; proved M1, M2, M3, M6. |
| **S3** | `temper-scip` decoder & ingest | ✅ **Complete.** Disproved D3, D4; found T2, T3, T5, T6; proved M5. |
| **S4** | Event, projector, replay & prune | Pending. Now also owns: the two-blob payload, `kb_code_symbols` never-pruned, chain gaps. |
| **S5** | The judgment membrane | Pending. Unaffected by the spike findings. |
| **S6** | Grounding read surface | Pending — **and at risk.** `code_implementations` has **no data** on Rust (§0.5/D2). **S6 must first measure `scip-typescript`**: does it populate relationships, and do its descriptors embed the **file path** (which would void the file-move payoff for TS entirely)? **Both are load-bearing and neither has been measured.** |

---

## 8. Risks

- **Membrane erosion** — the entire value proposition depends on never merging the two graphs. Guard
  with an explicit invariant + test **in both directions** (S5).
- **Notification firehose** — if materiality is decided in SQL, stewards drown and stop reading. §1.3 is
  the mitigation, and it is load-bearing.
- **A feed that reports silence** — the failure mode that is **worse than reading nothing at all**, and
  the one we measured happening (§0.5). Mitigated by the content hash, and by making
  `unresolved_paths` a first-class output rather than an empty set.
- **Per-indexer capability drift** — relationships, `enclosing_range`, and packed-vs-typed ranges are
  **properties of the indexer, not of SCIP.** Every number in this spec is rust-analyzer's.
  **`scip-typescript` is entirely unmeasured.**
- **Trunk-chain gaps** — a steward reading "nothing changed" across a hole is worse than one reading
  nothing. Gaps must be detected and surfaced (S4).
- **Producer-asserted hashes** — the server cannot verify a content hash (it has no source). This is
  what machine-principal attribution is for, and it is a real trust boundary to name out loud.

---

## 9. Explicitly deferred

- **Churn × judgment analytics.** **No new machinery** — a `GROUP BY` over data S2 and S5 already
  produce. Churn alone is noise; judgment alone has no denominator. **Harvest it; do not build it.**
- **Fuzzy code search**, **cross-repo `external_symbols`**, **region derived from the curated graph**,
  **push-based subscription** (the watermark model makes this a later optimization, not a redesign).

---

## 10. Appendix — substrate citations

- Ledger + strict event-type registration: `migrations/20260624000001_canonical_schema.sql:465-506`.
- Replay: `crates/temper-substrate/src/replay.rs`, `tests/replay_roundtrip.rs`.
- Curated edges (kept separate): `kb_edges` `canonical_schema.sql:628-650`; `edge_kind` enum `:95`.
- Contexts + authz: `contexts_readable_by` / `context_authorable_by_profile`
  `migrations/20260712000010_context_read_predicates.sql:84-124,171-199`.
- Polymorphic anchors (the pattern `kb_code_subscriptions` follows): `kb_resource_homes.anchor_table`
  `canonical_schema.sql:279`; `kb_edges.home_anchor_table` `:638`.
- **Agent judgment envelope:** `crates/temper-agents/src/envelope.rs`; steward at
  `packages/agent-workflows/steward/`.
- Machine principals: `migrations/20260711000010_machine_clients.sql`.
- Span-locator provenance (the citation seam): `migrations/20260710000001_block_provenance_annotate.sql`.
- **Idempotency contract to copy (NOT a CAS):** `block_append` in
  `migrations/20260708000012_streaming_ingest.sql`.
- SCIP: `scip.proto` v0.9.0 (github.com/sourcegraph/scip); docs `scip-code.org`.
