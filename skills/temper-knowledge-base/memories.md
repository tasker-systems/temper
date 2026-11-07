# Memories

Read this when saving, correcting or reinforcing a durable memory, or deciding whether an existing
one still holds.

## The model

- **A memory is a resource of type `memory`.** Author, correct and supersede it with the ordinary
  resource tools, in the context whose readers it is for. Tools: `create_resource` →
  `update_resource_meta`.
- **`open_meta.status` is `active` or `superseded`.** Read the active ones and honour them.
- **`open_meta.verified` is the date the claim was last checked. `verified` is a judgment, not a
  timestamp:** advance it only after re-checking the claim against the system it describes; moving,
  re-rendering, re-reading or amending the body never advances it. An old date means nobody
  re-checked, not that it is false.
- **Correct by supersession.** Set the old account's `status` to `superseded` and leave it readable;
  it stays addressable and searchable.
- **A memory carries no reach of its own.** Its audience is whoever reads the context it lives in,
  so choosing the context is choosing the audience.
- **Nothing validates the open tier at write time.** These rules hold only if you follow them.
- Some memories carry `open_meta.source_file`: they were migrated from a contributor's machine by
  the CLI. Nothing on this surface runs that migration; once here, such a memory is an ordinary
  resource.

## Near-duplicates are for a human to judge

Before authoring into a context others write to, read what is already there.
**Compare the claim, not the wording**: two memories naming the same subsystem, command or CI job
often assert unrelated things, and a shared context accumulates exactly that vocabulary. Expect
three outcomes:

- **Duplicate**: one incident recorded twice, often from two machines. Keep the older, richer one.
- **Supersession**: a newer account strictly richer than the one it covers.
- **Both stale**: both out of date, the newer one still wrong. Recency arbitrates nothing.

Surface the short list; resolving it is a human's call. **Never merge automatically.** This is
guidance, not a gate.

## Recording that a memory was load-bearing

Call `update_resource` with `open_meta_add`, which unions over the stored list:

```json
{ "open_meta_add": { "reinforced": ["2026-08-02"] } }
```

`open_meta.reinforced` is a list of bare ISO dates: the days the memory did work. Two things count:
the situation it describes recurred, or **it caught you** before the mistake. Count the catch,
because a memory that works prevents its own situation from recurring.

If the catch exposed a case the body does not describe, **amend the body**; a metadata note about
the gap is not the fix. Amending the body does not advance `verified`.

## Traps

- **`open_meta` instead of `open_meta_add`**: replaces the `reinforced` list and silently discards
  every stored date, with a success response.
- **Adding a `by` or note field to a reinforcement**: the `property_set` event already records who,
  and a note makes every entry unique, so same-day duplicates stop collapsing.
- **Advancing `verified` because you read or moved a memory**: a claim that reads as checked when
  nobody checked it is the more dangerous error.
- **Treating a superseded memory as gone**: it is still in the record; falling out of a summary is
  not falling out of the record.
