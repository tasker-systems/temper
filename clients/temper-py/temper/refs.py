"""Ref resolution: `sluggify(title)-<uuid>` (or a bare UUID) to its UUID.

A pure port of `temper_workflow::operations::parse_ref`, and of the gem's
`lib/temper/refs.rb`. There is no by-slug lookup, so this never touches the network.
It does NOT port `sluggify`: the server derives the slug from the title.
"""

from __future__ import annotations

import re

# A hyphenated UUID. Deliberately narrower than Rust's `Uuid::parse_str`, which also
# accepts the simple (unhyphenated), braced, and URN forms: this package only ever
# hands the string straight back to the caller to put in a URL, so widening the accept
# set would mean emitting a non-canonical id. Narrower is safe — it rejects only
# inputs the server would have accepted.
_HEX = "[0-9a-fA-F]"
_UUID = re.compile(rf"\A{_HEX}{{8}}-{_HEX}{{4}}-{_HEX}{{4}}-{_HEX}{{4}}-{_HEX}{{12}}\Z")


def parse_ref(ref: str) -> str:
    """Resolve a ref to its UUID.

    Resolution is trailing-UUID-only, so a stale slug half is harmless. No fuzzy
    matching — unparseable input raises, never guesses.
    """
    if not isinstance(ref, str):
        raise TypeError("ref must be a str")

    ref = ref.strip()
    if _UUID.match(ref):
        return ref

    # A UUID contains four internal hyphens, so it is the last five hyphen-delimited
    # groups. Walk from the right.
    parts = ref.split("-")
    if len(parts) >= 5:
        tail = "-".join(parts[-5:])
        if _UUID.match(tail):
            return tail

    raise ValueError(f"not a ref (expected a UUID or `slug-<uuid>`): {ref!r}")
