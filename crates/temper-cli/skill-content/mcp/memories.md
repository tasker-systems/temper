# Memories

> **Memories are resources.** Durable working knowledge is stored as resources of type `memory`,
> carrying `open_meta.status` (`active` / `superseded`) and `open_meta.verified` (the date the claim
> was last checked). Read the active ones and honour them. A `verified` date far in the past means
> **nobody has re-checked the claim** — not that it is false.

## Populating the store is not a separate mechanism

A memory is authored, corrected and superseded with the ordinary resource tools, as doc type
`memory`, in whichever context the team already reads.

Tools: `create_resource` → `update_resource_meta`

Four things about the open tier are worth knowing before you write one, because **nothing validates
it at write time**:

- **`verified` is a judgment, not a timestamp.** Advance it only when the claim has actually been
  re-checked against the system it describes. Moving, re-rendering, or merely re-reading a memory
  must never advance it — a claim reading as checked when nobody checked it is the more dangerous
  direction of the two.
- **Correct by supersession.** Set the prior account's `status` to `superseded` and leave it
  readable, rather than editing the record of having been wrong out of existence. It stays
  addressable and searchable either way; falling out of a summary is never falling out of the
  record.
- **A memory carries no reach of its own.** Where it applies is a consequence of which context it
  lives in and who can read there, never something the memory asserts about itself. Choosing the
  context *is* choosing the audience.
- **A near-duplicate is for a human to judge.** Two accounts of nearly the same thing are surfaced
  for a decision, never merged automatically — one of them may be the account someone wanted to
  compare against.

## Recording that a memory was load-bearing

Call `update_resource` with `open_meta_add`, which unions over the stored list instead of replacing
it:

```json
{ "open_meta_add": { "reinforced": ["2026-08-02"] } }
```

`open_meta.reinforced` is a list of bare ISO dates: the days this memory did work. **Two things
count as a day's work, and the second is the one that matters.** The situation the memory describes
*recurred* — the trap it names actually fired. Or the memory *caught you*, and a mistake did not get
made because of it.

Counting only recurrence would be broken in a way worth understanding: **a memory that works
prevents its own situation from recurring.** It would go unreinforced, decay out of any summary, and
then the situation would recur — a loop that oscillates, each swing costing whatever the memory was
preventing. Counting the catch dissolves that.

**Use `open_meta_add`, never `open_meta`, for this key.** `open_meta` replaces each key it names, so
writing one date there discards every date already stored — silently, with a success response.

The date is bare, and the contract is bare deliberately. **No `by` field** — the update already
emits a `property_set` event carrying the acting principal, so the trail answers *who*, and a
self-asserted copy in a tier nothing validates could only ever disagree with the record that is
already right. **No note field** — a free-text note makes every record structurally unique, which
stops the union from collapsing same-day duplicates and loses the one-a-day grain that comes free
from the stored shape.

**The harder half: if the catch revealed a shape the memory's body does not describe, amend the
body — that is the act.** A metadata note *about* the gap is a breadcrumb standing in for the fix.
Reinforce the date *and* rewrite what the memory says. Amending a body is **not** re-checking the
claim, so it does not advance `verified`.

Some memories began as files on a contributor's machine and were moved in by a command-line
migration; those carry `open_meta.source_file`, naming the file they came from. That migration is a
machine-local concern and nothing on this surface runs it — but a memory that arrived that way is
an ordinary resource once it is here, and is read and corrected exactly like one authored here.
