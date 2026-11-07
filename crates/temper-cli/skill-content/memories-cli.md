# Memories

Read this when saving, correcting or reinforcing a durable memory, moving local memory files into
Temper, or touching this machine's `MEMORY.md`. Flag detail: `temper memory <cmd> --help`.

## The model

- **A memory is a resource of type `memory`.** Author and correct it like any resource, in the
  context whose readers it is for.
- **`open_meta.status` is `active` or `superseded`.** Read the active ones and honour them.
- **`open_meta.verified` is the date the claim was last checked. `verified` is a judgment, not a
  timestamp:** advance it only after re-checking the claim against the system it describes; moving,
  re-rendering, re-reading or amending the body never advances it. An old date means nobody
  re-checked, not that it is false.
- **Correct by supersession.** Set the old account's `status` to `superseded` and leave it readable;
  it stays addressable and searchable.
- **A memory carries no reach of its own.** Its audience is whoever reads the context it lives in.
- **The store is the record; `MEMORY.md` is a projection.** `temper memory emit` renders it. Do not
  hand-edit it.
- **`temper memory status` works before you opt in.** It reports opt-in, the contexts read, store
  and local counts, and which local files have no counterpart in the store. Never adopting is a
  supported end state.

## Populating the store: `harvest`, then `migrate`

1. **`temper memory harvest` first.** A memory's title exists only as link text in the hand-written
   `MEMORY.md`; harvest copies each title into the file it names. Skip it and an `emit` takeover
   destroys the titles, and `migrate` then skips every file no link names. It is idempotent.
2. **Check for near-duplicates yourself** (below). `migrate` does not.
3. **`temper memory migrate --dry-run`, then the real run**, one cohort per run, keyed on the files'
   frontmatter `type` (`--cohort`, default `feedback`).

`migrate` **reconciles rather than bulk-imports**:

- It matches on `open_meta.source_file` and skips files already in the store. That is the whole
  reconciliation, and it is why an interrupted run resumes instead of duplicating.
  It **detects nothing about near-duplicates**.
- It is interactive by default: one confirmation for the whole batch, defaulting to no. With no
  terminal it refuses to write unless `--unattended` authorizes the run; that flag skips no check,
  because there is none.
- `feedback` lands in the first `shared_contexts` entry; every other cohort in the first
  `project_contexts` entry; `--context` overrides. **An empty list is a refusal, never a fallback
  to the other list**: reach comes from which config list names the context.
- Every skipped file is reported with its reason.

### Near-duplicates are for a human to judge

Read the local memories against what the target context already holds.
**Compare the claim, not the wording**: two memories about one subsystem are not an overlap; two
asserting the same thing are. Shared vocabulary misleads, and a shared context has the most of it.
Expect three outcomes:

- **Duplicate**: one incident recorded twice, often from two machines. Keep the older, richer one.
- **Supersession**: a newer account strictly richer than the one it covers.
- **Both stale**: both out of date, the newer one still wrong. Recency arbitrates nothing.

Bring a short list a human can adjudicate. **Never merge automatically.** This is guidance, not a
gate; nothing stops you skipping it.

## Recording that a memory was load-bearing

```bash
temper resource update <ref> --open-meta-add '{"reinforced":["2026-08-02"]}'
```

`open_meta.reinforced` is a list of bare ISO dates: the days the memory did work. Two things count:
the situation it describes recurred, or **it caught you** before the mistake. Count the catch,
because a memory that works prevents its own situation from recurring.

If the catch exposed a case the body does not describe, **amend the body**; a metadata note about
the gap is not the fix. Amending the body does not advance `verified`.

## Rendering and gating the index

- `temper memory emit` renders the index. **It refuses to overwrite a file it did not write**, so
  move a hand-written `MEMORY.md` aside deliberately, after `harvest`.
- `emit` refuses the whole index if any memory's `status` or `verified` is malformed.
- `temper memory check` exits non-zero when the on-disk index has drifted. The index lives outside
  any repo, so `check` is the gate a person or hook runs.
- `reinforced_min` in `[memory]` has no default and is unset, so today reinforcing changes what
  `status` reports and nothing in the file. Once set, memories below it collapse into a per-section
  tail line whose command lists them: **demoted, never deleted.**

## Traps

- **`--open-meta` instead of `--open-meta-add`**: replaces the `reinforced` list and silently
  discards every stored date, with a success response.
- **Adding a `by` or note field to a reinforcement**: the `property_set` event already records who,
  and a note makes every entry unique, so same-day duplicates stop collapsing.
- **Reaching for a similarity score or auto-merge above a threshold**: both rejected; scores flag
  shared vocabulary, and auto-merge is what the rule forbids.
- **A malformed `reinforced` is soft**: `status` lists it under `reinforcement.malformed` and the
  index renders it as unreinforced, so check `status` when a memory goes missing from the index.
- **A memory you remember but cannot see in the index**: look for it on the tail line.
