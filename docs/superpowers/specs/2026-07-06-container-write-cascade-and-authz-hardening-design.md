# Container-write → node-write cascade + cogmap-authoring hardening

**Status:** decided (cascade direction + split), specced for implementation.
**Date:** 2026-07-06.
**Supersedes the open question in:** `docs/auth/cognitive-map-authoring.md` §"can_modify_resource does not consult cogmap-authorship" (lines 33–35) and its three "Known hardening gaps" (F1/F2/F3).
**Tasks:** `019f3739` (precedence — plan) drives `019f371d` (hardening — build). They are one seam approached from two sides; this doc unifies them.

## The one seam

Write-authority for a resource homed in a container (cogmap or context) is decided in
two disconnected places today:

| Op | Predicate | Where enforced |
|---|---|---|
| **create** node into a cogmap | `cogmap_authorable_by_profile` (container write) | **surfaces only** — mcp `resources.rs:422`, api `ingest.rs:41`. `DbBackend::create_resource` (`db_backend.rs:799`) has **no** authz call. |
| **modify** an existing node | `can_modify_resource` (node's own owner/originator/per-resource grant, `20260701000003:109`) | `DbBackend::check_can_modify_next` (`db_backend.rs:340`) |

`can_modify_resource`'s three arms — (a) home owner/originator, (b) direct profile
`can_write` grant, (c) reachable-team `can_write` grant — **never consult the container's
write capability.** Result, demonstrated live on cogmap `019f2391`: a co-author holding
`cogmap_authorable_by_profile = t` can *create* nodes but cannot `fold`/`facet`/`update`
nodes another principal originated, until a per-resource grant is added on that node.

## Decision

**Container-write confers node-`rwx` on resources homed in that container — symmetrically
for cogmaps and contexts.**

Rationale (the deciding argument): this mirrors unix directory semantics. Whoever can
write a directory can already supersede any file in it — `rm` the entry and recreate it —
regardless of the file's own mode. A cogmap co-author *already* holds create + the ability
to fold-and-recreate; gating node-*modify* on node-ownership was therefore illusory
security, not a real control. The coherent model is: **directory-write ⇒ file-rwx.** At
this stage of the project there are no established team practices to preserve, so we adopt
the clean model directly rather than the conservative one.

Provenance is unaffected: the event ledger records the *actual* emitter/originator on each
fold/assert/facet, so "co-author B modified A's node" reads truthfully after the fact. The
cascade broadens *who may act*; it does not rewrite *who acted*.

### Authority broadening — signed off

Every cogmap co-author (holder of `cogmap_authorable_by_profile`) and every context
writer can now modify/fold/supersede **every** node in that container, including
steward-authored, provenance-bearing nodes. This is a deliberate collaborative-stewardship
authority increase, accepted as the intended model.

## The predicates

### New: `context_authorable_by_profile(profile, context)`

Contexts (unlike cogmaps) have an owner, so the predicate has an owner floor that the
cogmap predicate lacks:

```sql
CREATE OR REPLACE FUNCTION context_authorable_by_profile(p_profile uuid, p_context uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        -- personal-owned: the owner authors their own context
        SELECT 1 FROM kb_contexts c
         WHERE c.id = p_context
           AND c.owner_table = 'kb_profiles' AND c.owner_id = p_profile
        UNION ALL
        -- team-owned: a reachable member of the OWNING team authors it
        SELECT 1 FROM kb_contexts c
         JOIN profile_effective_teams(p_profile) e ON TRUE
         CROSS JOIN LATERAL team_ancestors(e.team_id) a
         WHERE c.id = p_context
           AND c.owner_table = 'kb_teams' AND c.owner_id = a.team_id
    )
    -- explicit write grant (profile- or reachable-team-anchored)
    OR profile_explicit_grant(p_profile, 'write', 'kb_contexts', p_context);
$$;
```

**Team-owned ⇒ members author** is deliberate and is NOT a regression toward the pre-Q-A
"membership implies write" that `20260701000001_cogmap_write_tightening.sql` removed. Q-A
removed write for teams merely *joined-for-read* to a cogmap. **Owning** a context is a
strictly stronger relationship than being joined to a cogmap for read; a team that owns a
directory has directory-write. If a future decision wants to *narrow* this to explicit
grants only, that is a separate deliberate flip with its own migration — do not "simplify"
the team-owner arm away on the assumption it duplicates Q-A. It does not.

### Changed: `can_modify_resource(profile, resource)` — add the cascade arm

Reproduce the current body (`20260701000003:109-133`) verbatim and add one `UNION ALL`
arm. New migration, additive `CREATE OR REPLACE` (additive-only-on-`main`; matches the
`20260704000001` convention of "body reproduced verbatim, only the new arm is new"):

```sql
        UNION ALL
        -- container-write cascade: whoever may author the home container may modify its
        -- nodes (unix directory-write ⇒ file-rwx). Cogmap homes are ownerless (explicit
        -- grant); context homes add an owner floor. See context_authorable_by_profile.
        SELECT 1 FROM kb_resource_homes h
         WHERE h.resource_id = p_resource
           AND CASE h.anchor_table
                 WHEN 'kb_cogmaps'  THEN cogmap_authorable_by_profile(p_profile, h.anchor_id)
                 WHEN 'kb_contexts' THEN context_authorable_by_profile(p_profile, h.anchor_id)
                 ELSE false
               END
```

`kb_resource_homes(anchor_table, anchor_id)` (`20260624000001:276`) already tells us the
container; arm (a) already joins this table, so the shape is familiar.

### Consistency: add the `kb_contexts`/`write` arm to `derived_access_profile`

`derived_access_profile` (`20260704000001:11`) currently returns `false` for
`kb_contexts`/`write`. With the predicate now existing, wire it so `can(profile,'write',
'kb_contexts',id)` is coherent:

```sql
        WHEN p_subject_table = 'kb_contexts' AND p_action = 'write' THEN
            context_authorable_by_profile(p_profile, p_subject_id)
```

## Folding in the hardening findings (task `019f371d`)

### F1 — backend-side create gate (now converges with the cascade)

`DbBackend::create_resource` gets the `cogmap_authorable_by_profile` check for a `Cogmap`
home, before `writes::create_resource_with`, denying with `TemperError::Forbidden`. Keep
the surface pre-checks (fast-fail + specific error text). This is the *same predicate* the
cascade uses for modify — after this change, both create and modify on a cogmap node route
through `cogmap_authorable_by_profile` inside `DbBackend`.

Bonus fix it delivers: the mcp surface currently denies with `invalid_params` (400-class)
while api denies with `Forbidden` (403). A backend-side `Forbidden` normalizes the deny
semantics across surfaces.

**Scope guard:** F1 stays cogmap-only, matching the task. Create-*into-context* is not
backend-gated today; whether it should be is a separate question (the cascade decided here
is about *modify*, not context *create*) — noted as a follow-up, explicitly **out of scope**.

### F2 — `invocation_open` read-vs-write

Resolved by the rule the cascade makes natural:

- **self-attributed open** (`parent: None`): require **write** —
  `cogmap_authorable_by_profile(originating)`. You are claiming a slot on the accountability
  ledger of a map; that is an authoring act. Closes the "reader opens inert self-attributed
  envelopes" noise vector.
- **delegated open** (`parent: Some`): keep the **read** gate; the substrate's
  parent→originating delegation lineage (`writes.rs:889`, `OpenParams.parent`) is the real
  control for delegated sub-agents. A parent that authored may delegate an open it wouldn't
  itself need write for.

Document the intent at the gate either way.

### F3 — stale comments (3 sites)

Correct the "team-cogmap membership" wording to name the explicit-grant predicate at:
`temper-mcp/.../resources.rs:422`, `temper-api/.../ingest.rs:42-43`,
`temper-services/.../cogmap_service.rs:57-63`. Post-cascade the truthful description is:
*create gates on `cogmap_authorable_by_profile` (explicit write grant); modify additionally
cascades from container-write.*

## Test matrix

For **create** and **modify**, across **cogmap** and **context** homes:

| Principal | create | modify own node | modify other's node |
|---|---|---|---|
| home owner / node originator | ✅ | ✅ | ✅ |
| container writer (cogmap grant / context owner-or-write-grant) — **the new capability** | ✅ | ✅ | ✅ **(was ❌)** |
| reader only (membership read, no write) | ❌ | — | ❌ |

Plus the two gap-fillers the grounding surfaced:
- **Direct-backend** create denial: `backend.create_resource(Cogmap home)` denies a
  non-granted principal at the command layer (not only via the surface). No such test
  exists today.
- **invocation_open** readable-but-not-write-granted: self-attributed open denied; the
  existing `create_into_unreadable_cogmap_is_forbidden` only exercises a *non-reader*.

## Implementation split (decision-first → re-split)

Two PRs, per the chosen strategy:

**PR A — authz seam (build, `019f371d` + `019f3739` implementation):**
- New migration: `context_authorable_by_profile`; `can_modify_resource` cascade arm;
  `derived_access_profile` context/write arm.
- F1: `DbBackend::create_resource` cogmap gate.
- F2: `invocation_open` self-attributed-requires-write / delegated-stays-read.
- Test matrix above + the two gap-fillers.
- `.sqlx` regen: new `query!` in `create_resource` and `open_invocation`; run the
  ritual (`cargo sqlx prepare --workspace -- --all-features` → `prepare-services` →
  `prepare-api`, per-crate last) and note the `sqlx::migrate!()` stale-cache trap
  (`cargo clean -p temper-api` before integration tests if a "function does not exist"
  phantom appears).

**PR B — docs + comments (`019f371d` F3 + doc reflection):**
- F3 comment corrections (3 sites).
- Rewrite `docs/auth/cognitive-map-authoring.md`: the §33-35 "does not consult
  cogmap-authorship" claim is now **false** — replace with the cascade model; F1 no longer
  a gap; F2 resolved; the gate map's `create`/`modify` rows updated; the "Known hardening
  gaps" section retired or converted to "resolved".
- Fold this decision doc's outcome into that canonical doc.

Docs-only PR B pays ~zero CI (docs-only scope detection). PR A carries the migration +
Rust + test weight.
