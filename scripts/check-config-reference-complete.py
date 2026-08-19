#!/usr/bin/env python3
"""Cross-check the rendered config reference against a flat walk of the schema.

Why this exists
---------------

`check-config-reference-drift.sh` re-renders the page and diffs it. That proves
the page is **reproducible**, and says nothing about whether it is **complete** —
a renderer that drops a section renders a smaller page, the diff compares it
against an equally smaller committed page, and the gate goes green forever.

Here the risk is not hypothetical, it is measured. The renderer's tree walk had
**three** distinct traversal bugs, and each one silently omitted real fields:

  * `Option<MemoryConfig>` is spelled `anyOf: [{$ref}, {type: null}]`, not a bare
    `$ref`. The entire `[memory]` section — the most heavily documented part of
    the config — rendered as a single row of type "value".
  * `Vec<AuthProvider>` hides its element type behind `items.$ref`, so every
    field of `[[auth.providers]]` was absent.
  * A `$ref` to a string enum was labelled "table", which is not a completeness
    bug but is the same traversal blind spot wearing a different hat.

Together those accounted for 10 of 27 fields. Every one of them would have
shipped behind a green drift gate.

How it is independent
---------------------

This does NOT re-walk the tree. It reads the same schema **flatly** — every
`properties` map on the root and on every `$def`, with no notion of nesting,
paths, or sections — and compares the resulting multiset of leaf field names
against the rows the rendered page actually contains. A structured walk and a
flat enumeration are different enough that a bug in the first does not reproduce
in the second, which is the only property that makes a cross-check worth having.

What it cannot see, stated rather than assumed away
---------------------------------------------------

A field the SCHEMA itself omits — `#[serde(skip)]`, `#[schemars(skip)]` — is
invisible to both sides and this check will call it clean. The main path is
covered by the compiler rather than here: `schema_for!` will not compile unless
every reachable type derives `JsonSchema`, so a new section cannot be added
without one.
"""

import argparse
import json
import re
import sys
from pathlib import Path

# A rendered field row: `| `name` | type | default | description |`
ROW_RE = re.compile(r"^\|\s*`([a-z0-9_]+)`\s*\|")


def object_def_names(schema: dict) -> set[str]:
    """`$defs` entries that describe a TOML table (an object with fields)."""
    return {
        name
        for name, node in schema.get("$defs", {}).items()
        if isinstance(node, dict) and node.get("properties")
    }


def points_at_object(prop: dict, objects: set[str]) -> bool:
    """Does this property name a table — directly, through Option, or as an
    array element? Such a property becomes a SECTION, never a row."""
    refs: list[str] = []
    if "$ref" in prop:
        refs.append(prop["$ref"])
    for arm in prop.get("anyOf", []):
        if "$ref" in arm:
            refs.append(arm["$ref"])
    item = prop.get("items")
    if isinstance(item, dict) and "$ref" in item:
        refs.append(item["$ref"])
    if any(ref.rsplit("/", 1)[-1] in objects for ref in refs):
        return True
    # An inline object with fields is a table too.
    return prop.get("type") == "object" and bool(prop.get("properties"))


def flat_leaf_fields(schema: dict) -> list[str]:
    """Every leaf field name, from a walk that knows nothing about structure."""
    objects = object_def_names(schema)
    maps = [schema.get("properties", {})]
    maps.extend(
        node.get("properties", {})
        for node in schema.get("$defs", {}).values()
        if isinstance(node, dict) and node.get("properties")
    )
    leaves: list[str] = []
    for properties in maps:
        for name, prop in properties.items():
            if isinstance(prop, dict) and not points_at_object(prop, objects):
                leaves.append(name)
    return leaves


def rendered_fields(page: Path) -> list[str]:
    if not page.is_file():
        raise SystemExit(f"ERROR: no rendered page at {page}")
    rows = [m.group(1) for line in page.read_text().splitlines() if (m := ROW_RE.match(line))]
    if not rows:
        raise SystemExit(
            f"ERROR: {page} contains no field rows — refusing to report a clean cross-check "
            f"against an empty page."
        )
    return rows


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--input", required=True, type=Path, help="the emitted schema JSON")
    ap.add_argument("--tree", type=Path, default=Path("docs/reference/config"))
    args = ap.parse_args()

    if not args.input.is_file():
        print(f"ERROR: no such input file: {args.input}", file=sys.stderr)
        return 2

    document = json.loads(args.input.read_text())
    schema = document.get("schema")
    if not isinstance(schema, dict) or not schema.get("properties"):
        print(
            "ERROR: the emitted schema has no properties — refusing to cross-check against "
            "an empty schema.",
            file=sys.stderr,
        )
        return 1

    expected = sorted(flat_leaf_fields(schema))
    actual = sorted(rendered_fields(args.tree / "README.md"))

    if expected != actual:
        from collections import Counter

        missing = Counter(expected) - Counter(actual)
        extra = Counter(actual) - Counter(expected)
        print(
            "ERROR: the rendered config reference and the schema describe different field sets.",
            file=sys.stderr,
        )
        print(file=sys.stderr)
        for name, count in sorted(missing.items()):
            print(f"  IN SCHEMA, NOT RENDERED  `{name}`" + (f" x{count}" if count > 1 else ""), file=sys.stderr)
        for name, count in sorted(extra.items()):
            print(f"  RENDERED, NOT IN SCHEMA  `{name}`" + (f" x{count}" if count > 1 else ""), file=sys.stderr)
        print(file=sys.stderr)
        print(
            "       The drift gate cannot see this: it compares the rendered page against the\n"
            "       committed one, so a renderer that drops a field drops it from both sides\n"
            "       and stays green. That is what this check is for.",
            file=sys.stderr,
        )
        return 1

    print(
        f"Config reference is complete: {len(expected)} leaf fields, and a flat walk of the "
        f"schema agrees exactly with the rendered page"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
