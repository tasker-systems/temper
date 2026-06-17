# WS6 surface-completeness port — Spec A: addressing-model collapse

**Date:** 2026-06-17
**Status:** Design / spec. First of two specs for the WS6 surface-completeness port; worked sequentially with **Spec B (readback routing for `by_uri` + MCP enrichment)** and landed as a single PR.
**Parent strategy:** `docs/superpowers/specs/2026-06-16-ws6-flip-readiness-strategy.md` (§ "Surface-completeness port", item 2).
**Binding contract:** Adjudication 5 (slug-retirement surface contract), `docs/superpowers/specs/2026-06-12-ws6-convergence-delta-adjudication-design.md:253-279`.

## What this is

The flip-readiness strategy names three port gaps before the chunk-5 cutover. This spec covers the **addressing-model collapse** that underlies them: retiring `ResourceRef::Scoped` in favor of the decorated-ref identity contract (`UUID | sluggify(title)-<uuid>`, trailing-UUID-only resolution, one resolver). It is *backend-agnostic* — it changes how every surface addresses resources on the **legacy** `public.*` backend today, independent of the `kb_backend_selection` flag — and largely *compile-time*. The one place it touches `temper_next` is closing the native-id write-addressing gap as a free consequence.

Spec B (separate doc, same PR) then routes the two surviving lagging surfaces (`by_uri`, MCP `get_resource`/`list_resources` enrichment) through `temper_next` readback. The A/B seam (§1) keeps each spec coherent on its own.

## Why now

Per the flip-readiness bar (no flip-with-a-gap, adoption-grade): every lossy `NextBackend` port is a hard prerequisite. Native-id write addressing is currently stubbed — `crates/temper-api/src/backend/next_backend.rs:188,442` return `NotImplemented` for `ResourceRef::Scoped`. Adjudication 5 settled the identity contract that closes it; this spec lands that contract. It is also the largest standalone workflow-simplicity win in the converged surface (Adjudication 5: "the largest workflow-simplicity exposure (charter Q4)"), valuable on its own merits whether or not the flip follows.

## The settled contract (Adjudication 5, carried verbatim as the invariant)

> - **Identity in:** every surface accepts a bare UUID or the decorated form `sluggify(title)-<uuid>`. Resolution is **trailing-UUID-only** — the decoration half is parsed off and ignored, so a wrong or stale slug half is harmless by construction. Decorations are never stored, never authoritative, regenerated freely on title change.
> - **Name fragments are never identity.** Ref slots do not accept fragments — no fuzzy-match resolution, no ambiguity behavior, because no ambiguous input is ever a ref. Fuzzy finding lives in explicit search/list affordances whose output is decorated refs (copy → paste closes the loop).
> - **Identity out:** everything that prints a resource prints the decorated form. Vault projection filenames become `sluggify(title)-<uuid>.md` — every filename self-resolving.
> - **One resolver:** a single resolve affordance (UUID | decorated → resource), consumed by CLI, MCP, and the skill.

This spec implements that contract; it does not re-litigate it.

## 1. Scope & the A/B seam

**In Spec A:**
- Collapse `ResourceRef::Scoped`; one `parse_ref` resolver in `temper-core::operations`.
- Surface simplification: CLI `show`/`update`/`delete`/`edge`, MCP `get_resource` + relationship tools.
- Identity-out: a `decorated_ref(title, id)` helper; `ref` field at each surface's output boundary; vault projection filenames.
- Native-id write addressing in `NextBackend` (the `NotImplemented` arms disappear).

**Deferred to Spec B (named, not done here):**
- `by_uri`'s `temper_next` readback arm.
- MCP `get_resource`/`list_resources` relationship-enrichment over `temper_next`.

**The seam (load-bearing for A's coherence):** the `resolve_by_uri` *service* (`crates/temper-api/src/services/resource_service.rs:388`) and the `/api/resources/by-uri` endpoint (`crates/temper-api/src/handlers/resources.rs:102`) **stay** on legacy in Spec A. Spec A removes only the *CLI/MCP construction of scoped refs* and the API translator's scoped-ref arm (`crates/temper-api/src/backend/translators.rs:188`). A UUID ref resolves directly by id; nothing in Spec A needs the slug-scoped lookup path. Spec B owns routing the surviving by-uri/enrichment reads through readback.

## 2. The collapse + resolver (`temper-core`)

**`ResourceRef` (`crates/temper-core/src/operations/resource_ref.rs`).** The `Scoped { owner, context, doctype, slug }` variant is deleted. With one form left, `ResourceRef` collapses to a plain `ResourceId` at the command boundary (the enum's "exactly one form populated" guarantee is moot once there is one form). Plan-time call: whether `ResourceRef` survives as a thin newtype/alias or every `cmd.resource: ResourceRef` field becomes `cmd.resource: ResourceId` — decided against the ~30 call sites (the smaller mechanical diff wins; no behavior rides on it).

**One resolver.** `temper-core::operations::parse_ref(s: &str) -> Result<ResourceId, TemperError>`:
- bare UUID → `ResourceId`;
- `sluggify(title)-<uuid>` → parse the **trailing** UUID segment, ignore the decoration entirely (never compared to the stored title);
- anything else (a fragment, an empty string, a non-UUID tail) → typed error. **No fuzzy fallback** (contract: no ambiguous input is ever a ref).
- Migrates to `temper-workflow` at post-cutover crate extraction (tracked, not now).

**Decorated-form helper.** `temper-core::operations::decorated_ref(title: &str, id: ResourceId) -> String` = `format!("{}-{}", sluggify(title), id)`. Consolidates today's scattered forms: `ingest::slug_from_title` (`crates/temper-cli/src/projection.rs:231`) and `Vault::canonical_uri` (`crates/temper-core/src/vault.rs:106`). `sluggify` lands as the shared title→slug function both `parse_ref`'s inverse and the projector use.

## 3. Surface simplification (CLI + MCP)

**CLI (`crates/temper-cli/src/commands/`).**
- `show` / `update` / `delete`: a single decorated-ref-or-UUID positional, resolved by `parse_ref`. `--type` / `--context` / `--owner` **removed** from these commands (they existed only to scope a slug lookup; a global id needs no scope). Non-addressing flags (`--stage`, `--mode`, body input, etc.) are unchanged.
- `edge` source/target (`crates/temper-cli/src/commands/edge.rs:50`): decorated-ref-or-UUID positionals; the per-endpoint owner/context/doctype slug-scoping is removed.
- `create` keeps `--context` / `--type` (it creates *into* a context); `list` keeps them as **filters** (not addressing) — unchanged.

**MCP (`crates/temper-mcp/src/tools/`).**
- `get_resource` (`resources.rs:420`): collapses to id-only (decorated/UUID via `parse_ref`). The `slug + context_name` arm (`resources.rs:435-455`) and the `GetResourceInput.slug`/`context_name` fields are deleted.
- `update_resource` / `delete_resource`: already id-only — accept decorated/UUID through `parse_ref`.
- Relationship tools (`relationships.rs`): scoped-ref construction replaced by decorated/UUID refs.

## 4. Identity-out (rendering + filenames)

**`ref` at the output boundary.** Both surfaces emit `decorated_ref(title, id)` as a `ref` field where they print a resource — CLI output rows for `list`/`search`/`show`; MCP `EnrichedResource` (`crates/temper-mcp/src/tools/resources.rs:183`). **Not** added to the shared `ResourceRow` wire type — `ref` is derived from `title`+`id`, and the wire type stays clean (the same discipline that keeps `slug`/hashes off the §9 floor). Each surface computes it at serialization.

**Vault projection filenames.** `Vault::doc_file(owner, context, doc_type, slug)` (`crates/temper-cli/src/projection.rs:251,292`) changes its terminal segment from `<slug>.md` to `decorated_ref(title, id).md`; the `owner/context/doctype` **directory** hierarchy is unchanged (browsable organization; only the filename must self-resolve). The projector already has `title` and `id` in the row. The existing stale-file sweep (`crates/temper-cli/src/projection.rs:151-173`) walks `*.md` and removes files for resources *no longer present* — but it keys on the resource set, so an old `<slug>.md` for a resource that is still present (just renamed to the decorated form) is **not** an orphan it would catch as-is. Plan-time call: either the sweep is extended to remove files whose name ≠ the resource's current expected filename, or the writer removes the prior-named file when it writes the decorated name. Goal either way: a re-pull converges the vault to decorated names with no manual cleanup. (No code outside the vault reads these filenames — they are a projection cache — so this is cosmetic-convergence, not a correctness gate.)

## 5. Native-id write addressing (`NextBackend`)

The collapse closes the chunk-5 core gap for free:
- `crates/temper-api/src/backend/next_backend.rs:178` `resolve_new_id` takes a `ResourceId` (no `ResourceRef` match); maps prod→next via the existing `readback::ResolvedIds` bimap (`resolve_new_id` body, `:181-187`).
- The `ResourceRef::Scoped => Err(NotImplemented)` arms at `:188` and `:442` **disappear** — there is no scoped variant left to reject.

No new `temper_next` SQL; this is purely the consequence of the type collapse on the write path.

## 6. Testing & transition

**No DB migration.** Decorated refs are derived (title + id), nothing is stored; the collapse is compile-time. Conforms to "no premature backward compat" — `Scoped` is removed, not deprecated.

**Tests.**
- `parse_ref` unit tests: bare UUID; decorated form; **stale/wrong slug half is harmless** (resolves by trailing UUID regardless of decoration); fragment/empty/non-UUID-tail → typed error, no fuzzy fallback.
- `decorated_ref` round-trip: `parse_ref(decorated_ref(title, id)) == id` for representative titles (unicode, punctuation, empty-after-sluggify).
- CLI addressing tests rewritten off scoped refs (`crates/temper-cli/src/cloud_backend/translators.rs` test refs, `commands/resource.rs`).
- `NextBackend` resolve-by-uuid test covering the path that replaced the deleted `NotImplemented` arms.
- An e2e proving a decorated ref printed by `list`/`search` round-trips through `show`/`update`/`delete` (copy→paste loop), on the legacy backend (Spec A is backend-agnostic; the `next` path rides Spec B's harness).

**Companion change (flagged; out-of-repo, confirm artifacts at plan time).** The temper skill's command sequences speak `<slug> --type --context` (installed skill at `~/.claude/skills/temper/`, plus any in-repo `templates/skill.md` / `command-wrapper.md` the CLI generates from). These must update to decorated refs in lockstep with the CLI surface change, or the skill will emit broken commands. The plan inventories the exact artifacts; the Rust change and the skill-doc change ship together.

## Out of scope

- `by_uri` readback arm + MCP enrichment routing — **Spec B** (same PR).
- `correlation_id`→edge-handle rename and `ManagedMeta` genericization — non-gating §5 hygiene per the flip-readiness strategy; ride along whenever, not here.
- Crate extraction (`temper-workflow`) — post-cutover, last. The resolver lives in `temper-core::operations` until then.
- Re-mint-vs-preserve-id decision — temperkb-local, orthogonal (flip-readiness strategy § "one orthogonal call").

## Grounding citations (evidence, per implementation-grounding GD-1)

- `crates/temper-core/src/operations/resource_ref.rs:16-30` — `ResourceRef::{Uuid, Scoped}` (the type being collapsed).
- `crates/temper-api/src/backend/next_backend.rs:178-192,442` — `resolve_new_id`; `Scoped => NotImplemented` write-addressing arms.
- `crates/temper-api/src/backend/translators.rs:188-194` — API translator's `ResolveByUriParams` scoped arm (removed in A).
- `crates/temper-api/src/services/resource_service.rs:24-29,388-427` — `ResolveByUriParams` + `resolve_by_uri` service (stays for Spec B).
- `crates/temper-api/src/handlers/resources.rs:102-107` — `/api/resources/by-uri` handler (stays for Spec B).
- `crates/temper-cli/src/commands/resource.rs:543,986` ; `commands/edge.rs:50` — CLI scoped-ref construction sites.
- `crates/temper-mcp/src/tools/resources.rs:420,435-455` — MCP `get_resource` slug+context arm; `:183` `EnrichedResource`.
- `crates/temper-mcp/src/tools/relationships.rs` — relationship scoped refs.
- `crates/temper-cli/src/projection.rs:151-173,231,251,292` — projection filename construction + stale-file sweep.
- `crates/temper-core/src/vault.rs:106` — `canonical_uri` (decorated-form lineage).
- `~30 ResourceRef::Scoped / ::scoped(` call sites across CLI, MCP, API, core (`commands.rs`, `actions.rs`) — the collapse blast radius.
- Adjudication 5 + §D: `docs/superpowers/specs/2026-06-12-ws6-convergence-delta-adjudication-design.md:253-279,437-439`.
