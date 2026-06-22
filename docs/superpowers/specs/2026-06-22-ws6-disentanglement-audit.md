# WS6 Endgame — substrate-vs-scaffolding disentanglement audit (`crates/temper-next`)

Working artifact for the [migration endgame](2026-06-22-ws6-migration-endgame-design.md). Resolves
the spec's named **central analysis task**: per-item classification of `temper-next/src` into what
SURVIVES the namespace collapse (substrate, re-homed onto the canonical schema) vs what is DELETED
(scaffolding). Evidence-grounded (every verdict cites callers `file:line`), audited 2026-06-22 by a
six-way parallel read of the crate; load-bearing claims re-verified directly.

---

## The correction this audit makes

The endgame spec's first-pass table classified at the **module** grain and left a band as
*"mixed — audit per-item"* (`write`/`writes`/`payloads`/`ids`/`fingerprint`/`content`/`embed`) and
*"keep — verify each"* (`scenario`/`replay`/`events`). The audit's finding:

> **The "mixed" band is not mixed — every one of those modules is KEEP (substrate).** The real
> disentanglement is at the **symbol** grain inside the *scaffolding* modules: `synthesis/` is
> confirmed scaffolding, but it **harbours permanent live-path code** that must be carried out
> *before* the module is deleted. A blanket "delete `synthesis/`" would delete the live write
> path's property-classification logic (`key_fate`) and break compilation (`slugify`).

So the module-level table conflated *a module's primary role* with *every symbol's fate*. The
corrected model is three buckets: **KEEP**, **DELETE**, and **RE-HOME** (a survivor symbol carried
out of a deleted module).

---

## A. Module verdicts

| Module | Verdict | Decisive evidence |
|---|---|---|
| `ids.rs` | **KEEP** | Pure typed-UUID newtypes, zero SQL. Live path constructs them (`next_backend.rs:322,351,445–525`); `temper-agents` re-exports `InvocationId` (`envelope.rs:13`). Re-homes verbatim. |
| `fingerprint.rs` | **KEEP** | Pure SHA-256 over affinity inputs; cache-key/drift-signal on the Materialize path (`write.rs:37`, `drift.rs:125`). No I/O. Verbatim. |
| `payloads.rs` | **KEEP** | Typed event-payload wire contract; `EdgePolarity` on live path (`next_backend.rs:63–64`); re-exported by `temper-agents`. Only coupling: `verify_ledger_roundtrip` macro resolves against the `temper_next` `.sqlx` cache (regen, no text change). |
| `writes.rs` | **KEEP** | Live write composition — every fn called by `next_backend.rs:215–523`. **Carries the most rewrite work** (see §C). |
| `affinity.rs` | **KEEP** | Cogmap affinity math + `EdgeKind`/`Lens` types; `EdgeKind` on live path (`next_backend.rs:52–58`). Lifts to `temper-cogmap` unchanged. |
| `cluster.rs` | **KEEP** | Pure deterministic clustering (Lance-Williams agglomerative); zero schema awareness. Drives Materialize via `write.rs`. Lifts unchanged. |
| `drift.rs` | **KEEP** | Pure `classify`/`tier` + DB fns over canonical `kb_*` (unqualified). No literal coupling — inherits `substrate.rs`'s search_path. |
| `substrate.rs` | **KEEP** | Shared substrate access (`connect`/`load`/`cogmap_by_name`). **Carries the single load-bearing rewrite** — `substrate.rs:20` (see §C). |
| `write.rs` | **KEEP** | Cogmap-region materialization (binary `Materialize`, `main.rs:48`). Unqualified `kb_*` only; no literal coupling. |
| `content.rs` | **KEEP** | Pure chunk/embed-prepare; output consumed by live `writes.rs::prepare_block`. No SQL. *Sequencing caveat:* `synthesis/` imports `PreparedBlock`/`PreparedChunk` — do not delete `content.rs` with synthesis. |
| `embed.rs` | **KEEP** | "Job A" embed — reads `kb_chunks`, writes `kb_chunks.embedding` (`embed.rs:16–45`); binary `Materialize` (`main.rs:35`). Live product, not migration-only. |
| `events.rs` | **KEEP** | The single forward `fire()`/projector surface; live path reaches it via `next_backend.rs → writes.rs`. Builds forward `payloads::*` events; reads no `public.*`. |
| `replay.rs` | **KEEP** | **Forward-ledger** replay (walks canonical `kb_events` through the same `_project_*` halves) — *not* a legacy-trail backfill. Two fns (`formation_touched_since`, `content_touched_resources_since`) feed the live drift + incremental-materialize gates (`drift.rs:152`, `write.rs:259`). |
| `scenario/**` | **KEEP** | Surviving product feature (declarative cogmap authoring/replay over the canonical event surface). Zero `public.*`, zero `temper_next.`-qualified SQL. **Not on the live request path** — `next_backend.rs` never imports it; only `events.rs` depends on it (`model::LensDef`). Two surgical carve-outs (§B). |
| `synthesis/source.rs` | **DELETE** | Reads **only** `public.*` as the synthesis source (every query `public.kb_*`, `source.rs:34–211`). No survivors. |
| `synthesis/mod.rs` | **DELETE** | One-shot migration driver (`run`); sole non-test caller is the `Synthesize` subcommand (`main.rs:55`). No survivors. |
| `synthesis/bootstrap.rs` | **DELETE** (1 survivor) | Builds `temper_next` admin rows from `public.kb_contexts`. **Survivor: `slugify`** (§B). |
| `synthesis/key_fate.rs` | **RE-HOME (whole module, permanent)** | No SQL — §7 key-classification. `key_fate`/`KeyFate` on the live **write** path (`next_backend.rs:27,91`); `is_managed_property_key`/`MANAGED_PROPERTY_KEYS` used by readback + internally. **The standout finding** — permanent substrate mis-filed under `synthesis/`. |
| `synthesis/parity.rs` | **DELETE** (3 survivors, carry-until-shim-exit) | §8 cutover gate (scaffolding). Survivors `reconstruct_body`/`new_substrate_chunks`/`ReadChunk` are reached by live `readback::body` (§B). |
| `readback/mod.rs` | **DELETE-after-shim-exit** | Reconstructs old prod-shape rows from `temper_next.*` for the legacy read surface (`next_backend.rs` + `read_selector.rs`). Retired *with* the legacy shape by the sibling shim-exit spec, not at collapse. `neighbors`/`Neighbor` are test-only → die immediately. |

---

## B. Survivor symbols — MUST be carried out before their module is deleted

This is the audit's load-bearing deliverable. A blanket module deletion drops these.

| Symbol | Lives in (deleted) | Referenced by (surviving code, `file:line`) | Fate |
|---|---|---|---|
| `key_fate`, `KeyFate` | `synthesis/key_fate.rs` | live write path `next_backend.rs:27,91` (`properties_from_meta`) | **re-home (permanent)** |
| `is_managed_property_key`, `MANAGED_PROPERTY_KEYS` | `synthesis/key_fate.rs` | `readback/mod.rs:351,442` + internal `key_fate.rs:74` | **re-home (whole module travels together)** |
| `slugify` | `synthesis/bootstrap.rs:355` | live `writes.rs:64` (home-context resolution) + `scenario/access/loader.rs:136` (KEEP) | **re-home (permanent)** |
| `reconstruct_body`, `new_substrate_chunks`, `ReadChunk` | `synthesis/parity.rs` | live `readback::body` (`readback/mod.rs:608–609`) | **carry until shim-exit** (retire with readback) |
| `system_event_type_names()` | `scenario/bootseed.rs:22` | sole caller `synthesis/bootstrap.rs:105` (a DELETE target) | **delete-with-scaffolding** (becomes dead when synthesis goes) |

**Recommended re-home targets** (decide in the executable plan / shim-exit spec):
- `key_fate.rs` → a permanent home in the canonical write layer (it *is* the §7 property-tier
  policy; not migration logic). Candidate: a `temper-next` (→ future `temper-substrate`) `keys`/
  `properties` module, or `temper-core` if the policy should be surface-shared. **Not** under
  `synthesis/`.
- `slugify` → consolidate into a KEEP util reachable by both `writes.rs` and `scenario/`. Note
  `writes.rs` already imports it *from* bootstrap — invert the dependency.
- `parity::{reconstruct_body,new_substrate_chunks,ReadChunk}` → move alongside `readback/` (they
  are readback's body-reconstruction helpers, not synthesis's) so synthesis can be deleted at
  collapse while readback lives until shim-exit.

---

## C. Collapse-rewrite inventory — the `temper_next`-coupling sites

Concrete edits the collapse (rename/promote `temper_next` → the one canonical schema) must make.
Most KEEP modules are schema-*agnostic* (bare `kb_*`, resolved by the connection's search_path) — the
coupling concentrates in a few hooks:

| Site | What | Action at collapse |
|---|---|---|
| `crates/temper-next/src/substrate.rs:20` | `SET search_path = temper_next, public` in `connect()` `after_connect` | **Load-bearing.** The binary + cogmap + drift + embed + scenario + replay paths all inherit this one hook → the whole KEEP cogmap path resolves through it. Repoint to the canonical schema. |
| `crates/temper-next/src/writes.rs:83` | `SET LOCAL search_path TO temper_next, public` in `begin_scoped` | Every live write op shares it. Repoint. |
| `crates/temper-next/src/writes.rs:30` | `SELECT … FROM public.kb_profiles` (prod→next profile bridge) | **Dissolves entirely** — the two-hop prod→next identity resolution collapses to a single-schema lookup once the schemas merge. |
| `crates/temper-next/src/writes.rs:36,52,66–67` | `temper_next.`-qualified resolver queries (`kb_profiles`/`kb_entities`/`kb_contexts`) | De-qualify to the canonical schema. |
| `crates/temper-api/src/backend/next_backend.rs:172` (+ qualified `temper_next.kb_edges`/`kb_resource_homes`) | live write path search_path + qualified SQL | Repoint/de-qualify (live path; survives collapse). |
| `crates/temper-api/src/backend/read_selector.rs:234` + readback's 53 `temper_next.`-qualified refs | legacy read surface | **Deleted at shim-exit**, not rewritten. |
| `crates/temper-next/src/synthesis/mod.rs`, `bootstrap.rs` `SET LOCAL search_path` | synthesis | **Deleted with synthesis** — no rewrite. |
| per-crate `crates/temper-next/.sqlx` cache (+ `payloads.rs:558` macro) | offline cache targets the `temper_next` namespace | Regenerate against the canonical namespace (`prepare-next` task); re-unify with the workspace `public` caches (spec §Mechanics sqlx implications). |

**Pattern:** the literal `temper_next.` rewrite work is small and concentrated — two search_path
hooks (`substrate.rs:20`, `writes.rs:83`) carry the bulk of the KEEP surface by inheritance, plus a
handful of qualified resolver queries in `writes.rs`/`next_backend.rs`. Everything else is bare-`kb_*`
SQL that "just works" once the canonical schema is the one on the search_path. The bigger mechanical
cost is **cache regeneration + the two-source-of-truth reconciliation** (artifact `01/02.sql` builds
`temper_next`; `migrations/` builds `public`), which is the bootstrap-export seam, not this audit's.

---

## D. Sequencing implications (feeds endgame §Sequencing step 2 + 4)

1. **Before deleting `synthesis/`:** re-home `key_fate.rs` (whole module), `slugify`, and the three
   `parity` body-helpers (the last two: move next to `readback/`). Then `synthesis/{source,mod,
   bootstrap,parity}.rs` delete cleanly; `bootseed::system_event_type_names()` becomes dead and is
   removed in the same change.
2. **`content.rs` must not be deleted with `synthesis/`** despite synthesis importing its types —
   it is live-write substrate.
3. **`readback/` + the carried `parity` helpers retire together at shim-exit**, after the legacy
   prod-row shape is gone — a *later* step than the namespace collapse. The collapse de-qualifies the
   live `writes.rs`/`next_backend.rs` write SQL; it does **not** touch readback (which the shim-exit
   spec removes wholesale).
4. **`scenario/` survives** as the declarative-document product surface; only `slugify` re-home
   (its access-loader call) couples it to the deletion. It re-homes onto the canonical schema as a
   search_path no-op.
5. **The cogmap core (`affinity`/`cluster` + the `fingerprint`/`ids` pure helpers) is extraction-
   ready** — zero schema coupling, lib doc already flags "lift wholesale into temper-cogmap." This is
   the cleanest seam for the eventual crate extraction (last step, after shim-exit).

---

## E. Open follow-ups (not blocking the collapse)

- **`EdgePolarity::from_sql`** (`payloads.rs:89`) is synthesis-only (parses `public.kb_resource_edges.
  polarity`); inert, dies naturally when synthesis goes — drop it in the synthesis-deletion change.
- **`payloads`/`ids` are already the `temper-agents` contract surface** — the re-home of `key_fate`
  should respect that boundary (don't pull contract types into a migration-flavored module).
- **`kb_resource_audits` (empty post-flip)** + the `replay` masked-dump natural keys: re-verify the
  canonical table/column set still matches the `replay.rs:23–71` dump rules after the merge (a column
  rename there silently breaks replay determinism). Execution-phase check, gated.

---

## References

Endgame: `2026-06-22-ws6-migration-endgame-design.md` (§"What is scaffolding vs substrate" — this
audit resolves it). Schema diff: `2026-06-22-ws6-endgame-schema-diff.md`. Shim-exit (retires
`readback/` + the carried `parity` helpers): `2026-06-22-ws6-shim-exit-design.md`. Audited per-module
2026-06-22; survivor/search_path claims re-verified by direct grep.
