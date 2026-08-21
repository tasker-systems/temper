# Beat A — data artifacts: schema and write path

**Task:** `01a02163-faba-7a71-b09a-45eade04baba` · **Goal:** `01a02163-ba6a-7b00-91f5-5f416e43f4f6`
**Spec:** `internal/superpowers/specs/2026-08-20-resource-owned-data-artifacts-design.md`
(vault: `01a02163-8670-7cc2-96a6-1a520ec8a0f8`)

This is an **index over the spec, not a summary of it.** Read the spec's *Model* and *Replay*
sections before implementing — they are not restated here. Every step carries a
CONFORM / EXTEND / AMEND tag. No invented code bodies: where a shape must be followed, the step
cites the `file:line` to read and copy from.

## Beat sequence (Beat A only; B–D are later sessions)

- **Beat A** — schema + write path (this plan)
- **Beat B** — Rust substrate: typed payload, `EventKind`, `fire()`, schema snapshot, replay wiring
- **Beat C** — service layer: read path, visibility gating, shape-state reporting
- **Beat D** — surfaces: API, CLI, MCP, self-describing `describe` verb

## Grounding evidence (executed 2026-08-20, local Postgres :5437)

**Wrapper shape** — `\sf property_set`, live:

```sql
SELECT a.anchor_table, a.anchor_id INTO v_anchor_tbl, v_anchor
  FROM _property_owner_anchor(v_owner_tbl, v_owner) a;
v_ev := _event_append('property_set', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                      p_metadata => p_metadata, p_invocation => p_invocation,
                      p_correlation => p_correlation);
RETURN _project_property_set(v_ev, p_payload);
```

**Event type must be seeded** — `\sf _event_append`, live:

```sql
SELECT id, category INTO v_et, v_cat FROM kb_event_types WHERE name = p_type_name;
IF v_et IS NULL THEN RAISE EXCEPTION 'event_type % not seeded', p_type_name; END IF;
```

**Category vocabulary** — live constraint:

```
kb_event_types_category_check|CHECK ((category = ANY (ARRAY['domain','admin','system'])))
```

**Identity-as-input** — `_project_property_set` takes its row id from the payload
(`v_prop uuid := (p_payload->>'property_id')::uuid`), which is why ids reproduce under replay.

**Content-split precedent** — `migrations/20260714000002_block_content_verbatim.sql:32-37`.

**Search-vector call site to NOT copy** — the tail of `_project_property_set`:
`IF v_owner_tbl = 'kb_resources' AND v_key IN ('keywords','descriptor','tags') THEN PERFORM
_rebuild_resource_search_vector(v_owner); END IF;`

**Migration headroom** — `origin/main` and local both top out at
`20260820000010_survey_honors_funnel_width.sql`. Beat A takes `20260820000020`.

## Steps

### A1 — `kb_data_artifacts` and `kb_data_artifact_content`

**EXTEND** (new affordance; spec *Model*) with a **CONFORM** core: the metadata/bytes split follows
`kb_block_content` at `migrations/20260714000002_block_content_verbatim.sql:32-37`. Read that DDL and
mirror its shape and the meaning of its `content_hash` comment ("bare sha256 hex of content's raw
bytes (Rust `sha256_hex` twin)").

Metadata row must be **entirely payload-derivable** so it can join `PROJECTION_DUMPS` and byte-diff
(spec *Replay*). Content bytes ride the sidecar and are hash-proved.

Assert/fold columns follow the incumbent trio used by `kb_properties` /`kb_edges`:
`asserted_by_event_id`, `last_event_id`, `is_folded`.

### A2 — deliberately NO uniqueness index over (resource, family)

**CONFORM** to the clause `no-supersession-is-asserted-that-a-writer-did-not-declare`.

This is the single place the design departs from `uq_kb_properties_active`. The absence **must carry
a comment stating it is deliberate and why** — an uncommented missing index reads as an oversight and
the next reader will "fix" it. Quote the reason: the store cannot know whether run #2 supersedes run
#1; only the writer can.

### A3 — register the event type

**CONFORM.** `_event_append` hard-fails on an unseeded type (evidence above), and category must be
one of the three admitted values. `domain` is the correct category — this is cognition, not admin or
system infra. Follow how sibling domain event types are registered; do not invent a registration
path.

### A4 — anchor resolver arm for the new owner kind

**CONFORM** to `_property_owner_anchor`'s `kb_resources` arm, printed live. Reuse its
`ORDER BY (h.anchor_table = 'kb_cogmaps') DESC` tiebreak rather than re-deriving it — that ordering
is load-bearing (a resource homed in both anchors on the cogmap).

Per the discipline in `migrations/20260727000030_edge_owned_properties.sql`: owner kinds with no
caller **RAISE rather than get a speculative branch** — *"an unused arm is state we do not need, and
a silent NULL anchor would be worse than a clear refusal."*

### A5 — wrapper + projector

**CONFORM** to `property_set` → `_project_property_set`, both printed live above. Same four moves:
validate, resolve anchor, `_event_append`, `_project_*`. Identity payload-carried.

Revision is **fold-and-reassert** — there is no mutable `revised` column, and no `UPDATE` of a live
row's content. A revision folds the prior row and inserts a new one; the folded chain *is* the
revision history, and `revised` is inferred from it on read. Decided with the frame owner
2026-08-20.

### A6 — the projector must NOT touch the search vector

**CONFORM** to the negative clause `structured-data-is-never-found-by-resemblance`.

`_project_property_set` ends by rebuilding the resource search vector for three keys. The artifact
projector must not, and the omission **needs a comment naming the clause** — otherwise it reads as
something nobody got round to, and gets added later as a helpful improvement.

### A7 — CHECK constraint on selection intent

**EXTEND** (spec *Model*, the three-term closed vocabulary). Refusal happens at the storage layer as
well as the edge, so a path that bypasses the service layer cannot write an unrecognized term.

## Verification for Beat A

Witnesses are authored **here, inside the build** — not decomposed from the register in advance. Each
must **fail against the state its clause claims to change**, which is only honest now that the
mechanism exists.

- Write path round-trip: commit an artifact, retrieve it, bytes identical.
- Anchor resolution: an artifact on a context-homed resource and on a cogmap-homed resource both
  resolve; the cogmap tiebreak is exercised, not assumed.
- **Negative-face bite:** after committing an artifact, assert that no `kb_chunks`,
  no chunk embedding and no `kb_resource_search_index` change appears. This must be shown to FAIL if
  the search-vector call from A6 is (re-)introduced — that is the bite, and without demonstrating the
  failure the test proves only that nothing was observed.
- Fold-and-reassert: a second commit folds the prior row, both rows survive, and the live set is what
  the writer declared — not what an index decided.
- Intent CHECK refuses a term outside the vocabulary.

Tests use `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]` per the `artifact-tests` feature
(see `internal/agents/conventions.md`). Run with `cargo make test-artifacts`.

## Known traps for this beat

- `migrations/` changes do **not** trigger a rebuild of the `sqlx::migrate!` macro — a stale set runs
  silently. Touch `lib.rs` after adding the migration.
- Editing an **applied** migration (even a comment) trips the sqlx checksum. While unmerged, the fix
  is resetting the DB volume, not renumbering.
- New SQL means regenerating the `.sqlx` cache; `prepare-api --all-targets` writes untracked entries
  that must not be staged.
