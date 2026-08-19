#!/usr/bin/env python3
"""Emit `docs/reference/cli` from a BUILT `temper` binary's `--help` tree.

Why this shells out instead of reading the clap definitions
-----------------------------------------------------------

The thing users meet is the rendered help text of the binary they installed. A
source parser — even a correct one — answers a different question: what the
clap derives *say*, which is a claim about the tree rather than an observation
of it. Those can disagree (a `#[command(name = ...)]` rename, a feature gate, a
subcommand wired up in one place and not another), and when they do, a source
parser agrees with itself and disagrees with what ships.

So the ONLY input here is `<bin> <path...> --help`, captured as a subprocess.
This script never imports, links, or parses Rust.

It also never BUILDS. The caller passes `--bin`, because which binary is
documented is a decision the caller owns: the drift gate must document the tree
under review, not whatever happens to be on PATH.

Determinism
-----------

Three properties were measured against the real binary before this was written,
because each would otherwise make the drift gate red for reasons unrelated to
any diff:

* **Width.** clap wraps to the terminal width when stdout is a TTY. Here stdout
  is a pipe, and the output is then byte-identical under ``COLUMNS=60``,
  ``COLUMNS=200`` and unset — verified.
* **Machine-specific text.** No node's help embeds a home directory, a hostname
  or a detected core count. The `--embed-threads` help *describes* core
  detection but does not interpolate the result.
* **Order.** clap lists subcommands in declaration order, so the walk is stable
  across runs.

The whole 144-node walk costs about 0.6s, so nothing here is sampled or cached.

What it refuses to do
---------------------

An emitter that quietly writes less than it should is worse than one that
fails, because the drift gate downstream compares this output against *itself*
one commit later — it can prove reproducibility, never correctness. A parse
that silently dropped a subcommand would produce a stable, wrong tree and a
permanently green gate. So every way this script can under-produce is a hard
error, and the independent check that the tree is *complete* lives outside it,
in check-cli-reference-drift.sh, which compares against the Rust source — a
genuinely different derivation.
"""

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

GENERATED_MARKER = (
    "<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by "
    "scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->"
)

# A `Commands:` block ends at the first line that is not indented. `help` is
# clap's own auto-generated subcommand and is excluded: it exists at every node,
# documents nothing about temper, and would triple the tree.
CLAP_BUILTIN_SUBCOMMANDS = frozenset({"help"})


class EmitError(RuntimeError):
    """Anything that would make this emit an incomplete tree."""


def run_help(binary: Path, path: list[str]) -> str:
    """Capture one node's help text. Never tolerant: a node that will not
    render its own help is a broken tree, not a page to skip."""
    argv = [str(binary), *path, "--help"]
    try:
        proc = subprocess.run(argv, capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise EmitError(f"could not run {' '.join(argv)}: {exc}") from exc

    # clap prints `--help` to stdout and exits 0. A non-zero exit means the path
    # is not a real command — which, since every path here came from a parsed
    # `Commands:` block, means the PARSER is wrong, not the binary.
    if proc.returncode != 0:
        raise EmitError(
            f"`{' '.join(argv)}` exited {proc.returncode}. This path was read out of a "
            f"parent's Commands: block, so the block parser produced a name the binary "
            f"does not have.\nstderr: {proc.stderr.strip()[:400]}"
        )
    if not proc.stdout.strip():
        raise EmitError(f"`{' '.join(argv)}` printed nothing")
    return proc.stdout


def parse_subcommands(help_text: str) -> list[str]:
    """Names listed under `Commands:`, in clap's declaration order."""
    names: list[str] = []
    in_block = False
    for line in help_text.splitlines():
        if line.rstrip() == "Commands:":
            in_block = True
            continue
        if not in_block:
            continue
        if not line.strip() or not line.startswith("  "):
            break
        name = line.strip().split(None, 1)[0]
        if name not in CLAP_BUILTIN_SUBCOMMANDS:
            names.append(name)
    return names


def parse_options_block(help_text: str) -> str:
    """The verbatim `Options:` block, or '' when the node has none."""
    out: list[str] = []
    in_block = False
    for line in help_text.splitlines():
        if line.rstrip() == "Options:":
            in_block = True
            out.append(line)
            continue
        if not in_block:
            continue
        if not line.strip() or not line.startswith("  "):
            break
        out.append(line)
    return "\n".join(out)


class Node:
    def __init__(self, path: list[str], help_text: str):
        self.path = path
        self.help_text = help_text
        self.children: list[Node] = []

    @property
    def invocation(self) -> str:
        return " ".join(["temper", *self.path])

    def walk(self):
        yield self
        for child in self.children:
            yield from child.walk()


def build_tree(binary: Path) -> Node:
    root = Node([], run_help(binary, []))
    if not parse_subcommands(root.help_text):
        raise EmitError(
            "the root help lists no subcommands. Either the binary is not temper, or the "
            "`Commands:` block parser no longer matches clap's output. Refusing to emit "
            "an empty reference — a tree of one page would pass a drift gate forever."
        )

    def descend(node: Node) -> None:
        for name in parse_subcommands(node.help_text):
            child = Node([*node.path, name], run_help(binary, [*node.path, name]))
            node.children.append(child)
            descend(child)

    descend(root)
    return root


def render_node(node: Node, depth: int) -> list[str]:
    """A node's own help VERBATIM, then its descendants one level deeper.

    Verbatim is a decision, not laziness, and it was made against measurements
    rather than taste. Five global option lines repeat on every node and account
    for **30% of the whole reference** (62,305 of 207,800 bytes), so eliding
    them is genuinely tempting. Two attempts are recorded here because the next
    person will have the same idea:

    * **By block.** The root's `Options:` block is a superset — it alone carries
      `-V, --version` — so it matches nothing and the elision silently never
      fired. Reading the modal block off the corpus instead does identify the
      right text, but only 63 of 149 nodes have the globals as a standalone
      block at all; on the rest they are lines mixed in with the command's own.
    * **By line.** The set of option lines appearing verbatim in all 149 nodes
      is **empty**, because clap re-pads the help column to the widest option in
      each block — the `--vault` line alone has nine distinct paddings.

    So the only matching that survives is normalized (whitespace-insensitive)
    matching, and that is a fuzzy match. This script's entire doctrine is that
    it must never under-produce silently, and a fuzzy filter over what gets
    published is exactly that hazard. 30% boilerplate is the cheaper mistake:
    the reader sees what the binary actually prints, which is the point.
    """
    lines = [
        f"{'#' * depth} `{node.invocation}`",
        "",
        "```text",
        node.help_text.rstrip(),
        "```",
        "",
    ]
    for child in node.children:
        lines.extend(render_node(child, depth + 1))
    return lines


def render_index(root: Node) -> str:
    lines = [
        GENERATED_MARKER,
        "",
        "# CLI reference",
        "",
        "Every `temper` command, emitted from the built binary's own `--help`. If a page",
        "here disagrees with the binary in your hands, the page is a defect — nothing in",
        "this tree is hand-written.",
        "",
        "```text",
        root.help_text.rstrip(),
        "```",
        "",
        "## Commands",
        "",
        "| Command | Summary |",
        "| --- | --- |",
    ]
    for child in root.children:
        summary = child.help_text.splitlines()[0].strip().replace("|", "\\|")
        lines.append(f"| [`temper {child.path[0]}`](./{child.path[0]}.md) | {summary} |")
    lines.append("")
    return "\n".join(lines)


def emit(root: Node, out_dir: Path) -> int:

    # Wipe first. Without this a REMOVED command leaves its page behind as a
    # tracked, unmodified file — invisible to the `git status --porcelain` the
    # drift gate reads, so the gate stays green while the docs describe a
    # command that no longer exists. Re-emitting over the top cannot catch a
    # deletion; only a deletion can.
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True)

    (out_dir / "README.md").write_text(render_index(root))
    written = 1

    for child in root.children:
        summary = child.help_text.splitlines()[0].strip()
        lines = [
            GENERATED_MARKER,
            "",
            f"# `temper {child.path[0]}`",
            "",
            summary,
            "",
        ]
        # depth=2 for the command's OWN help: the H1 above already names it, and
        # emitting an H2 with the same text produced a page that introduced
        # itself twice.
        lines.extend(render_node(child, depth=2)[2:])
        (out_dir / f"{child.path[0]}.md").write_text("\n".join(lines).rstrip() + "\n")
        written += 1

    return written


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument(
        "--bin",
        required=True,
        type=Path,
        help="path to the BUILT temper binary to document. Required, and deliberately not "
        "defaulted to PATH: which binary gets documented is the caller's decision, and a "
        "drift gate must document the tree under review.",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=Path("docs/reference/cli"),
        help="output directory (default: docs/reference/cli). Its contents are REPLACED.",
    )
    args = parser.parse_args()

    if not args.bin.is_file():
        print(f"ERROR: no such binary: {args.bin}", file=sys.stderr)
        return 2

    try:
        root = build_tree(args.bin)
        node_count = sum(1 for _ in root.walk())
        written = emit(root, args.out)
    except EmitError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1

    # The gate parses this line to prove the emit did something. Keep the shape
    # stable, or check-cli-reference-drift.sh will refuse rather than quietly
    # compare nothing — which is the behaviour it wants.
    print(f"Emitted {written} cli reference files from {node_count} help nodes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
