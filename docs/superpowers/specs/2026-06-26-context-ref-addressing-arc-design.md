# Context-ref addressing arc — UUID-primary + decorated `@owner/slug` across every surface (design)

Status: approved (brainstorm 2026-06-26)
Scope: temper-core · temper-api/substrate · temper-mcp · temper-cli · temper-ui · `/temper` skill (generated)
Sibling-of: WS6 Spec A (resource addressing-collapse) — this is the same discipline applied to **contexts**.
Follows: Search Beat 2 (Surface A, PR #183), which deliberately deferred context filtering to this arc.

## 1. Problem

A context is addressed by bare **`name`** on every surface today (`--context temper`,
`?context_name=temper`, MCP `context_name`, UI route `[context]`). In a single-user vault that is
merely sloppy. For the imminent **multi-org, multi-user** release it is a correctness bug: a principal can
see several contexts that share a `name` across the teams they belong to and their own personal space.
`kb_contexts` makes only `(owner_table, owner_id, slug)` unique — **`name` is a non-unique display label**.

Two live defects already follow from name-addressing:

- `context_service::resolve_by_name` does `WHERE c.name = $2` visibility-gated and `fetch_optional` —
  **silent first-match** when two visible contexts share a name.
- Resource-list `filtered_visible_page` filters `c.name = $2` **directly in SQL** — same silent first-match.

And Search Beat 2 left `unified_search`'s `p_context_id` **dormant** (`search_select` passes `None`)
precisely because wiring search's context filter in isolation — by name — would split it from every other
name-based surface. The fix is cross-surface and atomic in spirit: convert **all surfaces together** to an
unambiguous addressing scheme, then wire the dormant filter.

The scheme mirrors resources: **UUID-primary, with a decorated human form** resolved through the unique
`(owner, slug)` natural key. **Bare `name` is rejected as an addressing key** (it stays a display label).

## 2. Current-state ground truth (verified against the live tree, 2026-06-26)

- **Resource ref precedent** — `temper_workflow::operations::refs`: `parse_ref(&str) → ResourceId`
  (bare UUID **or** decorated `…-<uuid>`, trailing-UUID-only, **pure/no-DB**); `sluggify`, `decorated_ref`.
- **Owner markers already exist** — `temper_core::validation::validate_owner_pattern` accepts `@handle`
  (personal) / `+team` (team): lowercase alnum + hyphens, leading-alnum. `OwnerPatternError` enum present.
- **Schema** — `kb_contexts(id PK, owner_table CHECK in (kb_profiles,kb_teams), owner_id, slug, name,
  created)`, `UNIQUE (owner_table, owner_id, slug)`, `idx_kb_contexts_owner(owner_table, owner_id)`.
  Owners: `kb_profiles.handle` (global UNIQUE) / `kb_teams.slug` (global UNIQUE). `slug` is **NOT NULL** and
  already populated for every row → **no data migration needed**.
- **Slug ≠ slugify(name) is possible** — `context_service::next_unique_context_slug` bases the slug on
  `sluggify(name)` but **appends a numeric suffix on collision**. The write path
  `writes::resolve_context` resolves by `slugify(name)` and would silently hit the wrong row in that case —
  a latent bug the slug-keyed ref eliminates. For the 6 existing contexts (`general`, `knowledge`,
  `storyteller`, `tasker`, `temper`, `writing`) name == slug, so the cutover is benign for current data.
- **`@me` precedent** — `substrate_read.rs` already treats `@me` as a literal owner convention resolving to
  the caller's `profile_id` (for the resource-list `owner` filter).
- **Name on the wire today** — `context_name: String` on `IngestPayload`, `ResourceListParams`,
  `SearchParams`; MCP `CreateResourceInput`/`ListResourcesInput`; CLI `--context`/`--context-to`
  (`Option<String>`); UI route `[context]` → `?context_name=`. UI **create** instead sends `kb_context_id`
  (UUID), resolved name-ward by `context_service::resolve_name_by_id`.
- **Skill is generated** — sources live in `crates/temper-cli/skill-content/` + `agent-skills/`; the
  `temper skill install` command (`commands/skill.rs`, `templates.rs`) assembles them, stamps a
  `config-hash`, and writes `~/.claude/skills/temper/`. The served `## Contexts` list + `--context <ctx>`
  examples originate there. Editing the installed files directly is wrong — edit the repo sources, regen.

## 3. Design

### 3.1 The parser — `parse_context_ref` (temper-core, pure, no DB)

A sibling to resources' `parse_ref`, returning a structured ref so resolution (which needs a principal +
DB) stays out of the pure layer.

```rust
// temper-core
pub enum ContextOwnerRef {
    Me,              // "@me"
    Handle(String),  // "@<handle>"
    Team(String),    // "+<team-slug>"
}

pub enum ContextRef {
    Id(Uuid),                                        // bare UUID — canonical
    OwnerSlug { owner: ContextOwnerRef, slug: String }, // decorated
}

pub fn parse_context_ref(s: &str) -> Result<ContextRef, ContextRefError>;
```

Grammar (accept/reject is load-bearing — it is the spec):

| Input | Result |
|---|---|
| `<uuid>` | `Id(uuid)` |
| `@me/<slug>` | `OwnerSlug { Me, slug }` |
| `@<handle>/<slug>` | `OwnerSlug { Handle(handle), slug }` |
| `+<team-slug>/<slug>` | `OwnerSlug { Team(team_slug), slug }` |
| `temper` (no sigil, not a UUID) | **Err** `BareNameRejected` |
| `@handle` / `+team` (no `/slug`) | **Err** `MissingSlug` |
| `@/slug`, `+/slug`, empty owner/slug | **Err** (reuse `validate_owner_pattern` + slug validation) |

The owner half is validated with the existing `validate_owner_pattern` (the `@`/`+` form **without** the
trailing context slug). The slug half is validated with the same slug rules contexts already enforce
(lowercase alnum + hyphens). Pure ⇒ exhaustively unit-testable with no DB.

### 3.2 The resolver — `resolve_context_ref` (one async fn, server-side)

The single source of truth. Lives where the other context reads live (temper-api `context_service`,
backed by substrate). **Visibility/membership gated** — this is the multi-org correctness core.

```rust
// temper-api (services), or substrate readback called from there
pub async fn resolve_context_ref(
    pool: &PgPool,
    principal: ProfileId,
    r: &ContextRef,
) -> ApiResult<ContextId>;
```

| `ContextRef` | Resolution | Failure |
|---|---|---|
| `Id(uuid)` | row exists **and** visible to `principal` | `NotFound` (absent or not visible) |
| `OwnerSlug{Me, slug}` | `(kb_profiles, principal, slug)` | `NotFound` |
| `OwnerSlug{Handle(h), slug}` | `handle → profile_id`; `(kb_profiles, profile_id, slug)`; visible to `principal` | `NotFound` |
| `OwnerSlug{Team(t), slug}` | `team-slug → team_id`; `(kb_teams, team_id, slug)`; `principal` is a member | `NotFound`/`Forbidden` |

Visibility for `Id` and `Handle` reuses the same gate the existing `resolve_by_name` query already
encodes (owned-by-principal **OR** shared via `kb_team_contexts`/`kb_team_members`). This function
**replaces** both `context_service::resolve_by_name` (the first-match-by-name bug) and the write-path
`writes::resolve_context` (slugify-name, self-only) — all context resolution funnels here.

> Not-found vs forbidden: to avoid leaking existence of contexts a principal can't see, `Id`/`Handle`
> misses return `NotFound`; only `Team` non-membership (where the team itself is discoverable) returns
> `Forbidden`. Final wording resolved in implementation against the existing `ApiError` conventions.

### 3.3 Wire contract — `context_name` → `context_ref`

Rename and re-type semantically (still a `String` on the wire; now a **ref**, parsed+resolved at the
boundary). One inbound concept, one resolver:

- `IngestPayload.context_name` → `context_ref`
- `ResourceListParams.context_name` → `context_ref` (drop the parallel `kb_context_id: Option<Uuid>` — a
  UUID is now just a valid ref; **one** way in)
- `SearchParams.context_name` → `context_ref` (and **wire it through** — §3.5)
- UI create request: drop `kb_context_id`, send `context_ref` (UUID is a valid ref)
- CLI resource-move `--context-to` → a ref

Each handler resolves `context_ref → ContextId` once, then filters SQL by the resolved **id**. This
**fixes** resource-list (`c.name = $2` → `c.id = $resolved`) and **lights up** search (§3.5).

### 3.4 Outbound — every context is copy-pasteable (`ref`, mirroring resources)

`ContextRow` gains a computed **`ref`** (decorated: `@<handle>/<slug>` or `+<team-slug>/<slug>`), and
resource/search/list result rows gain a **`context_ref`** alongside the display `context`/`context_name`.
Constructing the decoration requires surfacing the owner's `handle`/team-`slug` + the context `slug` in
those projections (today they carry owner *ids* + the context *name*). `@me` is **not** used outbound —
rows always print the concrete `@handle`/`+team` form so they round-trip for any reader. ts-rs regenerated.

### 3.5 Search — light the dormant filter

`search_select` stops passing `context_id: None`. It parses+resolves `params.context_ref` (when present)
to a `ContextId` and passes it as `unified_search`'s `p_context_id`. The SQL is **already written** for it
(`corpus` CTE: `p_context_id IS NULL OR EXISTS (… kb_resource_homes … anchor_id = p_context_id)`) — no
migration. An unknown/again-unresolvable ref is a hard error (not "whole corpus"), closing the Beat-2 C1
regression at the source.

### 3.6 Per-surface conversion (one branch, converted together)

- **temper-core** — `ContextRef`/`ContextOwnerRef`/`ContextRefError` + `parse_context_ref` (+ ts-rs);
  slug validator (extract/share with context-create if one isn't already public).
- **temper-api / substrate** — `resolve_context_ref`; rewire ingest, resource-list, resource-create,
  resource-move, `/api/search`, `/api/graph/subgraph`; delete `resolve_by_name`; collapse
  `writes::resolve_context` onto the new resolver; add the SQL filter-by-id; surface owner handle/slug in
  row projections (§3.4).
- **temper-mcp** — `context_ref` on `CreateResourceInput`/`ListResourcesInput` (+ doc strings); SearchParams
  inherited from core. No enum params ⇒ no `schemars(inline)` concern.
- **temper-cli** — `--context`/`--context-to` accept a ref; rewrite the `require_context` error text; **the
  projection/pull path is the gnarliest local piece** — `pull <ref>`, `cloud_backend/ctx.rs`'s
  `owner_for_context`, and `projection.rs::resolve_context_id` (today name-matches the contexts list) must
  consume the decorated ref / resolved id. Local projected-file layout already uses `@me/…`,`+team/…` dirs,
  so the on-disk shape is already ref-shaped.
- **temper-ui** — build `${owner}/${context}` → `context_ref` for list/search/graph; create sends
  `context_ref`; render the new `ref` field. Routes `[owner]/[context]` already carry the two halves.
- **`/temper` skill (generated)** — edit `crates/temper-cli/skill-content/*` + `agent-skills/*` (and
  `templates.rs` if any context example is baked): every `--context <ctx>` → decorated (`--context @me/temper`,
  `+team/general`), the session-start invocation, the `## Contexts` list, and the `require_context` error
  doc. Regenerate with `temper skill install`; the `config-hash` re-stamps.

## 4. Decisions

1. **Bare `name` is rejected**, not shimmed to `@me/<slug>`. The release ships to multi-person orgs within
   weeks; the friendly-shorthand convenience would bake a self-scoping assumption into the addressing layer
   exactly when collisions become real. Hard-reject now = right long-term posture (and a clean parser).
2. **Server-side resolution.** The wire carries the ref string; one `resolve_context_ref` resolves it with
   the authenticated principal. Single source of truth; CLI/MCP/UI stay thin; `@me` is resolvable (principal
   at the boundary); mirrors the existing server-side `writes::resolve_context`.
3. **Slug is the addressing key**, never `name`. The decorated path component is the context **slug** (the
   `(owner, slug)` unique half). `name` remains display-only.
4. **Full outbound `ref`** this arc (not deferred) — context/resource/search rows all carry a
   copy-pasteable decorated ref, matching resources' existing `ref` discipline.
5. **No data migration** — slugs already exist; this is wire + code only, additive-on-`main`-safe.
6. **Resolution funnels through one function** — `resolve_by_name` is deleted, `writes::resolve_context`
   collapses onto `resolve_context_ref`. No second resolver survives.

## 5. Test plan

- **Unit (temper-core, pure):** `parse_context_ref` accept/reject table in full — every row of §3.1,
  including `@me`, malformed owners, missing slug, bare-name rejection, UUID passthrough.
- **Integration (temper-api `test-db`):** `resolve_context_ref` for each variant; the **two ambiguity
  regressions** — two visible same-`name` contexts in different owners resolve distinctly by slug, and the
  slug-suffix-collision case (`next_unique_context_slug`) addresses the `-2` row; visibility gate (can't
  resolve a non-visible `@handle/slug` / non-member `+team/slug`); resource-list filters by resolved id.
- **E2e (`test-db`, real `/api/search`):** context-scoped search returns only in-context hits; unknown ref
  → error (Beat-2 C1 closed); known ref scopes. Fold in the deferred Beat-2 **graph-search e2e** (task
  019f05b1) here if convenient — same harness, same `/api/search` surface.
- **Skill:** `crates/temper-cli/tests/skill_test.rs` updated for the regenerated content + new `config-hash`.
- Full `cargo make check` + `cargo make test-e2e` (+ `prepare-api`/`prepare-e2e` after the SQL filter change).

## 6. Out of scope

### Rejected (load-bearing — resist scope creep back in)
- **Bare-name shorthand / `@me` elision on input.** Decided against (Decision 1). Inbound is always a ref.
- **A second / client-side resolver.** One server-side resolver only (Decision 2).
- **Addressing contexts by `name` anywhere.** `name` is display-only (Decision 3).
- **Renaming/restructuring `kb_contexts`.** Schema is already correct; this arc is wire + code.

### Deferred (in scope elsewhere / later)
- **Context CRUD ergonomics** (rename, re-slug, list-by-ref command surface beyond what filtering needs).
- **Cross-org context sharing semantics** beyond the existing team-membership visibility gate.
- **Search default-weight calibration** (the standing Beat-2 tuning task) — unrelated.

## 7. Open questions (resolve during implementation)
- **Error taxonomy** — exact `NotFound`/`Forbidden`/`BadRequest` mapping for each resolver miss (§3.2 note),
  matched to existing `ApiError` usage and the not-leaking-existence rule.
- **`ResourceListParams.kb_context_id` removal blast radius** — confirm no caller (UI/tests) still posts the
  UUID field before deleting it from the wire (vs. accepting it transitionally).
- **Skill `templates.rs` baked examples** — confirm whether any context example is in the Rust template vs.
  the markdown sources, so the regen covers every `--context` occurrence.
