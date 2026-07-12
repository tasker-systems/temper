# Trunk-Change Awareness — a SCIP Code Graph Teams and Stewards Can Watch

**Date:** 2026-07-12 (reframed same-day — see §0)
**Status:** Design proposed, not yet approved for implementation. Six research spikes gate ratification.
**Scope:** A native, event-sourced code-intelligence graph in Temper, sourced from SCIP, built as a
**sibling projection family** on the substrate kernel — kept architecturally distinct from the curated
resource/edge/cogmap graph and joined to it **only by symbol-string citation**.

> **Companion research:** [`docs/research/2026-07-12-scip-code-graph-integration-research.md`](../../research/2026-07-12-scip-code-graph-integration-research.md).
> Read it for the **SCIP substance** — the wire format, the symbol grammar, the row shapes, the sizing.
> Its *framing* is superseded by this document: it was written for a navigation product.
>
> **Goal** (Temper): *Trunk-change awareness: a SCIP code graph teams and stewards can watch*
> (`@me/temper`, ref `…-019f56e1-49f0-78e0-a843-75a96274ce1e`). Six spikes `advance` it.

---

## 0. What changed, and why — read this first

The first cut of this spec answered *"how do we build a SCIP projection that doesn't corrupt the
curated graph?"* It answered that well. But it never asked **who is asking, and what for** — so every
spike's acceptance criterion validated the *construction* (schema holds, decoder round-trips, replay
rebuilds) and none validated an outward claim about what an agent could then **do**.

That gap was not cosmetic. Four of the six "open questions to decide in Phase 0" were **operational
questions wearing architectural costumes**, and were literally undecidable without a named caller.

The operational intention, now stated:

- **Temper does not compete with local tooling.** rust-analyzer, LSP, and ripgrep own the working
  tree, the IDE, and the in-flight agent session. They always will, and they should. **Interactive
  navigation of a dirty tree is out of scope by fiat** — which removes the single hardest problem in
  the space.
- **The consumer is a steward agent watching trunk.** A team's context or cogmap declares a code
  region it owns; when `main` moves, an agent judges whether what shipped **materially changes the
  plans in flight**.
- **The primary read is therefore not `code_definition`.** It is:

  > *"What landed on main, in the part of the code I care about, since I last looked — and does it
  > touch anything my plans depend on?"*

Everything below follows from that sentence.

### Questions this settles for free

| Old open question | Status |
|---|---|
| Occurrence partition + retention working set | **Answered** — tip-full + change-set chain; the big table is bounded by *one* index, not by history (§6). |
| Vantage default when multiple indexes exist | **Collapsed** — we only index trunk, so the default vantage *is* trunk tip. Residual: multi-tool-per-commit. |
| Diff semantics stable under line-shift | **Answered** — we diff **symbol sets**; position is not a change class. This is why SCIP earns its keep over `git diff`. |
| Eager vs lazy reference materialization | **Dissolved** — a latency question for arbitrary navigation. The question is now "what does a change-set carry." |
| Cross-repo `external_symbols` | Still open — but a later phase of multi-repo subscription, not a Phase-0 blocker. |
| First indexers for dogfood | Unchanged — `rust-analyzer scip` + `scip-typescript`. |

### Questions it opens (the right ones)

1. **The content hash and formatting.** Does a rustfmt reflow produce a phantom `body-changed`? (S2)
2. **Trunk-chain gaps.** CI fails, a merge is missed, the chain has a hole — a watermark now points at
   an index with no successor. (S4)
3. **Ownership drift.** When CODEOWNERS *itself* changes, the region's symbol set shifts for reasons
   that have nothing to do with the code. A distinct class, or suppressed? (S1)

These are for the spikes to answer. We deliberately do **not** pre-answer them here.

---

## 1. Load-bearing premises

### 1.1 The two graphs are different in kind

A SCIP index is a **mechanically-generated, commit-pinned, closed-ontology projection of source at one
commit**. Temper's resource graph is **curated, assigned-identity, open-vocabulary, ledger-as-truth**.

- **Do NOT** put code symbols in `kb_resources`, code edges in `kb_edges` (never widen `edge_kind` with
  `calls`/`imports`), or code structure through the region/lens/salience producer.
- **DO** reuse the substrate *kernel* — `kb_events` ledger, append-then-project, CAS blobs,
  contexts-as-home + team-DAG authz, machine principals, replay/drop-rebuild, span-locator provenance,
  and the **agent invocation envelope**.

### 1.2 The membrane — stated properly

> **The code graph is the unjudged record. All judgment lives in the curated graph and cites into the
> code graph.**

- **Code graph → steward: mechanical facts only.** Structure, signatures, content hashes,
  commit-pinned, attributed to the CI indexer's machine principal. No opinion, ever.
- **Steward → curated graph: judgment.** An authored event under an invocation envelope carrying
  `reasoning` + `confidence` + `rationale`, **citing the symbol string**.
- **Citation flows one way** (curated → code, by string). **Judgment never flows back.**

*"Code edges never enter `kb_edges`"* is a **consequence** of this, not the principle. And the deepest
reason the graphs must stay separate is not scale — it is that **the code graph is regenerated
wholesale every commit.** Anything you wrote onto it would be destroyed.

This half of the system **already exists**: `crates/temper-agents/src/envelope.rs` carries
`AgentAuthorship`, `ConfidenceBand` (`Tentative`/`Probable`/`Confident`), `Disposition`,
`InvocationClosed`; `reasoning`/`confidence`/`rationale` surface through the CLI and MCP tools. We are
not designing agent judgment — we are **pointing existing agent judgment at a new class of fact.**

### 1.3 Materiality is reported, not decided, by the schema

The change feed emits **classified change facts**. The **agent judges**, because *"does this change my
upcoming plans"* is a judgment only something holding the cogmap can make. Encoding materiality as a
SQL predicate is how this becomes a notification firehose nobody reads.

This is also what makes a **noisy** signal (the content hash, §4) affordable: a judging agent, with
traceability, stands between the signal and anyone's attention.

---

## 2. Architecture at a glance

```
CI (has source)                  Temper substrate kernel                code-graph projection family
──────────────────              ──────────────────────                 ───────────────────────────
rust-analyzer scip  ─┐          kb_events (append-only truth)           kb_code_repositories  (NEW)
scip-typescript      ├─ .scip ─► code_index_ingested ──project──►       kb_code_indexes       (the vantage)
+ per-defn content   ┘  blob    (payload = metadata + CAS blob_hash)    kb_code_symbols       (string id)
  hash (§4)             │                                               kb_code_documents
                        └─► CAS (opaque bytes, kept forever)            kb_code_occurrences   (tip only)
                                                                        kb_code_relationships
                                                                        kb_code_change_sets   (NEW, per trunk step)
                                                                        kb_code_subscriptions (NEW)

primary read:  code_changes_since(subscription)  → region-filtered, classified, since-watermark
support reads: code_definition / references / implementations / hover  @ trunk tip (grounding)

membrane:  steward reads facts ──judges──► authored event (reasoning/confidence) ──cites symbol string──►
```

**Authz seam, unchanged:** `kb_code_repositories.context_id → kb_contexts` ⇒ code facts inherit
`contexts_readable_by` / `context_authorable_by_profile`. **Deny = zero rows, never a 403.** A team
that shares the home context can watch the repo — `context share` already ships this. **No new authz
surface.**

---

## 3. The watching model (new — S1)

| Table | Grain | Notes |
|---|---|---|
| `kb_code_repositories` | one per repo | **homed in a context**; remote URL, declared trunk branch. One context may home several repos. |
| `kb_code_subscriptions` | one per watcher | `(context \| cogmap) → (repository, region_selector, watermark)`. A **curated-graph** object. |

`kb_code_indexes` gains `repository_id`; its key becomes `UNIQUE(repository_id, commit_sha, tool_name)`
— the **repository**, not the context, is the index's parent.

**The region selector is declared as paths and stored as symbols.** CODEOWNERS-shaped globs
(`crates/temper-services/**`) are the *input*, because that is how orgs already express ownership. They
resolve, per index, into a **SCIP symbol set**, and that symbol set is what we persist and diff
against. Consequence, and the whole point: **a rename or a file move is provably not a change to the
region.** A curated symbol set cited on a cogmap is an **additive refinement** — it adds to a declared
region, it does not compete with it.

*Deriving* the region from the curated graph's existing citations was considered and **deferred**:
organic graph derivation only works once the thing is modeled discretely and precisely, and it is
silent about exactly the code nobody has written about yet — which is the code most likely to surprise
you.

---

## 4. Change-sets & materiality classes (the core — S2)

The primary artifact is a **change-set between consecutive trunk indexes**, classified, restricted to a
watched symbol set **S and its dependency frontier** (the symbols S calls — *"the migration function I
depend on changed its signature"* must wake me).

SCIP carries `SymbolInformation` (`documentation`, `signature_documentation`, `kind`,
`enclosing_range`) and the four `Relationship` booleans. **It does not carry bodies.**

| Class | Visible in SCIP alone? |
|---|---|
| `added` / `removed` | Yes |
| `signature-changed` (incl. docstring) | Yes — `signature_documentation` |
| `moved-only` — shifted 40 lines, otherwise identical | Yes, and **correctly dismissed as non-material**. The SCIP payoff. |
| `outbound-edge-changed` — body rewritten, now calls something new | Yes, indirectly (the occurrence set inside its range changes) |
| `inbound-edge-gained` — someone new now calls/implements a symbol in S | Yes |
| `body-changed` — same symbol, same signature, same symbols touched, **different logic** | **NO. Invisible to SCIP.** |
| `ownership-shifted` — the region moved, not the code | N/A — comes from the selector (§3), not the index |

**That `body-changed` row is the problem.** *"They rewrote the retry logic"* — a `<` became a `<=`, a
branch flipped, a retry count moved — is exactly the change that wrecks a plan, and SCIP is structurally
blind to it: the symbol still exists, still has the same signature, still calls the same things.

**The one departure from pure SCIP:** at ingest, the CI indexer **also computes a per-definition content
hash** over each symbol's `enclosing_range` from the source tree it already has checked out. Cheap, and
it fits the "an index is a view from somewhere" model. Costs: the payload is **SCIP plus one
Temper-computed field**, and **the ingester must have source** (CI does — but this constrains where
ingest can run). This is affordable *precisely because* of §1.3: a judging agent absorbs the false
positives.

---

## 5. Event model (additive)

One additive migration seeds these `kb_event_types` (+ payload schemas) and their `_project_*`
functions. `_event_append` **rejects unseeded names**, so nothing fires until registered.

| Event type | Payload (typed struct) | Projector effect |
|---|---|---|
| `code_index_ingested` | `{repository_id, commit_sha, ref_name, tool_name, tool_version, project_root, text_encoding, blob_hash, counts}` | read CAS blob → expand into `kb_code_*` **and derive the change-set against the prior trunk index — one txn** |
| `code_index_superseded` | `{index_id}` | flip `is_superseded` |
| `code_index_pruned` | `{index_id}` | **projection-only GC** — drop the superseded index's *occurrences*; **keep the change-set and the CAS blob** |

**One event per index ingest**, not per symbol — heavy data is CAS-referenced by `blob_hash` (the
block-content pattern). Attribution: `emitter_entity_id` = the CI indexer machine principal
(`kb_machine_clients`); `invocation_id`/`correlation_id` thread multi-tool ingests of one commit.

**Replay invariant:** drop `kb_code_*` → replay `code_index_ingested` (re-reading CAS blobs) → rebuild
**byte-identically across a multi-step trunk chain**, including the derived change-sets.

---

## 6. Retention — the thesis to validate (S2)

The first cut treated `kb_code_occurrences` as an unbounded scaling risk needing partitioning plus a
working set of "tip + open-PR heads." **That was sized for navigation**, where any commit might be
queried.

For a trunk change feed you need **the tip, plus the deltas between consecutive trunk indexes**. Once a
change-set is derived and persisted, the *older* index's occurrences can be pruned entirely. A steward
three merges behind reads **three small change-sets, not three full indexes**.

> **If it holds: the big table is bounded by ONE index, not by history.**

CAS blobs persist forever, so any historical index stays rehydratable by replay. **Prove or kill this
with real numbers** — it is S2's headline deliverable, and no spike in the first cut would have found
it, because they were all sizing for the wrong access pattern.

---

## 7. The spikes

Six spikes gate ratification. **S1, S2, and S3 can run in parallel** — a spike may decode with the
off-the-shelf `scip` CLI rather than waiting on our production decoder. S1 and S2 are the long poles and
the new conceptual core.

| # | Spike | Mode/effort | Headline deliverable |
|---|---|---|---|
| **S1** | The watching model — repository, context homing, subscription & region selector | plan/large | **A pure file move produces an empty region diff — demonstrated.** Authz with zero new surface. |
| **S2** | Change-set & materiality classes | plan/large | A classified change-set over two real consecutive `main` commits, **with numbers**; the §6 retention thesis proved or killed. |
| **S3** | `temper-scip` decoder & ingest path | plan/large | A real `.scip` decodes and stores in CAS idempotently; golden round-trip; the augmented payload round-trips. |
| **S4** | Event, projector, replay & prune | plan/medium | Drop-rebuild byte-identical **across a trunk chain**; prune leaves the chain queryable; **a chain gap is detected, not silently skipped**. |
| **S5** | The judgment membrane — steward consumption & citation | plan/medium | **End to end:** a change lands, a subscribed steward wakes, a `decision` cites the exact symbol at a stated confidence — **no schema change**. Membrane test-guarded **both ways**. |
| **S6** | Grounding read surface (checkout-free) | plan/small | def/refs/impls correct vs `scip` CLI snapshots; an agent with **no checkout** grounds against `main`. |

---

## 8. Risks

- **Membrane erosion** — the entire value proposition depends on never merging the two graphs. Guard
  with an explicit invariant + test **in both directions** (S5), analogous to the additive-only-on-`main`
  guard.
- **Notification firehose** — if materiality is decided in SQL, stewards drown and stop reading. §1.3 is
  the mitigation, and it is load-bearing.
- **Trunk-chain gaps** — a steward that reads "nothing changed" across a hole in the chain is **worse
  than one that reads nothing at all.** Gaps must be detected and surfaced (S4).
- **Local-symbol scoping** — `local <id>` is unique only per `(index, document)`. Mis-scoping corrupts
  find-references *and* silently corrupts change-sets.
- **Content-hash false positives** — a rustfmt reflow reported as `body-changed` erodes trust in the
  feed. S2 decides normalize-vs-accept.

---

## 9. Explicitly deferred

- **Churn × judgment analytics.** Needs **no new machinery** — it is a `GROUP BY` over data S2 and S5
  already produce: mechanical churn per region × how often a steward judged a change material, and at
  what confidence. Churn alone is noise; judgment alone has no denominator. **Harvest it once the data
  exists; do not build it.**
- **Fuzzy code search** — embedding docstrings/signatures into `unified_search` for "find similar code,"
  strictly separate from structural navigation.
- **Cross-repo `external_symbols`** — in-repo first; cross-repo follows multi-repo subscription.
- **Region derived from the curated graph** (§3) — interesting, premature.
- **Event-layer subscription** (agents *pushed* changes rather than reading a feed on wake) — the
  watermark model makes this a later optimization, not a redesign.

---

## 10. Appendix — substrate citations

All verified against `migrations/` (canonical baseline, not the retired
`docs/event-sourced-architecture-design.md` row-shapes):

- Ledger + strict event-type registration: `migrations/20260624000001_canonical_schema.sql:465-506`;
  `…02_canonical_functions.sql:765-787`.
- Replay + invariant: `crates/temper-substrate/src/replay.rs`;
  `crates/temper-substrate/tests/replay_roundtrip.rs`.
- Curated edges (kept separate): `kb_edges` `canonical_schema.sql:628-650`; `edge_kind` enum `:95`.
- Contexts + authz: `kb_contexts` `:159-168`; `contexts_readable_by` / `context_authorable_by_profile`
  `migrations/20260712000010_context_read_predicates.sql:84-124,171-199`.
- **Agent judgment envelope (the membrane's other half):** `crates/temper-agents/src/envelope.rs`
  (`AgentAuthorship`, `ConfidenceBand`, `Disposition`, `InvocationClosed`); steward runtime at
  `packages/agent-workflows/steward/`.
- Machine principals: `migrations/20260711000010_machine_clients.sql`.
- Span-locator / annotate-only provenance (the citation seam):
  `migrations/20260710000001_block_provenance_annotate.sql`;
  `docs/superpowers/specs/2026-07-10-issue-355-annotate-only-provenance-and-span-locators-design.md`.
- Segmented ingest (large-blob upload): `migrations/20260708000012_streaming_ingest.sql`;
  `crates/temper-mcp/src/tools/ingest.rs`.
- Embeddings / search (fuzzy only): `crates/temper-ingest/src/{embed,chunk,pipeline}.rs`;
  `migrations/20260711000050_search_vector_scope_aware.sql`.
- SCIP: `scip.proto` (github.com/sourcegraph/scip); docs `scip-code.org`.
