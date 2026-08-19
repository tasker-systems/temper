#!/usr/bin/env python3
"""Render `docs/reference/config` from the JSON that temper-core's
`config-reference` example emits.

Input is the document produced by:

    cargo run -q -p temper-core --features config-schema --example config-reference

which carries the schemars JSON Schema for `TemperConfig`, the real
`TemperConfig::default()` as JSON, and the same defaults rendered as TOML by the
same `toml` crate that parses config.toml at load time.

Why the schema rather than the source
-------------------------------------

The valuable half of a config reference is the prose, and the prose is the doc
comments — which live in Rust source. The tempting shortcut is to scrape `///`
lines out of `config.rs`. schemars exists precisely so that is unnecessary: the
derive lifts each doc comment into the schema's `description`, so the reference
is rendered from a **compiled artifact** of the types rather than from a parse
of the file that declares them. A struct that stops matching its documentation
cannot happen, because there is only one of them.

Undocumented fields are NOT rendered blank
------------------------------------------

A field with no doc comment gets an explicit `(undocumented)` marker and is
counted in a summary line at the foot of the page. Rendering it as an empty
table cell would let a documentation hole read as documentation — coverage
inferred from absence, which is the one thing every checking tool in this repo
refuses to do. The marker is deliberately ugly: it should cost something.
"""

import argparse
import json
import re
import sys
from pathlib import Path

GENERATED_MARKER = (
    "<!-- GENERATED — do not edit. Rendered from temper-core's `config-reference` example by "
    "scripts/emit-config-reference.py; see .github/scripts/check-config-reference-drift.sh. -->"
)

UNDOCUMENTED = "_(undocumented — this field has no doc comment in `TemperConfig`)_"


class RenderError(RuntimeError):
    """Anything that would make this render an incomplete or empty reference."""


def resolve(node: dict, defs: dict) -> dict:
    """Follow a `$ref` — including one wrapped in the `anyOf` that schemars uses
    for `Option<T>` — one hop into `$defs`.

    The `anyOf` arm is not a nicety. `Option<MemoryConfig>` is spelled
    `anyOf: [{$ref}, {type: null}]`, and a resolver that only understood a bare
    `$ref` walked straight past it: the entire `[memory]` section — the most
    heavily documented part of the config — was silently absent from the first
    rendered page, showing up as one row of type "value".
    """
    ref = node.get("$ref")
    if not ref:
        arms = [a for a in node.get("anyOf", []) if a.get("type") != "null"]
        if len(arms) == 1 and "$ref" in arms[0]:
            ref = arms[0]["$ref"]
    if not ref:
        return node
    name = ref.rsplit("/", 1)[-1]
    if name not in defs:
        raise RenderError(f"schema references unknown definition: {ref}")
    return defs[name]


def is_optional(node: dict) -> bool:
    """`Option<T>` — either ["T","null"] for scalars or an `anyOf` null arm."""
    types = node.get("type")
    if isinstance(types, list) and "null" in types:
        return True
    return any(a.get("type") == "null" for a in node.get("anyOf", []))


def struct_target(node: dict, defs: dict) -> dict | None:
    """The object definition this property ultimately names, if any."""
    resolved = resolve(node, defs)
    if resolved is not node or "$ref" in node or "anyOf" in node:
        if resolved.get("properties"):
            return resolved
    if resolved.get("type") == "object" and resolved.get("properties"):
        return resolved
    return None


def array_item_struct(node: dict, defs: dict) -> dict | None:
    """For `Vec<SomeStruct>`, the struct — so its fields get documented too
    rather than hiding behind `array of AuthProvider`."""
    if node.get("type") != "array":
        return None
    item = node.get("items")
    if not isinstance(item, dict) or "$ref" not in item:
        return None
    resolved = resolve(item, defs)
    return resolved if resolved.get("properties") else None


def type_name(node: dict, defs: dict) -> str:
    """A human type for the table, without pretending more precision than the
    schema carries."""
    optional = " (optional)" if is_optional(node) else ""

    # A `$ref` to a string enum should show its VALUES — that is the whole
    # content of the type, and "table" (the first version's answer) was not just
    # vague but wrong: LlmProviderType is a string, not a TOML table.
    if "$ref" in node or "anyOf" in node:
        target = resolve(node, defs)
        if "enum" in target:
            return " \\| ".join(f"`{v}`" for v in target["enum"]) + optional
        if target.get("properties"):
            return f"table{optional}"

    types = node.get("type")
    if isinstance(types, list):
        non_null = [t for t in types if t != "null"]
        return f"{non_null[0] if non_null else 'null'}{optional}"
    if types == "array":
        item = node.get("items", {})
        if "$ref" in item:
            return f"array of {item['$ref'].rsplit('/', 1)[-1]} tables"
        return f"array of {item.get('type', 'value')}"
    if types == "object":
        return f"table{optional}"
    if "enum" in node:
        return " \\| ".join(f"`{v}`" for v in node["enum"]) + optional
    return f"{types or 'value'}{optional}"


def format_default(value) -> str:
    if value is None:
        return "_unset_"
    if isinstance(value, bool):
        return f"`{str(value).lower()}`"
    if isinstance(value, (int, float)):
        return f"`{value}`"
    if isinstance(value, str):
        return "`\"\"` (empty)" if value == "" else f"`{value}`"
    if isinstance(value, list):
        return "`[]`" if not value else f"`{json.dumps(value)}`"
    if isinstance(value, dict):
        return "_(table)_"
    return f"`{value}`"


# A rustdoc intra-doc link — `[`stale_after_days`](Self::stale_after_days)`. Perfectly
# valid in `cargo doc`, and a BROKEN link the moment it is published as markdown: no
# reader of the docs site can resolve `Self::`. Caught in the first rendered page, in
# MemoryConfig's longest doc comment.
RUSTDOC_LINK_RE = re.compile(r"\[([^\]]+)\]\((?:Self|crate|super)::[^)]+\)")


def clean(text: str) -> str:
    """Doc comments are multi-line and contain markdown; a table cell is one line."""
    text = RUSTDOC_LINK_RE.sub(r"\1", text)
    return " ".join(text.split()).replace("|", "\\|")


class Renderer:
    def __init__(self, schema: dict, defaults: dict):
        self.defs = schema.get("$defs", {})
        self.schema = schema
        self.defaults = defaults
        self.undocumented: list[str] = []
        self.field_count = 0

    def render_object(
        self, node: dict, defaults, toml_path: str, depth: int, is_array: bool = False
    ) -> list[str]:
        node = resolve(node, self.defs)
        properties = node.get("properties", {})
        if not properties:
            return []

        required = set(node.get("required", []))
        # `[[x]]` is TOML's array-of-tables spelling, and using it here means the
        # heading is something a reader can literally paste into config.toml.
        description = node.get("description")
        if not toml_path:
            # The root carries no scalar fields of its own — every top-level property
            # is a section — so a heading here would introduce an empty one. Its doc
            # comment still earns its place as the section's preamble.
            lines = []
            if description:
                lines.extend([clean(description), ""])
        else:
            heading = f"[[{toml_path}]]" if is_array else f"[{toml_path}]"
            lines = [f"{'#' * depth} `{heading}`", ""]
            if description:
                lines.extend([clean(description), ""])

        nested: list[tuple[str, dict, object, bool]] = []
        rows = []
        for name, prop in sorted(properties.items()):
            resolved = resolve(prop, self.defs)
            # Two sources, in this order and for a reason. The SERIALIZED default is
            # authoritative where it exists, because it is the value the program
            # actually starts with. But it only reaches fields under sections that
            # are present in `TemperConfig::default()` — everything under `[memory]`
            # is absent there, since memory defaults to `None`. For those the schema's
            # own `default` (from `#[serde(default = "...")]`) is the only record, and
            # without this fallback every documented MemoryConfig default read
            # `_unset_`, which is wrong rather than merely incomplete.
            if isinstance(defaults, dict) and name in defaults:
                default_value = defaults[name]
            elif "default" in prop:
                default_value = prop["default"]
            else:
                default_value = None

            child_path = f"{toml_path}.{name}" if toml_path else name

            # A nested table gets its own section rather than an inscrutable row.
            if struct_target(prop, self.defs) is not None:
                nested.append((child_path, prop, default_value, False))
                continue

            # An array OF tables gets one too. `array of AuthProvider tables` is a
            # type, not documentation: without this the fields of every element are
            # simply absent from the reference.
            if array_item_struct(prop, self.defs) is not None:
                nested.append((child_path, prop["items"], None, True))
                continue

            self.field_count += 1
            # The description may sit on the property or, for a $ref'd scalar
            # newtype, on the definition it points at.
            desc = prop.get("description") or resolved.get("description")
            if desc:
                desc = clean(desc)
            else:
                desc = UNDOCUMENTED
                self.undocumented.append(f"{toml_path}.{name}" if toml_path else name)

            marker = " **(required)**" if name in required else ""
            rows.append(
                f"| `{name}` | {type_name(prop, self.defs)}{marker} "
                f"| {format_default(default_value)} | {desc} |"
            )

        if rows:
            lines.extend(["| Field | Type | Default | Description |", "| --- | --- | --- | --- |"])
            lines.extend(rows)
            lines.append("")

        for path, prop, sub_defaults, is_array in nested:
            lines.extend(self.render_object(prop, sub_defaults, path, depth + 1, is_array))
        return lines


def render(document: dict) -> tuple[str, int, list[str]]:
    schema = document.get("schema")
    defaults = document.get("defaults")
    defaults_toml = document.get("defaults_toml")
    if not isinstance(schema, dict) or not schema.get("properties"):
        raise RenderError(
            "the emitted schema has no properties. Either the example printed something else, "
            "or the JsonSchema derives are not in force. Refusing to render an empty reference "
            "— a page describing no fields would satisfy a drift gate forever."
        )
    if not isinstance(defaults_toml, str) or not defaults_toml.strip():
        raise RenderError("the emitted document carries no `defaults_toml`")

    renderer = Renderer(schema, defaults or {})
    body = renderer.render_object(schema, defaults or {}, "", 2)
    if renderer.field_count == 0:
        raise RenderError("walked the schema and found no fields — refusing to render")

    lines = [
        GENERATED_MARKER,
        "",
        "# Configuration reference",
        "",
        "Every field of `TemperConfig`, rendered from the type itself. Descriptions are the",
        "doc comments on the Rust struct, and defaults are the real `TemperConfig::default()`,",
        "so a page that disagrees with the binary is a defect — nothing here is hand-written.",
        "",
        "Config lives at `~/.config/temper/config.toml`, or wherever `TEMPER_GLOBAL_CONFIG`",
        "points. An absent file is not an error: every section falls back to the defaults below.",
        "",
        "## Defaults",
        "",
        "The starting config, serialized by the same `toml` crate that parses it at load time.",
        "Sections whose fields are all unset appear as empty tables.",
        "",
        "```toml",
        defaults_toml.rstrip(),
        "```",
        "",
        "## Fields",
        "",
    ]
    lines.extend(body)

    if renderer.undocumented:
        lines.extend(
            [
                "## Undocumented fields",
                "",
                f"{len(renderer.undocumented)} of {renderer.field_count} fields carry no doc "
                "comment on the Rust struct, so this reference cannot describe them. They are "
                "listed rather than left as blank cells, because a documentation hole that "
                "renders as whitespace reads as documentation.",
                "",
            ]
        )
        lines.extend(f"- `{path}`" for path in sorted(renderer.undocumented))
        lines.append("")
    else:
        lines.extend(
            ["## Undocumented fields", "", f"None — all {renderer.field_count} fields are documented.", ""]
        )

    return "\n".join(lines), renderer.field_count, renderer.undocumented


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument(
        "--input",
        required=True,
        type=Path,
        help="file holding the example's JSON output. A FILE, not a pipe, on purpose: piping "
        "hides the producer's exit status, and an empty stdin from a failed `cargo run` would "
        "otherwise reach the renderer as valid-looking absence.",
    )
    ap.add_argument("--out", type=Path, default=Path("docs/reference/config"))
    args = ap.parse_args()

    if not args.input.is_file():
        print(f"ERROR: no such input file: {args.input}", file=sys.stderr)
        return 2
    try:
        document = json.loads(args.input.read_text())
    except json.JSONDecodeError as exc:
        print(f"ERROR: {args.input} is not valid JSON: {exc}", file=sys.stderr)
        return 1

    try:
        page, field_count, undocumented = render(document)
    except RenderError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1

    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "README.md").write_text(page + "\n" if not page.endswith("\n") else page)

    # The gate parses this line to prove the render did something. Keep the shape stable.
    print(
        f"Emitted 1 config reference files describing {field_count} fields "
        f"({len(undocumented)} undocumented)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
