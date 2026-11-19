# Field scrub — when the leak is in a title, an origin, or a property

**For operators and auditors.** Anyone remediating a leak in a resource's metadata rather than its
content: a secret pasted into a title, a token in an origin URI, a personal identifier in a
property value.

## Its purpose

Field scrub serves **remediation** ([purpose (b)](./erasure-acts.md#the-two-purposes)) for metadata
only. It removes every earlier value of one field from the projections and from the ledger, and
the resource survives. It is a scrub, not an erasure. Its event is `resource_scrubbed`, which is
field-grain despite its name: it never records a whole-resource scrub.

## Running it

`temper admin erasure field <resource>` with no `--field` lists the fields and property families to
choose from, by handle, count and date, never by key or value. Then:

- `--field title` or `--field origin_uri`;
- `--field property --family <handle>`: one property family;
- `--field properties`: every family's earlier values.

Without `--execute` the command surveys. The API doors are
`POST /api/admin/resources/field-scrub/families`, `POST /api/admin/resources/field-scrub/survey`
and `POST /api/admin/resources/field-scrub`. No request, answer or record carries a key or a
value. The act is refused for an erased resource and for a cognitive map's charter.

**Today's value is kept by default.** Only the values the field held before are redacted. When
today's value is part of the leak, `--clear` replaces it first, with a placeholder title or
origin, or by unsetting the family, through ordinary events, and then redacts every value the
field held. `--field properties` cannot be cleared.

## What it changes

- the field's earlier values on the ledger, rewritten to placeholders and listed in the record's
  `redacted_fields`;
- the folded property rows that held earlier values of the family;
- the resource's search data, rebuilt.

## What it leaves

- every block, chunk and revision: for content, use [block history scrub](./block-erasure-and-history-scrub.md);
- properties owned by the resource's blocks and by its edges;
- edge labels.

Because the placeholders it writes look like erasure's, the word "erased" in a value of a live
resource means a field was scrubbed, not that the resource was erased.

## Further reading

- **The two purposes and the vocabulary:** [Erasure acts](./erasure-acts.md).
- **When the whole resource must go:** [Resource erasure](./resource-erasure.md).
