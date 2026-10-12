# The erasure policy

This directory declares what an erasure reaches. When a person or a resource is erased, these files
say which tables and columns hold what was erased, what the erasure does to each one, and what it
deliberately leaves alone and why.

The five `.toml` files are **one manifest**. One parser reads them as a unit, and the deployed
server carries them. Each file opens with a header comment giving its full format and vocabulary.
This README is the map across the five. Read it first, then the header of the file you are editing.

## The five files

| File | Declares | At what grain |
|------|----------|---------------|
| `manifest.toml` | The manifest's `format`. A parser refuses any format it was not written for. | — |
| `estate.toml` | What an erasure holds and how it reaches each table's rows. `scope` lists the rules that put containers in an estate. `[tables]` gives every content or identity table exactly one **grain**: `per-item`, `covered`, `identity` or `declared-out`. | table |
| `personal-data.toml` | For each column a structural derivation nominates: the **class** of personal data it holds, the **ceiling** on how far erasure could reach it, and the **achieved** reach of the person act. | column |
| `sensitivity-scan.toml` | Every `text`/`varchar` column that `personal-data.toml` does *not* declare, each with a **disposition**: `scan`, `structural`, `credential` or `out-of-scope`. | column |
| `resource-erasure.toml` | What the resource-erasure act does with every **carrier** column a resource reaches (`[tables]`), and with every free-text path in that resource's ledger payloads (`[payload]`). | column, payload path |

## How the files fit together

- **`personal-data.toml` and `sensitivity-scan.toml` partition the text columns.** Every
  `text`/`varchar` column of a public base table is declared in exactly one of the two, so no
  column falls in neither.
- **`resource-erasure.toml` defines nothing itself.** A carrier is a column that
  `sensitivity-scan.toml` marks `scan`, or that `personal-data.toml` classes `content`,
  `incidental` or `derived`. This file only says what the act does about those columns.
- **`estate.toml` takes its table set from the other two.** A content or identity table is a
  table with a column that is classed `content`, `incidental`, `derived` or `identifier`, or
  marked `scan`. Each such table needs a grain. The set is computed from those entries, never
  kept as a list.

## Derived candidates, declared judgements

No query can tell whether a `text` column holds a person's name, and a hand-kept inventory rots
the day someone adds a column. So the work is split:

- **The tests derive.** Each fence test reads the live catalog of a fully migrated database:
  foreign keys, polymorphic `*_table`/`*_id` pairs, column types, and the bodies of the erasure
  functions. It cannot miss a column, because it asks Postgres rather than a person.
- **These files declare.** For each candidate, they record the judgement that a query cannot
  make.
- **Each test fails in both directions.** It fails on a live candidate with no entry here, and
  on an entry here that names something no longer live. Neither half can drift without a failure.

| Fence (in `crates/temper-services/tests/`) | Holds |
|--------------------------------------------|-------|
| `personal_data_surface_test.rs` | Every derived candidate has a `personal-data.toml` entry. Every `achieved` claim is bound to the live person act. |
| `sensitivity_scan_surface_test.rs` | The two column manifests partition the text columns, and `declared_count` is right. |
| `resource_erasure_surface_test.rs` | Every carrier a resource reaches has a line. Each `handled`/`repointed` line is bound to a statement in the live resource act. Every string path of a registered payload schema has a `[payload]` line. A type with no schema is declared `permissive` and its paths are listed by hand, so this fence cannot check that list is complete. |
| `erasure_policy_catalog_test.rs` | Every content or identity table has exactly one grain. Every named table, column and bound function exists. An `fk` link is a foreign key and a `ref` link is not. A `cited_by` value is in its discriminator's vocabulary. Every covered chain ends in a unit. Every `scan` column of an identity table is reached by the person act or recorded as a column-scoped gap. |
| `erasure_policy_parse_test.rs` | The shipped manifest parses, and corrupt input is refused, with the failing file named. |
| `sensitivity_store_test.rs` | The sweep's surface registry covers both manifests: each `scan` column as a `text` surface, and each `incidental` column of `personal-data.toml` as a `jsonb` surface. A surface that neither manifest names any longer is disabled. |

All but `erasure_policy_parse_test.rs` need a database. Run them with `cargo make test-db`, or
run a single target with
`cargo nextest run -p temper-services --features test-db --test <target>`.

## Bound mechanisms and gaps

Wherever these files name a mechanism, they say whether it exists or is uncovered work:

```toml
mechanism = { bound = "_erasure_apply_redaction" }   # a live function or flag column
mechanism = { gap = { task = "01a12dcf-2ef5-729d-b6ef-051fdee3ef8c", what = "the personal team's description" } }
```

A gap names the task that tracks it, as a full UUID, and states what no mechanism reaches. The
parser refuses a gap without both. The gap and its task are the whole record of uncovered work,
so notes do not restate it.

Each file's header also says **what its check reads**: columns rather than rows, statements rather
than control flow, references with neither a foreign key nor a CHECKed discriminator, and schemas
other than `public`. Read that section before you treat a green test as coverage.

## The vocabulary at a glance

- **Grains** (`estate.toml`): `per-item` (fence, unit, home), `covered` (a parent path, and an
  optional `unit` of its own), `identity` (the person act's mechanism, and an optional
  `retention` category), `declared-out` (a reason, and an optional `retention` category).
- **Paths** (`estate.toml`): a `link` is `fk` (a column under a foreign-key constraint), `ref` (a
  uuid column with no constraint) or `pair` (a CHECKed `*_table` / `*_id` pair). A path read in
  reverse carries `cited_by = { column, value }`: the discriminator on the citing table, and the
  value that marks a citation.
- **Achieved claims** (`personal-data.toml`): `erased:<arm>`, `deleted:<arm>`,
  `cascaded:<arm>:<table>`, `resource-erasure`, `remainder: <reason>`, or `-`. An arm is a section
  of the person act's body, numbered as its comments number it.
- **Dispositions** (`resource-erasure.toml` `[tables]`): `handled`, `repointed:<table>.<column>`,
  `out-of-scope`. Under `[payload]`: `redact:<class>`, `structural`, `out-of-scope`, and
  `permissive` for the `*` line of a type with no schema.
- **Notes** are trimmed as they are read, so a note of only whitespace is empty, and a line that
  requires a reason fails without one.

## How the manifest is read

- **Parser:** `temper_core::erasure_policy::parse` (in `crates/temper-core/src/erasure_policy/`)
  turns the five texts into one typed `ErasurePolicy`. It lives in temper-core so that admin
  tooling and the CLI can express the policy's shape. Every table in it refuses unknown keys. A
  parse error names the file that failed.
- **The policy in force:** `temper_services::erasure_policy::in_force()` embeds the five files
  with `include_str!` and parses them once. The policy that governs an erasure is the deployed
  server's, so a client learns it through a door, never from a copy compiled into the client.
- This README is not part of the manifest, and nothing parses it.

## Changing the policy

Most changes start with a migration. Add the entries in the same PR that adds what they
describe. The fences fail until you do.

- **A new column.** If a personal-data derivation nominates it, it goes in
  `personal-data.toml`. If it is a `text`/`varchar` column nothing nominates, it goes in
  `sensitivity-scan.toml`, and `declared_count` and `count_at` change with it. If the column is a
  carrier in a table a resource reaches, it also needs a line in `resource-erasure.toml`.
- **A new table with content or identity columns.** It needs a grain in `estate.toml`. Name its
  path to a parent, or its own unit, or say why it is declared out.
- **A new event type or payload field.** Declare its free-text paths under
  `[payload."<type>"]` in `resource-erasure.toml`.
- **A new erasure function.** Change the `{ gap = … }` entries it fills to `{ bound = … }`, and
  close their tasks.

Any change to this directory is gated three ways:

- **Release register.** `wire-class-crosscheck.sh` requires the branch to add a
  `release-register/unreleased/` entry naming the `erasure-policy` surface.
- **CI scope.** `detect-ci-scope.sh` treats `policy/erasure/` as Rust-coupled, so the Rust
  pipeline runs, fences included.
- **Deploy.** `vercel-ignore-build.sh` deploys on a `policy/` change, because the server embeds
  these files.

A change to the TOML format itself bumps `format` in `manifest.toml` and the parser together.
