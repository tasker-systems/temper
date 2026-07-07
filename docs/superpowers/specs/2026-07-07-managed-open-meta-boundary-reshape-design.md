# Managed / open meta — the fate table *is* the boundary

**Date:** 2026-07-07
**Status:** Design approved; splits into two build phases (Phase 1, Phase 2), each with its own implementation plan.
**Supersedes framing of:** temper task `019f3740` (managed-vs-open ergonomics rethink) and `019f38f4-3dec`/`019f38f4-506f` (the `temper-llm-model` / slug bugs that fed it).

---

## Charter (load-bearing — read before touching anything)

**Temper is cloud-native only. There is no backward compatibility to preserve.** The local
projection vault is read-only, ripgrep-able scratch space — a raw on-disk overlay of cloud
state, never round-tripped, never kept in sync beyond projection. We design and build for the
system we *want*, not to preserve any current behavior. Do not spend effort on migration
shims, deprecation windows, or dual-read paths.

**The entire public-caller blast radius is exactly four generated/derived surfaces**, and all
four are updated *as part of* this work, never deferred:

1. MCP tool descriptions (schemars-derived from the input structs)
2. CLI shapes + `--help` (clap-derived)
3. utoipa OpenAPI docs (derived from the API handler input structs)
4. Skill documents (the `temper` skill's `reference.md` and any managed-meta prose)

There is no other caller contract. Changing the wire shape is safe precisely because these four
surfaces *are* the contract, and they regenerate from the code we change.

---

## Problem

`managed_meta` and `open_meta` are presented to callers as a near-symmetric pair of frontmatter
tiers, but they are not two flavors of the same thing:

- **`managed_meta`** is a *closed, temper-owned vocabulary*. Its keys carry baked-in
  expectations at the relational-table or operational-consistency level. A caller cannot invent
  one.
- **`open_meta`** is free-form, user-owned, "bring-your-own-tagging" `jsonb`. No schema.

Two concrete faults follow from blurring them:

### Fault 1 — the surface invites callers to write keys that silently migrate

The MCP `ManagedMeta` type is a typed object (good — it stops clients string-encoding nested
JSON), but it carries a `#[serde(flatten)] extra: HashMap<String, Value>` catch-all
(`crates/temper-workflow/src/types/managed_meta.rs:106-109`). A caller can put an arbitrary key
in `managed_meta.extra`; on write `key_fate` classifies unknown keys as `Property` (the
conservative carry, `crates/temper-substrate/src/keys.rs:76`); on readback
`is_managed_property_key` (`keys.rs:58-60`) does not recognize it, so it reconstructs into
`open_meta`. The key silently changes tiers with no signal. (This is *not* a per-doc-type
eviction — the split is deterministic via the fate table — but it is a real, silent tier
migration for any caller-invented key. The CLI cannot express this at all: every field is a
typed flag.)

### Fault 2 — identity fields are mis-filed as metadata (the broken-faith)

The typed `ManagedMeta` struct includes `temper-title` and `temper-slug` as fields. But those
are `KeyFate::Die` keys (`keys.rs:67`) — they are the resource's **identity**, already carried
authoritatively by `kb_resources.title` / `.slug`, and already exposed as first-class params
(`title`, `slug`) on the create/update inputs. A caller sees the same concept in two places and
cannot tell which is authoritative. This is the "things that should just be part of the wire
shape, lumped into managed meta" contract-faith break.

### Root cause — the managed vocabulary is triple-enumerated

The set of managed keys is spelled out in three places that must stay in lockstep:

1. `ManagedMeta` typed struct fields (`crates/temper-workflow/src/types/managed_meta.rs`)
2. `MANAGED_PROPERTY_KEYS` + `key_fate` (`crates/temper-substrate/src/keys.rs`)
3. doc-type schema `properties` + `split_managed_open` rules
   (`crates/temper-workflow/src/frontmatter/tiers.rs`, `crates/temper-workflow/schemas/*.json`)

The `temper-llm-model` bug (`keys.rs:108-115`) was a drift between #1 and #2: the key existed
conceptually and in the struct, but was missing from `MANAGED_PROPERTY_KEYS`, so it read back
into `open_meta`. That is not a one-off — it is the failure mode triple-enumeration guarantees
will recur.

---

## The invariant this design serves

> **`managed_meta` contains *exactly* the `KeyFate::Property` keys** — optional workflow +
> provenance metadata, every one defaulted or omittable. Keys that `Die` /
> `ReconcileToDocType` / `Edge` are identity / home / type / relationships, not metadata, and
> live as first-class fields on the wire shape.

The fate table already draws this line. `KeyFate` is really encoding *where a key belongs*:

| Fate | Keys | What they actually are | Where they belong |
|------|------|------------------------|-------------------|
| `Die` | `temper-title`, `temper-slug`, `temper-id`, `temper-context` | identity / home | **first-class wire fields** (`title`, `slug`, home `context_ref`/`cogmap`) — required where the resource needs them |
| `ReconcileToDocType` | `temper-type` | the doc-type | **first-class wire field** (`doc_type_name`) |
| `Edge` | `temper-goal` | a relationship | **first-class** (act / edge projection) |
| `Property` | `temper-stage`, `-mode`, `-effort`, `-status`, `-seq`, `-branch`, `-pr`, `-llm-model`, `-llm-run`, `-provenance` | optional workflow + provenance metadata | **this is `managed_meta`** — all of it optional, smart-defaulted |

Consequence: `managed_meta` becomes genuinely *meta* — optional metadata with smart defaults.
The only two schema-"required" Property keys (`temper-stage`→`backlog`, `temper-status`→`active`)
are satisfied by server-side defaults applied *before* validation
(`crates/temper-workflow/src/defaults.rs:11-71`, ordered ahead of validate in
`crates/temper-services/src/backend/db_backend.rs:245-264`). So **the caller is never required
to send anything into `managed_meta`.** "Required for a doc-type" (title, type, context) is
expressed by those being *required first-class fields on the wire shape*, not stringly keys
buried in a metadata bag.

---

## Current-state map (grounded reference for implementers)

- **Fate table:** `crates/temper-substrate/src/keys.rs` — `KeyFate` (`:9-26`),
  `MANAGED_PROPERTY_KEYS` (`:42-53`), `key_fate` (`:65-78`), `is_managed_property_key` (`:58-60`).
- **Typed managed shape:** `crates/temper-workflow/src/types/managed_meta.rs:29-110` —
  `Option<_>` fields with `#[serde(rename = "temper-*")]`, plus the `extra` bucket (`:106-109`).
- **MCP inputs/handlers:** `crates/temper-mcp/src/tools/resources.rs` — `CreateResourceInput`
  (`:26-70`), `UpdateResourceInput` (`:116-150`), `UpdateResourceMetaInput` (`:164-177`);
  handlers `:379-538` and `:695-794`; identity injection `ensure_managed_identity_keys` call at
  `:476`; title/slug mirror at `:743-748`.
- **CLI shapes:** `crates/temper-cli/src/cli.rs` — Create (`:298-350`), Update (`:407-486`).
  Routing/split: `crates/temper-cli/src/commands/resource.rs` —
  `build_partial_managed_meta_from_args` (`:1056-1083`), `build_partial_open_meta_from_args` +
  `PartialOpenMeta` (`:1095-1134`), `validate_update_args` (`:1287-1335`).
- **Write path (tier dissolve + defaults + validate):**
  `crates/temper-services/src/backend/db_backend.rs` — `properties_from_meta` (`:197-217`),
  `validate_managed_meta_pipeline` (`:245-264`), create (`:872-899`), update (`:984-1049`).
- **Identity/default helpers:** `crates/temper-workflow/src/operations/actions.rs` —
  `ensure_managed_identity_keys` (`:49-63`), `apply_defaults` (`:71-89`),
  `merge_managed_meta`/`merge_open_meta` (`:257-337`);
  `crates/temper-workflow/src/defaults.rs:11-71`.
- **Frontmatter split (projection):** `crates/temper-workflow/src/frontmatter/tiers.rs:20-65`
  (`split_managed_open`); `crates/temper-workflow/src/frontmatter/document.rs:252-308`
  (`set_managed_meta`).
- **Readback (inverse of §7):** `crates/temper-substrate/src/readback/mod.rs:240-284`
  (`readback::meta` buckets by `is_managed_property_key`).
- **Schemas + discovery:** `crates/temper-workflow/schemas/*.schema.json`
  (`base.schema.json` `required: [temper-id, temper-type, temper-context, temper-created,
  temper-title]`; `task.schema.json` `required: [temper-stage, temper-slug]`);
  `describe_doc_type` MCP tool at `crates/temper-mcp/src/tools/doc_types.rs`. Un-updatable set:
  `SYSTEM_MANAGED_FIELDS` (`crates/temper-workflow/src/.../fields.rs:56-67`).

---

## Phase 1 — Enforce closed-ness + make it discoverable

*Low-risk, entirely within today's shape. Reject and discoverability ship together — rejecting
unknown keys without a way to discover valid ones is hostile.*

**P1.1 — Reject unknown managed keys at the type boundary (Decision A).**
Delete `ManagedMeta.extra` (the `#[serde(flatten)]` catch-all — the sole leak vector) and add
`#[serde(deny_unknown_fields)]` to `ManagedMeta`. An unknown key under `managed_meta` becomes a
deserialization error, making the illegal state (a caller-invented managed key) unrepresentable
— parse-don't-validate, no fourth list to maintain. `extra` is dead weight for reads (readback's
`is_managed_property_key` is closed and never emits unknowns), so removal is safe. At the MCP
boundary, wrap the serde error to a caller-legible hint: *"unknown managed key `foo`;
caller-defined keys belong in `open_meta`."* (`deny_unknown_fields` cannot coexist with
`flatten`; removing `extra` is what unlocks it.)

**P1.2 — Discoverability of the managed vocabulary.**
`describe_doc_type` (MCP) surfaces the managed vocabulary with types / enums / defaults per
doc-type. CLI `resource create`/`update --help` names each managed flag and its allowed values.
A caller can see exactly what is legal the moment rejection turns on.

**P1.3 — Slug precedence, documented.**
State the rule wherever slug is surfaced: *slug is title-derived (`KeyFate::Die`); to override,
pass the top-level `slug`; a `managed_meta` slug is inert.* (Structural removal of the
`managed_meta` slug field is Phase 2; Phase 1 documents the precedence so callers stop being
surprised.)

**Phase 1 done when:** an MCP caller sending an unknown `managed_meta` key gets a clear
rejection naming the key + pointing at `open_meta`; `describe_doc_type` and CLI `--help`
enumerate the managed vocabulary with types; slug precedence is documented across the four
surfaces. Full MCP + CLI + API + skill parity.

---

## Phase 2 — Reshape the wire contract (the deeper change)

*Touches the shared input shapes + the `DbBackend` write path across all three surfaces
(CLI / MCP / API). Bigger, but bounded by the invariant.*

**P2.1 — Shrink `ManagedMeta` to the Property vocabulary only.**
Remove `temper-title`, `temper-slug` (and any `-id` / `-context` / `-type`) from the struct.
What remains is exactly the Property keys: `stage, mode, effort, status, seq, branch, pr,
llm-model, llm-run, provenance` — every field `Option`, every field optional on the wire.

**P2.2 — Promote identity / home / type to first-class *required* wire fields.**
`title`, `doc_type_name`, and home (`context_ref` / `cogmap`) are the single, required source on
the create input; `slug` optional-derived from title. These already exist as top-level params —
promotion is mostly *deleting the `managed_meta` duplicates*, retiring
`ensure_managed_identity_keys` and the handler title/slug mirroring
(`resources.rs:476`, `:743-748`), and writing identity straight to columns. Update mirrors this:
identity changes flow through top-level fields only; `managed_meta` on update carries only
Property keys to patch.

**P2.3 — Smart defaults make schema-required ≠ caller-required.**
`temper-stage`→`backlog`, `temper-status`→`active` stay applied server-side before validation
(already the pipeline order). The caller never has to send anything into `managed_meta`;
"required" is a storage invariant satisfied by defaults.

**P2.4 — Single-source the Property vocabulary.**
Kill the drift that caused the `temper-llm-model` bug: the `ManagedMeta` Property fields,
`MANAGED_PROPERTY_KEYS`, and `key_fate`'s `Property` arm must be provably in lockstep — one
shared source of truth, or a compile-time parity test that fails if any of the three drifts. The
struct is the authority the surface validates against; no fourth hand-maintained list.

**P2.5 — Regenerate the blast radius in-arc.**
MCP descriptions (schemars), CLI `--help` (clap), utoipa OpenAPI, and the `temper` skill docs all
update as commits within Phase 2 — full-surface parity per the repo's standing rule.

**Phase 2 done when:** `managed_meta` (all surfaces) accepts only the optional Property
vocabulary; identity/home/type are required first-class wire fields with no `managed_meta`
duplicate; a create with no `managed_meta` at all succeeds with correct defaults; the Property
vocabulary is single-sourced with a drift-guard test; all four blast-radius surfaces reflect the
new shape.

---

## Phase 2 — resolved shape (2026-07-07 light brainstorm)

A grounded pass over the actual code fixed the shape of P2.2 (identity promotion) and resolved
two forks the phase sketch left open. This section is the authority the Phase 2 implementation
plan indexes into.

### The shrink is bigger than title/slug — 7 fields leave, each with a destination

`ManagedMeta` (`crates/temper-workflow/src/types/managed_meta.rs`) has 17 fields today. The
Property-only target is exactly the 10 keys in `MANAGED_PROPERTY_KEYS` (`stage, mode, effort,
status, seq, branch, pr, llm-model, llm-run, provenance`). So **7 fields are removed**, each to
a real destination:

| Leaving field | Fate | Destination |
|---|---|---|
| `temper-title` | `Die` | top-level `title` (already on create + update inputs) |
| `temper-slug` | `Die` | top-level `slug` (already exists) |
| `temper-type` | `ReconcileToDocType` | `doc_type_name` (create) / `move_to.type_to` (update) |
| `temper-context` | `Die` | home `context_ref` / `cogmap` (already exists) |
| `temper-goal` | `Edge` | **removed — see Fork 1** |
| `temper-updated` | system (`TIER1_SYSTEM_FIELDS`) | server-managed — dropped from caller input |
| `temper-source` | system (`TIER1_SYSTEM_FIELDS`) | origin-related — dropped from caller input |

### Update write path is the core of P2.2

`crates/temper-services/src/backend/db_backend.rs:980-1046` currently digs `temper-title` /
`temper-type` **out of** `managed_meta` to drive the `kb_resources.title` column and the doctype.
After promotion it reads identity from the top-level `cmd.title` / `cmd.slug` /
`cmd.move_to.type_to`; `managed_meta` on update carries only Property patches. Retire:

- `ensure_managed_identity_keys` (`crates/temper-workflow/src/operations/actions.rs:49-63`)
  entirely — it only existed to hash-match the old managed_meta identity storage.
- The MCP handler injection at `crates/temper-mcp/src/tools/resources.rs:477` (create) and the
  title/slug mirror at `:743`.
- `validate_managed_meta_pipeline`'s `identity_slug` injection simplifies: validation seeds
  identity from the top-level source, not from keys injected into the managed value.

The top-level `title`/`slug` fields on `UpdateResourceInput` **already exist** — today they are
shadowed by the managed_meta mirroring. Promotion is mostly *deleting the shadow*.

### Fork 1 — `temper-goal` (resolved: remove, defer the real feature)

Grounded finding: on the **live** write path `temper-goal` is inert — Edge-fated, so
`properties_from_meta` filters it out of `kb_properties`, and no live create/update consumer mints
the edge (the "edge pass" is substrate seed/scenario synthesis, not live CRUD). So the CLI
`--goal` flag and MCP `managed_meta.goal` currently write a key that becomes neither property nor
edge.

**Decision:** remove `temper-goal` from `ManagedMeta` and drop the inert CLI `--goal` write. Live
goal-edges go through the relationship/edge path (`assert_relationship`). Promoting `goal` to a
first-class field with **live** edge projection is a genuine new feature, captured as its own
follow-up task (`019f3d55`), not smuggled into the boundary reshape. Phase 2 stays a pure
reshape.

### Fork 2 — meta-only PUT path (resolved: keep Property-only)

With `ManagedMeta` shrunk, the meta-only path (`UpdateResourceMetaInput` /
`PUT /api/resources/{id}/meta`, `resources.rs:164-177`) can no longer carry title/type.

**Decision:** keep it Property-only (`managed_meta` + `open_meta`). Identity / type changes require
the full `update_resource` path. The meta-only path stays exactly "patch workflow metadata without
re-chunking the body." Its non-Option `managed_meta: ManagedMeta` shape is coherent — Property-only,
still required-shaped.

### Single-source (P2.4) — recommended shape

`MANAGED_PROPERTY_KEYS` (`crates/temper-substrate/src/keys.rs:42-53`) is already the authoritative
Property set, and `key_fate` matches on it. Add a compile-time drift-guard test asserting
`ManagedMeta`'s Property `serde(rename)` set ⟺ `MANAGED_PROPERTY_KEYS` ⟺ `key_fate`'s Property arm
all agree. Fold `split_managed_open` (`frontmatter/tiers.rs`) toward `MANAGED_PROPERTY_KEYS` where
practical. This structurally forecloses the `temper-llm-model` drift class.

### P2.5 also cleans up stale `extra`-bucket doc-comments

Phase 1 removed the `ManagedMeta.extra` catch-all, but several doc-comments still describe it and
must be corrected in-arc: `resources.rs:159-163` and `:486`; `managed_meta.rs:111-113` and
`:155-158`.

---

## Decisions & rationale

- **Reject (A), not warn/reroute/carry.** The user's model is that `managed_meta` is
  *definitionally* closed. The most legible expression of "definitionally closed" is that the
  type system won't let you say otherwise. Reject > runtime warning > silence. It is also the
  smallest surface (delete a bucket, add a derive) and structurally forecloses the
  `temper-llm-model` bug class.
- **Fate table as the boundary.** We did not invent a new taxonomy; we noticed the existing
  `KeyFate` already separates identity/home/type/relationships (`Die`/`ReconcileToDocType`/
  `Edge`) from metadata (`Property`). The design just aligns the wire shape to a line the
  persistence layer already draws.
- **One input shape, not per-doc-type input variants.** Because every Property key is optional
  with a smart default, nothing in `managed_meta` is caller-required, so no per-doc-type required
  shape is needed. The genuinely-required things (title, type, context) are the `Die`/`Reconcile`
  keys and become required first-class fields on the single input.
- **No backward compat** (see Charter). No shims, no deprecation windows.

## Open questions / risks

- **Error ergonomics of `deny_unknown_fields`.** serde's default message ("unknown field `foo`,
  expected one of …") is serviceable but leaks internal field names and does not point at
  `open_meta`. Plan P1.1 wraps it at the MCP boundary; confirm the wrap covers the API handler
  path too (utoipa/axum deserialization), not just the MCP tool.
- **`split_managed_open` (projection) consistency.** The frontmatter tier-split classifier
  (`tiers.rs:20-65`) is a *separate* enumeration from `key_fate`. It drives read-only vault
  projection only (no backward-compat concern), but Phase 2 should confirm it still splits the
  reshaped vocabulary correctly, and ideally fold it toward the same single source (P2.4).
- **`UpdateResourceMetaInput` non-Option fields** (`resources.rs:164-177`) take `managed_meta` /
  `open_meta` as required. Confirm Phase 2's shrink keeps this meta-only path coherent (managed
  now Property-only, still required-shaped).

## Related observations (out of scope — captured so they are not lost)

- **Open-key case inconsistency.** The CLI serializes open relationship keys kebab-case
  (`relates-to`, `depends-on`, `preceded-by`, `derived-from`;
  `commands/resource.rs:1095-1113`) while `base.schema.json` declares them snake_case
  (`relates_to`, …). This is an `open_meta` consistency wart independent of the managed/open
  boundary; note it, do not fold it in here.
