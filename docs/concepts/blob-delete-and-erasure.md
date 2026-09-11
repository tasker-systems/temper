# Deleting a blob — custody, the ledger, and bytes

**For users, operators, and integrators.** Anyone who commits binary blobs (files) and needs
to know who can remove them, what the removal record looks like, and what happens to the
bytes.

## What deleting a blob is

Deleting a blob **strikes its record**: the row empties (the media type and byte count are
gone), and the blob reads as absent through every read path, list, and graph surface — as if
it had never been committed. One event is written to the ledger, carrying what was struck,
by whom, and when. That event is the whole story: a second delete of the same blob reads
exactly like deleting a blob that never existed (the same 404), and fires no second event.

Deleting is not editing. Removing a blob's *relation* to a resource (folding the relation) is
an edit of the relating resource — it touches neither the blob's record nor its bytes. The
only act that empties a blob record outside a subject erasure is the delete itself.

## Who may delete — custody, and nothing else

Delete standing is **custody**, resolved inside the delete's own transaction:

- **An attached blob** (it has at least one live relation to a resource): you need delete
  standing over **every** resource it relates to. Today that means you own the home each
  related resource lives in (or hold an explicit delete grant on it).
- **An unattached blob**: you need custody of its **home**. A personal context — its owner.
  A team context — a direct **owner role** in the owning team. A blob homed in a cognitive
  map has no custodian at all: the delete refuses rather than guess, and the erasure's
  sweep names such rows as held rather than striking them — today, nothing empties a
  map-homed blob's record.

What never confers delete standing:

- **Authorship.** Having uploaded a blob gives you no standing over it if custody has since
  moved elsewhere — and being the first uploader is never disqualifying either. Origin is
  provenance, never a gate.
- **Role.** A team `maintainer` or `member` cannot strike a team-homed blob; the team
  `owner` role is the single delete-level authority the role ladder carries, and it is
  scoped to blob homes — it never confers delete over a *resource*.
- **Administrator standing.** Instance administrators hold no delete standing by virtue of
  being administrators.

A caller who can read a blob but holds no custody gets a refusal naming as much; a caller
who cannot read it gets the same answer an unknown blob gets, so probing blob ids discloses
nothing.

## What the ledger remembers — and what the row cannot tell you

A struck row carries **no marker of which act emptied it**. An ordinary delete and a subject
erasure leave the identical row shape; what distinguishes them is the **event** each wrote —
`blob_deleted` for the ordinary act, the erasure event for an erasure. This is deliberate:
the ledger is the only place the distinction lives, and replaying the ledger reproduces
exactly whichever act emptied the row — a re-commit of identical bytes after either act
mints a fresh, ordinary row, never refused by and never deduplicated against the struck one.

## The team-context policy (erasure's bound)

Erasure — a subject's right to be forgotten — is bounded where the team's corpus begins: an
erasure never strikes bytes a team context holds and never empties team-homed blob rows. In
team contexts the erasure breaks *attribution* (the pseudonym break), not the bytes. The
delete act above is the ordinary, custodian-driven counterpart: it operates wherever custody
resolves, including team homes through the owner role. The bound has a mirror side: content
homed in the subject's *own* personal contexts wipes with the erasure whoever authored it —
including content guests wrote there under a grant — with one named exception: file rows a
guest *uploaded* there survive the sweep, named in the act's record rather than struck.
Both lines, and the terms of use they carry, are in
[Erasure and the terms of use](./erasure-and-terms-of-use.md).

## The bytes

Blob bytes live in content-addressed external storage, deduplicated across homes: N homes
committing byte-identical content produce N records over one stored object. A delete
therefore releases the bytes only when the struck record was the **last live record**
carrying its content hash — decided inside the delete's own transaction. Released bytes are
removed from storage after the delete commits; the release is watched by a retry-and-age-
alerting fence, so a storage outage strands nothing silently.

## Relations and the narrow door

Blobs relate to **resources**. The relate door refuses blob-to-cogmap and blob-to-blob
relations: no delete standing could ever resolve over such a peer, so the relation would
pin the blob's record permanently against any future delete. A handful of relations minted
before that narrowing may still exist — a blob related to a cogmap, say — and such a
relation does exactly what it always would: it holds the blob's record until someone with
standing over it folds it (the dependent's exit). If a delete is refused with the custody
message and you hold custody of every resource you can see, look for one of these legacy
relations on the blob's relations list. Existing relations to a resource are unaffected by
a delete of another blob, and a struck blob's own relations persist as history — they
render absent because the blob is gone, not because they were ended.

## Further reading

- **Teams and the role ladder:** [Teams and roles](./teams-and-roles.md).
- **The context model blobs are homed in:** [Contexts and Refs](./contexts-and-refs.md).
- **The CLI commands:** [Blob reference](../reference/cli/blob.md).
