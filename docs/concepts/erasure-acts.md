# Erasure acts — two purposes, four acts, one vocabulary

**For operators, auditors, and integrators.** Anyone who runs, audits, or reads the record of an
act under `temper admin erasure`, and needs to know which act serves which purpose and what each
word in its record means.

Every act on this page is irreversible, runs only for an instance administrator (system-admin
standing), and surveys by default: without `--execute` it reports what it would do and changes
nothing. A caller without system-admin standing is answered as a not-found before anything is
looked up, and the attempt adds nothing to the ledger.

## The two purposes

**(a) Person erasure.** A person asks for their data to be removed (a data-protection request, a
regulator's order), and the organization hosting Temper determines the request must be honored.
The act breaks the link between the person and their id, erases everything in their own
contexts, and leaves what they wrote into other teams' contexts in place, attributable only to a
pseudonym, except the text of a relationship that points at a resource in what it erases. It is
not part of leaving: see [Erasure and the terms of use](./erasure-and-terms-of-use.md).

**(b) Remediation.** Something that must not exist was written down: a secret, a credential, a
personal identifier, a confidential document. The person who wrote it, or a security team,
decides it needs removing beyond a soft delete. How far the act reaches depends on the leak: one
field of a resource, one block, or the whole resource.

Both purposes take the offending material out of the event ledger too, not only out of what reads
show, wherever the act's reach includes the ledger.

## Which act serves which purpose

| Act | Command | Purpose | Unit | The resource afterwards |
|---|---|---|---|---|
| [Person erasure](./erasure-and-terms-of-use.md) | `temper admin erasure principal` | (a) | a person and their own contexts | each resource in their contexts is erased by resource erasure, except a map's charter |
| [Resource erasure](./resource-erasure.md) | `temper admin erasure resource` | (b), the whole resource | one resource | a husk |
| [Block history scrub](./block-erasure-and-history-scrub.md) | `temper admin erasure block-history` | (b), one block | the history of named blocks | live; each live block keeps its current text |
| [Field scrub](./field-scrub.md) | `temper admin erasure field` | (b), metadata only | the history of one field | live; today's value kept unless `--clear` |

Person erasure is built on resource erasure: the same act runs over every resource in the
person's own contexts, and the person act adds the identity steps, the pseudonym break, and the
erasure of each relationship homed in those contexts that no resource erasure reached, whether it
was still live or already folded. The one exception is a cognitive map's charter: its text is
emptied, and it is held, not erased, until map-grain erasure exists. The record lists it among the
charters held.

These are **not** erasure, and none of them removes content from the ledger:

- deleting a resource (a soft delete: the resource stops reading as live);
- deleting a blob (a strike: see [Deleting a blob](./blob-delete-and-erasure.md));
- retiring a context (reversible);
- folding a block, an edge or a region;
- retracting a facet.

## The words

**Erase** removes the thing. Resource erasure and person erasure are the two erasing acts.

**Scrub** trims history or prior values and leaves the thing live. Block history scrub and field
scrub are the two scrubbing acts. They treat the ledger differently: block text never sits on the
ledger, only its hashes, so block history scrub rewrites no ledger payload; field scrub rewrites
the field's prior values on the ledger.

**`resource_scrubbed`** is the field scrub's event. It is field-grain despite its name: it never
records a whole-resource scrub, and never a block.

**Husk** means a resource with `erased_at` set. Only resource erasure makes one, whether an
operator runs it on one resource or person erasure runs it across an estate. A husk answers
`410 Gone` to those who held it directly: the owner of its home, or a holder of a read grant on
the resource itself. Everyone else, including those who read it only through its context or a
cognitive map, gets the same not-found an unknown id gets.

**Soft-deleted** means a resource deleted the ordinary way: it stops reading as live, and its
content and its ledger history stay. A soft-deleted resource is not a husk. It can be erased, and
then it becomes one; it can be scrubbed, and then it stays soft-deleted.

**Tombstone** means only a profile's `tombstoned_at`: the profile a person erasure has run on. Its
id stays, as the pseudonym, and its identifiers are gone.

**Redact** means rewriting paths on the ledger to placeholders. A record's `redacted_fields`
lists the paths an act rewrote. The one exception is the person erasure record's
`redacted_hashes`: it lists content hashes whose rows the act **emptied by hash**, such as a
held charter's text, and names no ledger rewrite.

**Empty** means a content row's text set to empty (or `{}` for structured content), with the row
kept. For a blob it means its pathname, type and bytes are gone.

**Sentinel** means a placeholder derived from ids and ledger order, never from time or operator
input, such as `erased-<id>`. Field scrub writes these placeholders on live resources, so the word
"erased" in a value does not mean the resource was erased; only `erased_at` and a
`resource_erased` record mean that.

**Fold** means visibility ends and the row still exists.

**Strike** means a blob record emptied, by a delete or by an erasure.

## Further reading

- **Person erasure, and the terms a contributor accepts:** [Erasure and the terms of use](./erasure-and-terms-of-use.md).
- **The CLI commands:** [`temper admin erasure`](../reference/cli/admin.md).
