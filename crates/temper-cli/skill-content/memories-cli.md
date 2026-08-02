# Memories

> **Memories are resources.** Durable working knowledge is stored as resources of type `memory`,
> carrying `open_meta.status` (`active` / `superseded`) and `open_meta.verified` — the date the
> claim was last checked. A `verified` date far in the past means **nobody has re-checked it**, not
> that it is false.

This machine's `MEMORY.md` is a **rendered projection** of those resources, written by
`temper memory emit`. Do not hand-edit it: the record is the store, the file is a cache.

## Start with `status` — it works before you have adopted anything

```bash
temper memory status
```

Reports what this machine carries whether or not a `[memory]` section exists in `config.toml`:
whether it has opted in, which contexts it reads, how many memories are in Temper, how many local
memory files sit beside the index, and which of those have **no counterpart** in the store.
Declining is a supported end state — a machine can measure its own divergence and never adopt.

## Populating the store — `harvest`, then `migrate`

A machine still holding local memory files has not populated anything yet. Two commands do that,
and **the order is load-bearing**.

**`temper memory harvest` runs first.** A memory's human-readable title exists in exactly one
place: the link text in the hand-written `MEMORY.md`. `harvest` copies each curated title into the
file it names. Skip it, and letting `emit` take the index over destroys those titles — and with
them `migrate`'s ability to move the remaining files at all, since a file no link names is skipped
(*"no MEMORY.md link names it, so it has no curated title"*) rather than given a title invented
from its filename. It is idempotent: a file already carrying a `title:` is skipped. Each stamp also
pins `metadata.modified` to the file's pre-write mtime, so the write's own mtime bump cannot
re-date a claim nobody re-checked.

**`temper memory migrate` moves the files in, and it reconciles rather than bulk-imports.** Before
each write it searches the target context and **surfaces near-matches for you to judge** — it never
resolves an overlap itself. Three consequences a reader who expects a bulk import will trip over:

- It is **interactive by default**, and the prompt's default answer is *no*. With no terminal
  attached it **refuses to write** unless `--unattended` explicitly authorizes a run that *skips
  every collision* rather than resolving one.
- `--dry-run` is always permitted and writes nothing. Run it first.
- Re-running is safe. A file already in Temper is matched on `open_meta.source_file` and skipped,
  so a run interrupted halfway is resumed rather than duplicated.

One cohort per run, keyed on the files' own frontmatter `type`:

```bash
temper memory migrate --dry-run                     # cohort `feedback` — the default
temper memory migrate --cohort project --dry-run
temper memory migrate --cohort reference --dry-run
```

`feedback` is the cross-project cohort and lands in the first `shared_contexts` entry; every other
cohort lands in the first `project_contexts` entry, beside the resources it discusses. Override
with `--context <ref>`. A cohort whose list is empty is a refusal, never a silent fallback to the
other list — **reach is a property of which config list names the context**, and a memory never
carries one of its own.

Every skipped file is reported with its reason. A file that vanished silently from a migration
would be indistinguishable from one that migrated.

## Rendering and gating the index

```bash
temper memory emit          # render the index from Temper and write it
temper memory check         # exits non-zero when the on-disk index has drifted
```

`emit` **refuses to overwrite a file it did not write** — an existing hand-written `MEMORY.md`
survives until someone deliberately moves it aside — and it refuses the whole index rather than
render a memory whose `status` or `verified` is malformed. That refusal is the takeover gate, so
take it only once `harvest` has run.

The index lives outside any repo, so nothing in git can diff it; `check` is what a person or a hook
runs instead, gating on the exit code.
