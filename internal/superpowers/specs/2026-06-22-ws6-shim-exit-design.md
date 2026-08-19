> **SUPERSEDED (2026-06-25)** by
> [`2026-06-25-ws6-shim-exit-native-shape-design.md`](2026-06-25-ws6-shim-exit-native-shape-design.md).
> This version predates the WS6 re-home (#168) and #166 collapse: stale file references
> (`next_backend.rs`), the endgame is now done (not a pending precondition), and its timestamp
> premise was wrong (timestamps are not event-derived today — see the successor §3). The crate split
> is also descoped from shim-exit into a sibling Spec B. Kept for decision history.

# WS6 Shim-Exit — surfaces native to the new schema, not shimmed

Design spec for retiring `NextBackend::reconstruct_resource_row` and the §9 invariant-floor
reconstruction, so api/cli/mcp/ui speak the **native** `temper_next`-derived shape rather than
a fabricated reproduction of the old production `ResourceRow`.

Sibling of the [migration endgame](2026-06-22-ws6-migration-endgame-design.md). **Sequenced
after** the endgame collapse (a singular schema) and **before** crate extraction. Backlog task
`019ee5a4-710a`.

---

## Problem

`reconstruct_resource_row` (crates/temper-api/src/backend/next_backend.rs:113) answers reads in
the OLD production `ResourceRow` shape by reading the §9 invariant fields from
`readback::resource_row` and **fabricating the rest**:

| `ResourceRow` field | Shim fill (today) | Native source (target) |
|---|---|---|
| `created` / `updated` | `Utc::now()` at read time — **wrong every read** | event ledger `occurred_at` (create event / last-mutation event) |
| `kb_doc_type_id` | `Uuid::nil()` (§7 dissolved the typed id) | drop the id; `doc_type_name` is authoritative (from `kb_properties`) |
| `slug` | `None` | native slug-less addressing (ref = preserved id; decorated slug is presentation) |
| `managed_hash` / `open_hash` | `None` | native content/property hashing if still needed, else drop |
| `kb_context_id`, ids, profiles, title, stage… | re-minted verbatim (preserved) | unchanged — already native |

Functional, but "shimmed, not evolved." Without this spec the shim — and the dead-shape
`ResourceRow` it serves — is permanent, and temper-ui/cli keep consuming fabricated timestamps
and nil ids.

---

## Native surface shape

Define the type api/cli/mcp/ui share over the unified schema directly. Principles:

1. **Timestamps are real and event-derived.** `created` = the resource's genesis event
   `occurred_at`; `updated` = the latest mutation event `occurred_at`. No read-time `now()`.
   This is the event-sourcing substrate doing its job (`temper-next/src/events`).
2. **Doc type by name, no typed id.** §7 dissolved `kb_doc_type_id` to a tierless property;
   the native shape carries `doc_type_name` only. Drop `kb_doc_type_id` from the wire type.
3. **Slug-less addressing.** Addressing is already ref-by-id (decorated slug is presentation,
   trailing-UUID resolution). The native shape omits a stored `slug`; CLI synthesizes the
   decorated form for display.
4. **Hashes: keep only if load-bearing.** `managed_hash`/`open_hash` fed the projection-cache
   diff. With cloud-only + the projection demoted to read-only cache, decide per-consumer
   whether native content hashing is still needed or droppable.

The native type lives in `temper-core` with `ts-rs` derives (shared Rust↔TS), replacing the
prod-shaped `ResourceRow` at the read boundary. **ts-rs regeneration** so temper-ui consumes
the native shape (generate-ts-types).

---

## Retiring the shim

- Replace `reconstruct_resource_row` callers (`show_resource`, the read selector's full-row
  `list`/`search` arms, update/create/delete echo-backs) with a native read that returns the
  native type directly from `readback`/the substrate — no field fabrication.
- Remove the §9 invariant-floor *reconstruction* path. (The §9 floor remains the *correctness
  property* proven at migration time; it is not a runtime read shape post-collapse.)
- After the endgame collapse there is one schema, so `readback` reads need no namespace gymnastics;
  fold `readback` into the substrate's native read API.

---

## Crate extraction (belongs here, last)

This is where `temper-substrate` (the schema-neutral kernel: events, resources, edges,
properties) and `temper-workflow` (Domain-A opinionation: task/goal/stage/mode/effort) split
from the temper-next crate, per [[project_shared_kernel_two_domains]] /
[[project_neutral_api_temper_workflow]]. Extraction is **last** — only once surfaces speak the
native shape, so the crate boundaries are drawn around real types, not shim-era ones.

---

## Sequencing

endgame collapse (singular schema) → **shim-exit (native shape + ts-rs regen)** → crate
extraction. Within shim-exit: define native type in temper-core → migrate each surface
(api → cli → mcp → ui) off the shim → regen ts-rs → delete `reconstruct_resource_row` +
prod-shaped `ResourceRow` read usage.

---

## Risks / gates

- **Usability floor (F3).** Post-cutover work must not destabilize working surfaces. Each surface
  migrates behind the endgame's end-to-end surface-parity test (extended to assert native fields:
  real timestamps, name-only doc type).
- **Timestamp semantics change is observable.** Consumers sorting/filtering on `created`/`updated`
  will see real values instead of read-time `now()` — intended, but call it out (ui ordering, any
  cached client assumptions).
- **Hash drop blast radius.** If `managed_hash`/`open_hash` are dropped, audit every consumer
  (projection cache, sync diff) first.

---

## Out of scope

- The collapse mechanics themselves (endgame spec).
- The OSS bootstrap export (bootstrap-export spec).

---

## References

- `crates/temper-api/src/backend/next_backend.rs` (`reconstruct_resource_row`, the read selector arms)
- `crates/temper-next/src/readback/`, `crates/temper-next/src/events`
- [[project_shared_kernel_two_domains]], [[project_neutral_api_temper_workflow]]
- Endgame: `docs/superpowers/specs/2026-06-22-ws6-migration-endgame-design.md`
