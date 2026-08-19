#!/usr/bin/env python3
"""Cross-check the committed CLI reference against the clap source.

Why this exists, and why it is not the same check as the drift gate
-------------------------------------------------------------------

`check-cli-reference-drift.sh` re-runs the emitter and compares the result to
what is committed. That proves the tree is **reproducible**. It cannot prove the
tree is **complete**, and the distinction is the whole point:

    The emitter discovers subcommands by parsing each node's `Commands:` block
    out of rendered help text. If that parse ever drops a command — a clap
    output change, a name with unusual spacing, an unanticipated block
    terminator — the emitter produces a smaller tree, the drift gate compares
    that smaller tree against the equally smaller committed one, and goes
    green. Forever. The reference would simply stop mentioning a command and
    nothing would ever say so.

A gate that compares an artifact against itself measures reproducibility, not
correctness. So completeness is established from a **second, independent
derivation**: the clap definitions in `crates/temper-cli/src/cli.rs`. Rendered
help text and Rust source are different enough surfaces that a bug in one is
very unlikely to be mirrored in the other.

This is deliberately the *cross-check* and never the generator. A source parser
answers "what do the derives say", which is a claim about the tree; the emitter
answers "what does the binary print", which is an observation of it. When they
disagree the binary wins — but the disagreement itself is the finding, and
without this script nothing would ever notice one.

On parsing Rust with regular expressions
----------------------------------------

Fragile in general, adequate here, and the fragility fails in the safe
direction: this script reports a mismatch rather than adjudicating one, so a
parser that under-reads produces a loud failure, not a silent pass.

One structural rule matters, and it was learned by getting it wrong. The first
version keyed on the field name and type (`action: SomethingAction`) and
silently missed three whole subtrees — `cogmap`, `invocation` and `steward` all
spell it `cmd: SomethingCmd`. Keying on the ATTRIBUTE instead (the field
declared immediately after `#[command(subcommand)]`, whatever it and its type
are called) reconstructs all 148 nodes exactly. Naming conventions are not
load-bearing; the attribute is.
"""

import argparse
import re
import sys
from pathlib import Path

# `pub enum Foo {` … up to the closing brace in column 0.
ENUM_RE = re.compile(r"^pub enum (\w+) \{$(.*?)^\}$", re.M | re.S)
# A variant: four-space indent, CamelCase, then `{`, `,` or end of line.
VARIANT_RE = re.compile(r"^    ([A-Z]\w*)\s*(\{|,|$)")
# An explicit rename, e.g. `#[command(name = "region-metrics")]`.
RENAME_RE = re.compile(r'^    #\[command\([^)]*name = "([^"]+)"')
# The field that carries a subcommand — matched on the ATTRIBUTE above it, not
# on its own name or type. See the module docstring.
SUBCOMMAND_ATTR = "#[command(subcommand)]"
FIELD_TYPE_RE = re.compile(r"^\s*\w+:\s*([A-Za-z]\w*)\s*,")
# Emitted headings look like: `### \`temper admin connection provision\``
HEADING_RE = re.compile(r"^#{1,6}\s+`temper\s+([^`]+)`\s*$")


def kebab(name: str) -> str:
    """clap's default variant-to-command rename."""
    return re.sub(r"(?<!^)(?=[A-Z])", "-", name).lower()


def parse_enums(source: str) -> dict[str, str]:
    return {m.group(1): m.group(2) for m in ENUM_RE.finditer(source)}


def parse_variants(body: str) -> list[tuple[str, str | None, str | None]]:
    """(variant, explicit_rename, child_enum) in declaration order."""
    lines = body.split("\n")
    out: list[tuple[str, str | None, str | None]] = []
    rename: str | None = None
    i = 0
    while i < len(lines):
        rm = RENAME_RE.match(lines[i])
        if rm:
            rename = rm.group(1)
        vm = VARIANT_RE.match(lines[i])
        if vm:
            name, child = vm.group(1), None
            if vm.group(2) == "{":
                j, depth = i, 0
                while j < len(lines):
                    depth += lines[j].count("{") - lines[j].count("}")
                    if lines[j].strip() == SUBCOMMAND_ATTR:
                        for k in range(j + 1, min(j + 4, len(lines))):
                            fm = FIELD_TYPE_RE.match(lines[k])
                            if fm:
                                child = fm.group(1)
                                break
                    if depth == 0 and j > i:
                        break
                    j += 1
                i = j
            out.append((name, rename, child))
            rename = None
        i += 1
    return out


def tree_from_source(source_path: Path) -> set[str]:
    enums = parse_enums(source_path.read_text())
    if "Commands" not in enums:
        raise SystemExit(
            f"ERROR: no `pub enum Commands` in {source_path}. Either the CLI root moved or "
            f"the enum parser no longer matches. Refusing to report a clean cross-check "
            f"against an empty source tree."
        )
    paths: set[str] = set()

    def walk(enum_name: str, prefix: list[str]) -> None:
        for name, rename, child in parse_variants(enums.get(enum_name, "")):
            path = [*prefix, rename or kebab(name)]
            paths.add(" ".join(path))
            if child in enums:
                walk(child, path)

    walk("Commands", [])
    return paths


def tree_from_docs(tree_dir: Path) -> set[str]:
    pages = sorted(tree_dir.glob("*.md"))
    if not pages:
        raise SystemExit(
            f"ERROR: no markdown pages under {tree_dir} — refusing to report a clean "
            f"cross-check against an empty tree."
        )
    paths: set[str] = set()
    for page in pages:
        for line in page.read_text().splitlines():
            hm = HEADING_RE.match(line)
            if hm:
                # The index's own `temper` root heading is not a command path.
                invocation = " ".join(hm.group(1).split())
                if invocation:
                    paths.add(invocation)
    return paths


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--tree", type=Path, default=Path("docs/reference/cli"))
    ap.add_argument("--source", type=Path, default=Path("crates/temper-cli/src/cli.rs"))
    args = ap.parse_args()

    if not args.source.is_file():
        print(f"ERROR: no such source file: {args.source}", file=sys.stderr)
        return 2
    if not args.tree.is_dir():
        print(f"ERROR: no such reference tree: {args.tree}", file=sys.stderr)
        return 2

    from_source = tree_from_source(args.source)
    from_docs = tree_from_docs(args.tree)

    missing = sorted(from_source - from_docs)
    extra = sorted(from_docs - from_source)

    if missing or extra:
        print(
            "ERROR: the committed CLI reference and the clap source describe different trees.",
            file=sys.stderr,
        )
        print(file=sys.stderr)
        for path in missing:
            print(f"  IN SOURCE, NOT DOCUMENTED  temper {path}", file=sys.stderr)
        for path in extra:
            print(f"  DOCUMENTED, NOT IN SOURCE  temper {path}", file=sys.stderr)
        print(file=sys.stderr)
        print(
            "       The drift gate cannot see this: it compares the emitted tree against\n"
            "       the committed one, so an emitter that drops a command drops it from\n"
            "       both sides and stays green. That is what this check is for.\n"
            "\n"
            "       If the binary is right and this parser is wrong, fix the parser in\n"
            "       scripts/check-cli-reference-complete.py — do NOT silence the check.",
            file=sys.stderr,
        )
        return 1

    print(
        f"CLI reference is complete: {len(from_source)} command paths, and the clap source "
        f"and the committed pages agree exactly"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
