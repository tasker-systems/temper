# Code Quality Best Practices

**Purpose.** This is the **explicit lens for code review** in Temper. Reviews here check
*opinionated best-practice*, not just correctness — a change can be correct and still fail
review for bundling four responsibilities into one function, keying on a loose marker, or
swallowing an error to make a test pass. Correctness is the floor, not the bar.

This doc folds in and supersedes the terse **Code Quality Rules** that lived in `CLAUDE.md`
(which now points here). It draws on three external influences, adapted to Temper's stack:

- the sibling **tasker-core** project's `docs/development/best-practices-rust.md`,
- Microsoft's [Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/) (the `M-*` rule IDs below are theirs — shared vocabulary across both sibling projects),
- [rust-skills](https://github.com/leonardomso/rust-skills) and the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/).

A guideline earns its place here only if it is *beneficial for safety, cost-of-goods, or
maintenance*, *agreeable to experienced Rust developers*, and *practically applicable* — the
same bar Microsoft sets. Rules are opinionated defaults, not absolutes: deviate when you can
name the reason, and leave that reason in a comment or the PR.

---

## 1. The opinionated lens

### 1.1 One function, one responsibility — and keep it short

A function should do one nameable thing. If you cannot summarize it in a single clause
without "and", it is doing too much. Long functions that bundle distinct *phases* are the
single most common structural smell in this repo.

**Heuristics (defaults, not hard gates):**
- A function past **~60 lines** is a review flag — not automatically wrong, but it must
  justify its length (a flat match over many variants is fine; four sequential phases is not).
- When a function reads as *"phase 1 … phase 2 … phase 3 …"*, each phase is a helper.
  Extract them so the parent becomes a short orchestrator that names the phases in sequence.
- Prefer free functions for general computation; reserve associated functions / methods for
  construction and operations that genuinely need `self` (`M-REGULAR-FN`).

**Worked example — the motivating case for this doc.** `DbBackend::reconcile_apply`
(`crates/temper-api/src/backend/db_backend.rs`) was ~200 lines bundling: read-the-live-slice,
the resource create/update/no-op loop, the edge-assert loop, and the tombstone fold loop. The
fix is to extract `read_kernel_index` / `apply_resource_phase` / `apply_edge_phase` /
`apply_tombstone_phase`, each taking `&mut PgConnection`, leaving `reconcile_apply` a short
orchestrator. The diff stays behavior-identical; the *shape* tells the reader the four phases
at a glance.

### 1.2 Identity and keys: unique-by-construction, not loose markers

Distinguish a **key** (uniquely identifies a thing, safe to diff/dedup/join on) from a
**marker** (attribution or a hint, non-unique, never authoritative). Never let a loose marker
drift into a load-bearing key.

- Diff, dedup, and join on a value that is **unique by construction** — a UUIDv7 primary key,
  a stable landmark id. Reconcile keys on the entry's pre-generated `id`, *never* on
  `origin_uri`, precisely because `origin_uri` is loose, non-unique attribution.
- When two sides of a boundary must agree on an identity, both sides inject the canonical keys
  from a **single typed source** (the `ensure_managed_identity_keys` symmetric-defense pattern)
  — so the wire payload cannot drift between sender and receiver.
- Encode the key's uniqueness where it is created, not by convention downstream. A "we always
  set this so it's effectively unique" marker is a latent bug; make it unique-by-construction
  or treat it as the marker it is.

### 1.3 Parse, don't validate — encode constraints in types

Push invariants into the type system so illegal states are unrepresentable.

- Wrap domain ids in **newtypes** (`ProfileId`, `ResourceId`, `CogmapId`) rather than passing
  bare `Uuid`s — the compiler then refuses a target id where a source id belongs.
- Convert at the boundary with `TryFrom` / `FromStr`; once past the boundary, hold the parsed
  type, not the raw string. `EdgeKind::from_sql(...)` returning `Option` and erroring into a
  `BadRequest` is the pattern — parse once, then trust the type.
- Prefer `&str` / `&[T]` parameters over `&String` / `&Vec<T>`; accept the least specific type
  that does the job.

### 1.4 Names carry responsibility

- Avoid weasel-word names — `Service`, `Manager`, `Helper`, `Handler`, `Factory`, `Util` —
  that describe a role instead of a responsibility (`M-WEASEL-WORDS`). Name the thing for what
  it *does*. (`backend` / `services` here are established module-level terms, not per-type weasel words.)
- Keep identifiers concise; don't repeat the module name in every item (`M-SHORT-NAMES`).
- No stringly-typed branching over a bounded set you own — match on the enum directly, and
  encapsulate enum→value mappings as inherent methods, not scattered `match "literal"` arms.

### 1.5 Params structs over long signatures

Functions with **more than 5 domain-related parameters** take a params struct (the repo
already uses `KernelCreateParams`, `UpdateParams`, `KernelEdgeParams`). A params struct also
makes call sites self-documenting (`field: value`) and resists positional-argument mistakes.
`#[expect(clippy::too_many_arguments)]` is a smell to fix, not a suppression to keep.

### 1.6 Error handling and escalation

- **Libraries use `thiserror`** typed errors; binaries may use `anyhow`. Propagate with `?`;
  add context with `.map_err(...)` when the bare error loses the call site.
- **Never soften a contract to make a test or build pass.** If resolving a failure would mean
  swallowing an error, loosening a type, or weakening an assertion, **stop and report blocked**
  — escalate, don't soften. This applies doubly to dispatched subagents: a green suite bought
  by a silently-swallowed error is worse than a red one.
- **No `.unwrap()` / `.expect()` on fallible runtime values** in library paths. `expect` is
  acceptable only on an invariant guaranteed earlier — and then its message states *why* the
  invariant holds (`M-PANIC-MESSAGE`).
- A panic means *stop the program*; it is for detected programming bugs, not recoverable
  conditions (`M-PANIC-IS-STOP`, `M-PANIC-ON-BUG`). Return a typed error for anything a caller
  could reasonably handle.
- Authorization checks run **before** any mutation — never write-then-check.

### 1.7 Lint and suppression discipline

- Override a lint with `#[expect(lint, reason = "…")]`, never bare `#[allow]` — `expect` rots
  loudly when the suppression is no longer needed (`M-LINT-OVERRIDE-EXPECT`). Every suppression
  carries a reason.
- All public types implement `Debug` (`M-PUBLIC-DEBUG`); error and string-wrapper types
  implement `Display` (`M-PUBLIC-DISPLAY`).
- Hardcoded constants get a comment explaining the value, its rationale, and any external
  coupling (`M-DOCUMENTED-MAGIC`).

### 1.8 No "for now", no premature compat, no premature abstraction

- This codebase is young — **remove dead code, don't keep it "for compat."** Delete the old
  path; don't gate it behind a flag nobody flips.
- Never ship a "for now" workaround or placeholder. Revert a half-baked attempt to baseline and
  capture the real architecture as a task instead.
- Don't abstract on the first occurrence. Extract a shared helper on the *second* real call
  site, when the shape is known — but see §3 for the cases (SQL filters, wire types) where the
  cost of drift makes early extraction worth it.

---

## 2. Repository invariants (non-negotiable)

These are structural laws of the codebase, folded from the former `CLAUDE.md` rules. Violating
one is a blocking review finding regardless of correctness.

- **Typed structs over inline JSON.** Never `serde_json::json!()` for data with a known shape —
  define a struct so the compiler checks it.
- **Shared types at boundaries.** When Rust calls TypeScript (or vice versa), the wire type
  lives in `temper-core` with `ts-rs` derives; both sides share the generated type. Never
  hand-mirror a Rust struct as a zod schema.
- **Persistence is its own layer; surfaces dispatch through `DbBackend`.** SQL and persistence
  CRUD live in a dedicated persistence layer — historically `temper-api/src/services/`, now
  consolidating into **`temper-substrate`** (`writes` / `readback`) — *never* inline in a
  surface, and never mixed in with the behavior abstraction that calls it. Surfaces (HTTP
  handlers, MCP tools, CLI actions) build a backend per request and dispatch one operations
  command per inbound call for **writes** — they never call persistence directly and never inline
  `sqlx::query!()`. Read paths (list, show, get_meta, search) stay service-direct by design. (The
  layer is moving; the rule is invariant — keep CRUD out of endpoints and out of behavior code.)
- **Auth before writes.** Authorization precedes any mutation (also §1.6).
- **Profile scoping.** Every data query scopes through `resources_visible_to`,
  `can_modify_resource`, or equivalent — including async workflows, which verify access before
  writing.
- **Pino structured logging (TypeScript).** Use pino (`packages/temper-cloud/src/logger.ts`)
  with contextual field objects; structured fields, hierarchical names, redact sensitive data
  (`M-LOG-STRUCTURED`). No `console.log`.
- **Schema-required defaults at write time.** Doc-type schemas in `temper-core/types/schemas/`
  declare required frontmatter; creation and update paths populate every required field at
  write time via `apply_doc_type_defaults` / `Frontmatter::set_managed_meta` — never rely on a
  downstream backfill. Inject canonical identity keys (`temper-title`, `temper-slug`) via
  `ensure_managed_identity_keys` on **both** sides of the wire (§1.2).

---

## 3. SQL discipline

- Production queries live in the persistence layer (`temper-api/src/services/`, consolidating
  into `temper-substrate`) and use the compile-time-checked macros `sqlx::query!()` /
  `query_as!()` / `query_scalar!()`. Runtime `query_as` is acceptable only where a `::vector`
  cast or dynamic column/ORDER BY defeats the macro — the `unified_search` query in
  `search_service.rs` is the established exception and the template for new ones.
- **DRY SQL via views.** When several service functions join the same tables the same way,
  extract a SQL view and query it with simple `WHERE`/`ORDER`/`LIMIT` — don't copy multi-table
  JOINs across functions (they drift, and the planner loses a stable plan).
- **Shared predicate sets.** When list + count + facets need the same filters, extract one
  filter-builder producing the shared conditions and bind values — never duplicate filter logic.
- **Cache regeneration is part of the change.** After touching SQL, regenerate the committed
  `.sqlx/` cache: `cargo sqlx prepare --workspace -- --all-features` for production targets, and
  the per-crate `cargo make prepare-services` / `prepare-e2e` for **test-target** macro queries
  (the workspace ritual skips test targets). A missing cache entry is caught only by offline
  `cargo make check`, not by the live-DB clippy job.

---

## 4. Testing

- **TDD by default** — write the failing test first, then the implementation. For a
  behavior-preserving refactor, the existing suite *is* the safety net: green before, green
  after, no new behavior.
- **Never remove or weaken an assertion to fix a failure.** A failing test is evidence; treat
  it as such (see §1.6 escalation).
- **Pair a filtered single test with a full-crate run before commit.** A green `-E 'test(x)'`
  proves the one path; the crate suite proves you didn't break a sibling. Full-workspace
  nextest belongs at PR-prep, not per task.
- **Feature-gate tests that need a runtime.** Every file with `#[sqlx::test]` needs
  `#![cfg(feature = "test-db")]`; tests calling `compute_body_chunks` / embed paths need
  `#[cfg(feature = "test-embed")]` — otherwise CI's no-DB / no-ONNX jobs fail fast.
- **Test at the production caller's level.** When wiring a hook into an existing path, pair the
  direct-call unit test with an e2e test driving the real caller — a direct-call test passes
  even when the wiring is broken.
- **Name tests for the behavior asserted** —
  `state_transition_from_complete_to_pending_fails`, not `test_transition_2`.
- **Trust the exit code, not nextest's per-binary "Summary" line** under `--no-fail-fast`.

---

## 5. How reviews use this lens

A review (human or `/code-review`) checks, in order:

1. **Correctness** — does it do the right thing? (the floor)
2. **Structure** — §1: single responsibility, function length, keys-not-markers, types encode
   constraints, params structs, error/escalation discipline.
3. **Invariants** — §2 and §3: any violation is blocking regardless of correctness.
4. **Tests** — §4: right level, right gates, assertions intact.

When a review finds a §1 structural smell that is real but out of the PR's scope, **file a
follow-up** rather than expanding the PR — and note it, so it isn't silently dropped. The
`reconcile_apply` decomposition itself was held out of PR #177 this way and became the first
audited item under this lens.

---

## 6. Rule index (audit checklist)

Stable IDs for citing rules in reviews and for the audit harness (each finding cites one). A
violation is "what it looks like in the wild" — the trigger an auditor greps/reads for.

| ID | Rule (§) | A violation looks like |
|----|----------|------------------------|
| **CQ-1** | Single responsibility / length (§1.1) | A function past ~60 lines, or one that reads "phase 1 … phase 2 …", or whose summary needs an "and". |
| **CQ-2** | Keys unique-by-construction, not loose markers (§1.2) | Diffing/dedup/joining on a non-unique marker (e.g. `origin_uri`); identity keys not injected from one typed source on both sides of a boundary. |
| **CQ-3** | Parse, don't validate (§1.3) | Bare `Uuid`/`String` where a newtype belongs; revalidation scattered downstream instead of parse-at-boundary; `&String`/`&Vec<T>` params. |
| **CQ-4** | Names carry responsibility (§1.4) | A *type* named `Manager`/`Helper`/`Service`/`Factory`/`Util`; a stringly-typed `match "literal"` over a bounded set the code owns. |
| **CQ-5** | Params structs (§1.5) | >5 domain params on a fn; `#[expect(clippy::too_many_arguments)]`. |
| **CQ-6** | Error handling & escalation (§1.6) | `.unwrap()`/`.expect()` on a fallible runtime value in a library path; write-then-check (auth after mutation); panic for a recoverable condition; a softened contract/assertion. |
| **CQ-7** | Lint & suppression discipline (§1.7) | Bare `#[allow]` (vs `#[expect(reason=…)]`); a public type without `Debug`; a magic constant with no explaining comment. |
| **CQ-8** | No "for now" / no premature compat/abstraction (§1.8) | Dead code kept "for compat"; a placeholder/"for now" workaround; a one-use abstraction. |
| **CQ-9** | Typed structs over inline JSON (§2) | `serde_json::json!()` for data with a known shape. |
| **CQ-10** | Shared types at boundaries (§2) | A zod schema (or other hand-mirror) duplicating a Rust struct instead of the `ts-rs`-generated type. |
| **CQ-11** | Persistence is its own layer (§2, §3) | Inline `sqlx::query!()` in a handler/MCP tool/CLI action; persistence CRUD interleaved with behavior code. |
| **CQ-12** | Auth before writes / profile scoping (§1.6, §2) | A data query not scoped through `resources_visible_to`/`can_modify_resource`; a mutation before its authorization check. |
| **CQ-13** | SQL discipline (§3) | Runtime `query` where a macro works; multi-table JOINs copy-pasted across fns (→ view); filter predicates duplicated across queries (→ shared builder). |
| **CQ-14** | Testing (§4) | A removed/weakened assertion; a missing `test-db`/`test-embed` gate on `#[sqlx::test]`/embed tests; a direct-call test where a production-caller e2e is needed; a non-descriptive test name. |

This index is **rubric-shaped, not lens-specific**: a later security sweep gets its own `SEC-*`
index in its own doc and runs the same audit harness against it.

---

## References

- tasker-core (sibling project) — `docs/development/best-practices-rust.md`
- [Microsoft Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/) — `M-*` rule IDs
- [rust-skills](https://github.com/leonardomso/rust-skills)
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
