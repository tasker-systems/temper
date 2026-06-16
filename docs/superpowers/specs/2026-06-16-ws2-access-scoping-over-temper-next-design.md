# WS2 — access-scoping over `temper_next` (consumer axis): design

**Date:** 2026-06-16
**Parent:** `2026-06-16-ws6-flip-readiness-strategy.md` (WS2 is flip-prerequisite #1).
**Status:** Design — approved decisions below; ready for writing-plans.
**Builds on (proven, do not redesign):** `2026-06-02-access-capability-model-design.md`, `2026-06-11-access-scaffold-scenario-proof-design.md` (PR #129).

## The reframe: WS2 is wiring, not model design

The access model, its visibility function, and a leak-safety proof **all already exist and are deployed**:

- `schema-artifact/02_functions.sql:121` — `CREATE FUNCTION resources_visible_to(p_profile uuid)` (consumer axis), plus `team_ancestors` (`:25`), `profile_effective_teams` (`:45`), the personal-team / system-membership triggers, and the producer-axis `resources_accessible_to_cogmap` (`:150`).
- `migrations/20260613000001_install_temper_next.sql` **already installs `resources_visible_to`** into the deployed `temper_next` namespace — so WS2 has **no migration step for the read function**.
- PR #129's access-scenarios (`schema-artifact/access-scenarios/context-share-access.yaml`) already assert the function correct: `visible_to` / `producer_reach` / `edge_visible_to` checks over a rich topology (alice/bob/carol/nomad, teams + context-shares + edge-home gating).

What is missing is purely the **executable wiring**: the `NextBackend` read path applies *no* principal, and there is no write-axis gate.

> **Invariant carried verbatim from the flip-readiness strategy:** *No flip-with-a-gap.* The single-tenant "safe-by-population" argument is rejected — WS2 must reproduce production's scoping for an arbitrary org with real differential access.

## Scope (decided)

- **Consumer axis only.** WS2 wires `resources_visible_to(profile)` into the workflow/KB read+write surfaces that flip. The producer axis (`resources_accessible_to_cogmap`) stays built-but-dormant until workstream 7 exercises cogmap agent reads. The producer axis is already proven, so deferring its *wiring* carries no design risk.

**Out of scope (other flip-prereqs, separate units):** `by_uri` re-addressing and MCP `get_resource`/`list_resources` enrichment reads (strategy step 2, surface-completeness); native-id write addressing / `ResourceRef::Scoped` collapse (strategy step 2); deployed-adapter feature enable (step 3).

## The gap, grounded

**Reads are unscoped.** Every `readback` function takes `&PgPool` with no principal:
- `crates/temper-next/src/readback/mod.rs` — `list(pool)` `:148`, `meta(pool,new_id)` `:221`, `resource_row(pool,new_id)` `:316`, `body(pool,new_id)` `:410`, `fts_search(pool,query)` `:444`, `vector_search(pool,emb)` `:504`, `neighbors(pool,new_id)` `:560`.
- `crates/temper-api/src/backend/read_selector.rs:12-13` — *"Reads are visibility-UNSCOPED at the §9 floor (access-scoping over `temper_next` is a named flip prerequisite, WS2)."*

**No write-axis function.** `02_functions.sql` defines the two *read* axes but **no `can_modify`/`can_write`**. Production gates writes with `assert_can_modify` (`crates/temper-api/src/services/resource_service.rs:517` — *"Check whether the profile can modify a resource. Returns Forbidden if not."*, `:529` returns `ApiError::Forbidden`). `temper_next` has no equivalent, so NextBackend writes are currently ungated.

## Design

### D1 — Production's scoping pattern is the CONFORM target

Production applies the function as a **JOIN**, not a per-row call:
```
FROM vault_resources_browse vb
JOIN resources_visible_to($1) rv ON rv.resource_id = vb.id
```
(`resource_service.rs:249` list, `:257` count, `:262` facets, `:348` show-by-id, `:370` show-by-slug). A non-visible resource falls out of the join; the single-row reads then `.ok_or(ApiError::NotFound)?` (`:357,380,424`).

**Deny semantics (CONFORM, load-bearing):** a not-visible resource returns **404 `NotFound`** on reads (never 403 — denying existence prevents an existence-leak oracle); writes return **403 `Forbidden`** via the modify gate. WS2 reproduces exactly this split.

### D2 — Thread the principal through `readback` (consumer axis)

Add a `principal: Uuid` (profile id) parameter to the readback read functions and apply `resources_visible_to(principal)` as a JOIN against `temper_next.kb_resources`:
- **Set reads** (`list`, `fts_search`, `vector_search`, `neighbors`) — JOIN-filter to the visible set. `neighbors` gates traversal endpoints by visibility, mirroring production's `graph_traverse_seed_scoped_visibility` (`migrations/20260420000004_*`).
- **Single-resource reads** (`resource_row`, `meta`, `body`) — JOIN-gate; absent ⇒ `None` ⇒ surface maps to 404.

The principal is the JWT-authenticated profile already available at each surface (the same value production threads into `resources_visible_to($1)` today). The `read_selector` arms (`list_select`/`get_content_select`/`get_meta_select`/`search_select`) and the MCP search path take it from their existing auth context and pass it down. **No `SET LOCAL` session-var indirection** — explicit parameter, conforming to the function's `(p_profile)` signature.

**Principal id mapping — preserve profile ids in synthesis (decided 2026-06-16).** `resources_visible_to(p_profile)` expects a `temper_next` profile id, but the auth'd principal is a *production* profile id. Synthesis currently re-mints profile ids (`bootstrap.rs:202` `insert_profile` inserts with the DB-default uuid, using `old_id` only for handle disambiguation), and there is **no read-time profile bimap** (unlike resources' `ResolvedIds`-by-`origin_uri`). Rather than add a read-time mapping layer, **synthesis preserves production profile ids verbatim** — `insert_profile` inserts the explicit `id = old_id` (PR#124 identity-as-input). Then `resources_visible_to(prod_profile_id)` resolves directly, no mapping. This is the targeted slice of the broader "preserve vs re-mint ids" question (profiles only — they are just principals, with none of the native-id-addressing implications resource ids carry); it tightens parity (synthesized `Pete.id == prod Pete.id`) and never widens the §9 floor (profile ids are not asserted invariants there). The personal-team / system-membership triggers key on the profile id and are unaffected.

### D3 — Add the write-axis `can_modify` function (forward migration) + wire NextBackend writes

The install migration is **frozen append-only** (the 4c critical fix: `migrations/20260613000001` is checksum-tracked and must never change in place). So the new write-axis gate lands as a **new forward migration** (`20260616…` lineage), idempotent, adding:

```
CREATE FUNCTION can_modify_resource(p_profile uuid, p_resource uuid) RETURNS boolean
```
expressed over the same machinery as `resources_visible_to` but on the **write capability** — `kb_resource_access.can_write` for profile-anchored grants and team-anchored grants on a reachable team, plus owner/originator. (Exact body grounded against `resources_visible_to`'s shape at plan time; CONFORM to the consumer-axis function's reachability CTE, swap `can_read`→`can_write`.)

NextBackend create/update/delete/relationship then call this gate **before any mutation** (CONFORM to the auth-before-writes rule and production's `assert_can_modify` placement), returning `Forbidden` on failure.

### D4 — Semantic drift guard (CONFORM)

The 4c semantic migration drift guard (committed migrations reconstruct the artifact schema, compared via normalized `pg_catalog` fingerprints) must keep passing — so `can_modify_resource` is **also added to `schema-artifact/02_functions.sql`** in the same change, keeping the artifact the design-master and the forward migration its faithful append.

## Proof (decided: scenario + thin parity — both)

**P1 — Wiring correctness (the new behavior), against the rich PR #129 topology.** Reusing the access-scenario harness machinery and the alice/bob/carol/nomad topology, assert over `temper_next` through the *wired read path* (not the bare SQL function):
- visible-set exactness — `list`/search return exactly `resources_visible_to(P)` for principal P;
- deny — a single-resource read of a not-visible resource returns 404;
- edge-home gating — `neighbors` honors edge-home visibility (the private-edge-between-public-endpoints crux);
- write gate — `can_modify_resource` denies a non-owner/non-granted writer (403).

**P2 — Production parity (no regression on real data), thin.** Extend the chunk-3 parity-read harness to be **principal-aware**: for the actual synthesized production topology, a principal-scoped `temper_next` read returns the same row/result *set* as the production scoped read for the same principal. Production's real access topology is effectively trivial (owner + public floor — synthesis ports no `kb_resource_access` grants or `kb_team_members`; it synthesizes a personal-team + `kb_team_contexts` auto-share per the §2 amendment, `crates/temper-next/src/synthesis/bootstrap.rs:288`), so P2 is deliberately thin — its job is to assert the wiring doesn't regress real reads, with P1 carrying the differential-access weight.

**Acceptance criterion:** P1 green over the scenario topology + P2 green over the synthesized production topology ⇒ the consumer-axis read/write surface of `temper_next` is access-correct, closing flip-prerequisite #1.

## Units (for writing-plans)

1. Preserve production profile ids in synthesis (`insert_profile` inserts explicit `id = old_id`) so the auth'd principal resolves directly (D2 principal-mapping).
2. Thread `principal` through `readback` read fns + JOIN-filter (D1/D2); read_selector + MCP search pass it down.
3. `can_modify_resource` — artifact `02_functions.sql` + forward migration + drift-guard (D3/D4).
4. NextBackend writes call the modify gate before mutation (D3).
5. P1 scenario-topology wiring tests (reuse access-scenario harness).
6. P2 principal-aware extension to the chunk-3 parity harness.

## Grounding citations (evidence, per implementation-grounding GD-1)

- `schema-artifact/02_functions.sql:121,150,25,45` — read-axis functions present; no write-axis function.
- `migrations/20260613000001_install_temper_next.sql` — `resources_visible_to` already deployed.
- `crates/temper-api/src/services/resource_service.rs:249,348,370` — JOIN scoping pattern; `:357,380,424` NotFound; `:517,529` Forbidden write gate.
- `crates/temper-next/src/readback/mod.rs:148,221,316,410,444,504,560` — unscoped read signatures.
- `crates/temper-api/src/backend/read_selector.rs:12-13` — unscoped, WS2 named.
- `crates/temper-next/src/synthesis/bootstrap.rs:288` — synthesized personal-team auto-share; no prod grant port.
- `schema-artifact/access-scenarios/context-share-access.yaml:68-83` — the rich-topology leak-safety checks WS2's P1 reuses.
