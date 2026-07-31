# Classifying the non-macro `sqlx` call sites

> **`[corrected — 2026-07-30, during Arc A]` Every count below was produced by an enumerator that
> could not see 10 of the call sites it was counting.** `scripts/classify-sqlx-calls.py` matched a
> turbofish with `(?:::<[^>]*>)?`, and a character class cannot cross a nested generic — so every
> `sqlx::query_as::<_, (Uuid, Option<String>, i32)>(…)` was invisible to it. Ten production runtime
> sites (8 in `graph_service.rs`, 2 in `context_graph_service.rs`) were therefore never classified
> at all. **The true starting total was 112, not 102, and the convertible set was 66, not 56.** The
> 46 legitimate exceptions are unaffected — none of the hidden sites is one, and the allow-list seed
> is still 46. The script is fixed; the tables below are annotated where they were wrong.
>
> This mattered more than an off-by-ten. The instruction governing Arc A is *"if the count is not 46,
> A1/A2 are not done"* — and with the 56 known sites converted, the **old** script reported exactly
> 46. The done-number was reachable while ten unconverted sites sat behind the blind spot: a check
> that would have gone green for the wrong reason, on the very task whose subject is mechanisms that
> read as coverage over a corpus they cannot see.

`[observed — 2026-07-30]` Input to step 3 of
[the schema/binary pairing design](../superpowers/specs/2026-07-30-schema-binary-pairing-design.md#6-the-macro-is-the-rule-exceptions-are-an-allow-list-not-a-habit),
which builds the allow-list this classification decides the contents of. It builds nothing itself.

**Why this matters.** The `.sqlx` cache is a faithful record of the binary↔schema wire contract only
for `query!`-family macros. A runtime `sqlx::query(...)` leaves no cache entry, so it is invisible to
the change detector — and *a mechanism that is invisible exactly where it matters is worse than one
that is absent, because it reads as coverage.*

**Reproduce every count here** with `python3 scripts/classify-sqlx-calls.py`. Nothing below is
asserted from reading.

## Headline: most exemptions have no technical reason

| class | count | verdict |
|---|---:|---|
| `dynamic-table` — `replay.rs` dump/restore | 36 | **legitimate**, self-documented |
| no technical reason visible | **56** → **66** | **convert to macros** |
| *…of which invisible to the enumerator* | *10* | *see the correction above* |
| `vector-cast` — a `$n::vector` bind | 7 | **legitimate** |
| `dynamic-sql` — statement assembled at runtime | 3 | **legitimate** |
| **total non-macro, production code paths** | ~~102~~ **112** | |

**66 of 112 have no reason.** That is the finding. The spec anticipated it — *"Any call site that
turns out to have no reason should become a macro rather than an allow-list entry — that is the point
of doing this at all"* — but not the scale: the exemptions are mostly habit, and an allow-list built
by transcribing the current state would enshrine that habit as policy.

The 10 late-found sites are the same story in a harsher key. They are `query_as::<_, (…)>` reads over
set-returning graph functions, and they convert: the `edge_kind[]` **array bind** that looked like a
real obstacle is fine (`&req.edge_kinds as &[EdgeKind]`, since `EdgeKind` carries
`#[sqlx(type_name = "edge_kind")]` at `crates/temper-core/src/types/graph.rs:30-31`), and SQL enum
columns decode natively via `AS "edge_kind!: EdgeKind"` with no `::text` round-trip. Their only real
cost is that `query!` yields a named record rather than a tuple, so each consumer's destructuring
changes.

## The single biggest cluster is exempt "for consistency"

`temper-substrate/src/readback/mod.rs` holds 19 non-macro calls and states its own reason at
`mod.rs:16-18`:

> *"Most reads are runtime `sqlx::query` (the pgvector `::vector` cast forces runtime; **the rest
> follow for consistency**)."*

**Three** of those 19 carry `::vector`. The other 16 are exempt because their neighbours are. That is
a style convention, not an obstacle — exactly the decay the spec's "a reason turns an exception into a
declaration" clause exists to prevent.

**Two stale claims in that same header, worth fixing when the module is touched:**

- It calls itself *"read-only parity tooling"* whose purpose is assertion against production reads.
  It is now **a production dependency**: `temper-services/src/services/citation_audit_service.rs:129`
  and `:136` call `readback::is_resource_visible` and `readback::citation_audit_trail`, and
  `evidential_standing_service.rs:38` calls `readback::resource_standing`.
- It says *"the pgvector `::vector` cast forces runtime"* as though it covered most of the module. It
  covers 3 of 19.

## Corrections to the spec's measured table

Three of its rows do not survive re-measurement. The non-macro figure — the one the work depends
on — is exactly right.

| spec row | spec | actual | note |
|---|---:|---:|---|
| non-macro, production code paths | 102 | ~~102~~ **112** | ✋ **this row was the confident one, and it was wrong** — see the correction at the top. Spec and classification agreed at 102 because both read the same blind enumerator; agreement between two consumers of one broken instrument is not corroboration. |
| non-macro inside test modules | 217 | **217** | ✅ exact *as measured* — but measured by the same script, so it carries the same blind spot and is a floor, not a count |
| macro calls, production source | 435 | **311** | counts test-module macros as production; 311 + 119 = 430 total |
| `embed.rs` — "6, confirmed `::vector`" | 6 | **2** | only `:41` and `:224` cast; `:159`/`:253` are dynamic `&sql`; **`:16` and `:184` are plain static literals** |
| residual to classify | ~60 | **60** | ✅ |

The `embed.rs` row is the one that matters: it was recorded as **confirmed**, and the confirmation
does not hold. Four of its six calls are something other than what the table says, two of them
convertible.

## The legitimate exceptions, enumerated with their reasons

### `dynamic-table` (36) — `temper-substrate/src/replay.rs`

Self-documented at `replay.rs:13-14`: *"Dumps/restores are dynamic-table operations, so this module
uses runtime `sqlx::query` (the established exception class) rather than compile-checked macros."*
The table name is the loop variable; no macro can express it. Accept as a **class**, not 36 entries.

### `vector-cast` (7) — a `$n::vector` bind the macro cannot type

| site | statement |
|---|---|
| `temper-substrate/src/embed.rs:41` | `UPDATE kb_chunks SET embedding = $1::vector WHERE id = $2` |
| `temper-substrate/src/embed.rs:224` | `UPDATE kb_chunks SET embedding = $1::vector, embedded_with = $2 …` |
| `temper-substrate/src/write.rs:686` | `::vector` bind |
| `temper-substrate/src/write.rs:988` | `coalesce(…, $2::vector)` centroid update |
| `temper-substrate/src/readback/mod.rs:868` | `::vector` |
| `temper-substrate/src/readback/mod.rs:1383` | `::vector` |
| `temper-substrate/src/readback/mod.rs:1434` | `::vector` |

### `dynamic-sql` (3) — the statement text is assembled before the call

| site | what varies |
|---|---|
| `temper-services/src/backend/substrate_read.rs:265` | `ORDER BY {sort_col} {dir}` — the documented dynamic-ORDER-BY case |
| `temper-substrate/src/embed.rs:159` | `sqlx::query(&sql)` |
| `temper-substrate/src/embed.rs:253` | `sqlx::query_scalar(&sql)` |

## The exemptions that were asserted rather than reasoned

Four call sites carried comments declaring their own exemption. None of the four claims survived
contact, and one had spread.

| site | the claim it made | what is actually true |
|---|---|---|
| `readback/mod.rs:17` | *"the pgvector `::vector` cast forces runtime; **the rest follow for consistency**"* | 3 of 19 cast. The other 16 were exempt because their neighbours were. |
| `admin_ledger_service.rs` | *"the dynamic-predicate case the `search_service` precedent covers"* | Its own next sentence conceded *"nothing is interpolated"*. `($2::jsonb IS NULL OR …)` is a static statement with NULL-passthrough params. |
| `region_clocks.rs` | reads as `dynamic-table` — the table differs by anchor | It already selects between two **static literals**. A `query_scalar!` per match arm puts both in the cache. |
| `scenario/runner.rs` | *"the live-edge resolution + ambiguity guard is dynamic intent"* | The statement is a static literal. Its only friction was a `$3::edge_kind` enum bind, which `readback::find_edge` had already solved by casting the column instead. |

**And one was false in a way that propagated.** `edge_service::list_resource_edges` claimed
`resources_visible_to` / `edges_visible_to` cannot appear in a macro because *"sqlx's compile-time
describe inlines the SQL-function bodies at plan time"*. Describe does no such thing, and this crate
already disproved it — `team_service.rs:673` calls `resources_visible_to` from a `query_scalar!`.
`lineage_service`'s module doc then adopted the claim **by citation** ("matching `edge_service`"),
which is how two further sites became runtime on the strength of a false statement nobody re-checked.

This is the shape the allow-list exists to stop. A written reason is not a reason; it is a claim,
and an unexamined one propagates faster than the practice it justifies.

## The enumerator was blind, and the blindness pointed at the done-number

Recorded here because the allow-list check is specified to shell out to this script, so its coverage
*is* the check's coverage.

`scripts/classify-sqlx-calls.py` matched an optional turbofish with `(?:::<[^>]*>)?`. Angle brackets
nest; a character class cannot cross them. In
`sqlx::query_as::<_, (Uuid, Option<String>, i32)>(`, `[^>]*` halts at the `>` closing
`Option<String>`, the optional group backtracks away, and the required `\s*\(` meets a `:`. The call
site vanishes — silently, with no partial match to notice.

It hid 10 production sites, every one a tuple-projecting `query_as::<_, (…)>`. What makes it worth a
section rather than a bugfix note is *where* the blindness landed: with the 56 known sites converted,
the old script reported exactly **46** — the number Arc A's own instruction uses as its
done-condition. The gate would have gone green while ten unconverted sites hid behind the instrument
that was supposed to be watching them.

The fix is a two-stage scan: a regex for the path and function name, then a forward walk that skips
an optional turbofish by counting `<`/`>` depth before requiring the open paren.

## One exemption that looks legitimate and is not

`temper-services/src/backend/region_clocks.rs:139` reads as `dynamic-table` — the table differs by
anchor. But it **already avoids interpolation**, selecting between two string literals:

```rust
let sql = match anchor {
    HomeAnchor::Context(_) => "SELECT shape_materialized_event_id FROM kb_contexts WHERE id = $1",
    HomeAnchor::Cogmap(_)  => "SELECT shape_materialized_event_id FROM kb_cogmaps  WHERE id = $1",
};
```

Two `query_scalar!` calls in the match arms compile, and both land in the `.sqlx` cache. The
enum is closed, so this is exhaustive either way. **Convertible.**

This is the shape to watch for while building the allow-list: *"a parameter cannot be a table name"*
is true and does not imply *"therefore runtime"*, when the set of tables is closed and small.

## How far "convertible" has actually been established

**Honestly: as a triage plus a spot check, not as 56 proofs.**

The 56 are those whose call argument is a static string literal with no `::vector` and no runtime
assembly. Three representative shapes were converted to macros and compiled against the live dev
database — `cargo check -p temper-substrate`, exit 0:

- a plain scalar — `SELECT id FROM kb_profiles WHERE id = $1`
- a **SQL-function call** — `SELECT cogmap_readable_by_profile($1, $2)`, the shape most likely to
  defeat inference; it types as `Option<bool>` (functions are nullable to sqlx) and compiles
- a **multi-column join through a set-returning function** — `resources_visible_to($1)`

So the shapes that dominate are demonstrably convertible. Each remaining site still needs its
own compile, and **two costs should be planned for rather than discovered**:

1. **Conversion is not a one-line swap where the untyped `Row` API is used.** Many of these read
   columns via `row.get("origin_uri")`. A macro returns an anonymous struct with typed fields, so the
   call site changes too, and nullability that `Row::get` papered over becomes explicit.
2. **Every converted site adds a `.sqlx` cache entry**, which is the *point* — that is what makes it
   visible to the change detector — but it also restales the cache and demands the regeneration
   ritual. See the `sqlx-query-cache` skill.

## What this implies for step 3

- The allow-list should carry **4 reasons**, not 3: `dynamic-table`, `vector-cast`, `dynamic-sql`,
  and `dynamic-order-by` (or fold the last into `dynamic-sql` — `substrate_read.rs:265` is its only
  member, and it is the case the existing prose rule at
  `docs/development/code-quality-best-practices.md:165` already names).
- It should be seeded at **46 entries** (36 + 7 + 3), not by transcribing the pre-Arc-A state.
  Seeding from that state would bless 66 sites that have no reason, and a baseline that blesses the
  thing it exists to prevent is worse than no baseline. **46 survives the correction unchanged** —
  none of the 10 late-found sites is a legitimate exception — but reaching it took 66 conversions,
  not 56.
- **Conversion and enforcement are separable and should be sequenced that way.** The check can land
  against a 46-entry allow-list only once the 56 are converted; landing it earlier means either a
  bloated baseline or a red gate. Converting first, in file-sized batches, keeps every step green.
- `readback/mod.rs` is the natural first batch: 16 of the 56 are there, they share one stale
  rationale, and fixing that header is part of the same edit.
