# Code Review: Unified Resource Command Implementation

**Branch:** `jct/cli-standardization-and-documentation`
**Commits:** `fa5aa10..709c79b` (15 commits)
**Files changed:** 46 files, +3412 / -918 lines
**Date:** 2026-04-07
**Reviewer:** superpowers:code-reviewer agent
**Status:** 434/434 tests pass, all checks clean

---

## What Was Done Well

- The overall restructuring is clean and well-decomposed. The single `ResourceAction` enum in `cli.rs` is a significant simplification over the previous 5 separate enums.
- Event consolidation from 7 variants to 3 (`ResourceCreate`, `ResourceUpdate`, `Normalize`) is a good reduction in surface area.
- The schema-driven validation approach in `update()` is the right design -- field legality is derived from the embedded JSON schemas rather than hardcoded.
- Existing tests were updated to match the new event types, research path fix, and doctor cleanup.
- The cleanup work (removing `SyncAutoConfig`, `LEGACY_FIELD_MAP`, `temper-legacy-id`, stale template copies) was thorough.
- Templates for concept and decision match the spec exactly.

---

## Issues

### Critical

- [x] **C1. `SessionAction` is dead code in `cli.rs`**
  `SessionAction` (lines 314-351 of `cli.rs`) is defined but never used. Should be removed.

- [ ] **C2. `temper-legacy-id` missing from `SYSTEM_MANAGED_FIELDS` blocklist**
  The spec (line 128-129) says to keep it in the blocklist for `resource update` so old files don't get accidentally modified. Currently omitted from `SYSTEM_MANAGED_FIELDS` in `schema.rs`. Since the field is also removed from `KNOWN_TEMPER_FIELDS` and the base schema, the omission is arguably safe but deviates from spec.

### Important

- [x] **I1. `update()` has 20 parameters -- needs a parameter struct**
  The `update()` function takes 20 arguments with `#[allow(clippy::too_many_arguments)]`. An `UpdateParams` struct would improve readability and extensibility.

- [x] **I2. `--extends`, `--preceded-by`, `--derived-from` specified but not implemented**
  The spec (lines 137-139) lists these as base schema fields updatable on all types. Absent from both clap definition and `update()`. Either implement or update spec.

- [x] **I3. Array field names use underscores instead of hyphens (BUG)**
  In `resource.rs` lines 596-601, array updates use `relates_to` and `depends_on` instead of `relates-to` and `depends-on`. The YAML frontmatter uses hyphens. `append_frontmatter_array` will insert a new `relates_to:` key instead of appending to existing `relates-to:`.

- [ ] **I4. `fields_renamed` still tracked in doctor_fix despite `LEGACY_FIELD_MAP` removal**
  `FixReport` still has `fields_renamed` counter and `RenameField` actions are still emitted. The rename infrastructure serves other purposes beyond legacy fields, so this may be intentional. Verify.

### Suggestions

- [x] **S1. `append_frontmatter_array` should live in `vault.rs`**
  It's a self-contained YAML manipulation function that belongs alongside `set_frontmatter_field`, `parse_frontmatter`, and `replace_body`.

- [x] **S2. No unit tests for new functions in resource.rs**
  These functions have no test coverage:
  - `validate_doc_type()` -- trivial, low priority
  - `extract_date_prefix()` -- pure function, easy to test
  - `find_resource_file()` -- complex matching logic, high value
  - `append_frontmatter_array()` -- complex string manipulation, **highest priority**
  - `create_simple_resource()` -- integration-level, lower priority
  - `update()` schema validation -- high value but needs test fixtures

- [ ] **S3. Stdin read location inconsistency**
  Concept/decision reads stdin inside `create_simple_resource()`, while session/research read stdin in `create()` before delegating. Minor maintainability concern.

- [ ] **S4. `list()` requires context for goals but not tasks**
  Goal list calls `require_context()` while task list uses optional context. Spec says `--context` is optional for all types. Matches pre-existing behavior but deviates from spec.

- [ ] **S5. `find_resource_file` matching is overly broad**
  The `stem.contains(&needle)` match means "fix" would match "prefix-fix-suffix", "fixing-bugs", etc. Consider tightening to word boundaries or removing in favor of exact and date-prefix matches only.

- [x] **S6. Concept slug incorrectly gets date prefix (BUG)**
  In `create_simple_resource()`, the slug always gets `format!("{today}-{}", vault::slugify(title))`, but the spec says concept paths are `{context}/concept/{slug}.md` with "no date prefix -- concepts are identified by name" (spec line 209).

---

## Spec Adherence

| Spec Requirement | Status |
|---|---|
| Replace 5 per-doctype enums with ResourceAction | Done |
| resource create with type dispatch | Done |
| resource list with type-specific filters | Done |
| resource show with slug matching | Done |
| resource update with schema validation | Done |
| `--extends`, `--preceded-by`, `--derived-from` flags | Missing (I2) |
| `--context-to` for moving resources | Done |
| `--type-to` / `--type-from` for type conversion | Done |
| Array field append behavior | Done (but key naming bug I3) |
| Concept template + schema date field | Done |
| Decision template | Done |
| Remove SyncAutoConfig | Done |
| Remove LEGACY_FIELD_MAP | Done |
| Remove stale template copies | Done |
| Remove temper-legacy-id from schemas/types | Done |
| Keep temper-legacy-id in SYSTEM_MANAGED_FIELDS | Not done (C2) |
| Fix research path bug | Done |
| Remove note.rs module | Done |
| Event consolidation (7 to 3) | Done |
| Skill/docs updates | Done |
| Concept path: no date prefix | Incorrect (S6) |
