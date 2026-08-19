#!/usr/bin/env python3
"""Report reach, link integrity and publish coverage for the public docs tree.

`docs/` is synced wholesale to the Apidog documentation site. This answers the
questions nothing else in the repo can:

    which pages can a reader actually get to, which links are broken, and did
    what we committed actually get published?

Discipline this script obeys, and why it matters here
-----------------------------------------------------

Inherited deliberately from `scripts/register-coverage.py`, which is the model.

**It detects; it does not decide.** An orphan page is not an error. A page may be
legitimately unreachable from a door — reference material reached from the site's
own navigation, a page linked only from prose elsewhere. So orphans are reported
and never set a non-zero exit. Only *factual* defects do, under --strict, and
there is exactly one: a link whose target does not exist.

**Coverage is never inferred from absence.** Every place this script cannot see
is reported as UNKNOWN rather than as clean. The published navigation tree is the
sharpest case and is discussed below.

**A pre-existing set is reported, never adjudicated.** 25 links in `docs/` point
outside the published tree. Every one is dead for a reader of the site, and
every one predates this script. Failing on them would make the check red the day
it landed, and a gate that fails on arrival trains people to stop running it —
the lesson `register-coverage.py` records under CROSS-GOAL.

What this CANNOT see, stated rather than assumed away
------------------------------------------------------

**The published navigation tree.** Apidog reconciles PAGES but leaves FOLDER
NODES behind: after the tree went from 550 pages to 36, about fourteen empty
folder shells stayed in the sidebar, and `doors/` sorted below all of them. That
was confirmed by clicking each one in a browser. It is invisible here, and not
for want of trying — Apidog's entire public API is four operations (import
OpenAPI, import Postman, export OpenAPI, assign a team role), with no endpoint
for navigation nodes or ordering, and probing twenty plausible route names under
`/v1/projects/{id}/` returned a redirect-to-help for every one. Empty folders
contribute no lines to `llms.txt`, so nothing here can observe them. Pruning and
ordering are manual actions in the Apidog UI.

So the nav is reported UNKNOWN. It is not counted clean, and a green run of this
script must never be read as "the site navigates correctly".

**The publish comparison is against PRODUCTION.** `llms.txt` reflects the last
publish of `main`. Run from a branch, every page the branch adds will read as
unpublished and every page it removes as orphaned-in-production. That is correct
behaviour and useless signal; the check says which revision it compared against
so the difference is legible rather than alarming.
"""

import argparse
import json
import os
import re
import sys
import urllib.error
import urllib.request
from collections import deque
from pathlib import Path

LINK_RE = re.compile(r"\[[^\]]*\]\(([^)]+)\)")
EXTERNAL_PREFIXES = ("http://", "https://", "mailto:", "tel:")
# Overridable so test-docs-coverage.sh can point at an unreachable host and assert the
# UNKNOWN path, rather than testing it by unplugging the network. No CI job sets it.
LLMS_TXT_URL = os.environ.get("DOCS_COVERAGE_LLMS_URL", "https://docs.temperkb.io/llms.txt")

# Subtrees under docs/reference/ and the generator that owns each. A path here
# that no entry claims is content nothing regenerates and no drift gate covers,
# sitting in a tree whose whole premise is that it is generated.
CLAIMED_REFERENCE_SUBTREES = {
    "cli": "scripts/emit-cli-reference.py",
    "config": "scripts/emit-config-reference.py",
}
GENERATED_MARKER_RE = re.compile(r"<!--\s*GENERATED\b")


class TreeUnreadable(RuntimeError):
    """The tree cannot be parsed, so no number this script prints would mean anything."""


def strip_code_fences(text: str) -> str:
    """Blank out fenced blocks, preserving line numbering.

    Not optional, and not defensive. `docs/guides/operational-memory.md` shows a
    rendered MEMORY.md inside a fence, and that sample contains
    `[title](019fc011-…)` links to vault resources. Matched naively they are two
    dangling links that no edit can fix, in a page that is completely correct —
    and --strict fails on dangling links, so this would have made the one
    blocking check permanently red over illustrative content.
    """
    out: list[str] = []
    fence: str | None = None
    for line in text.split("\n"):
        stripped = line.lstrip()
        if fence is None:
            if stripped.startswith("```") or stripped.startswith("~~~"):
                fence = stripped[:3]
                out.append("")
                continue
            out.append(line)
        else:
            if stripped.startswith(fence):
                fence = None
            out.append("")
    return "\n".join(out)


def page_links(page: Path) -> list[str]:
    return [m.group(1) for m in LINK_RE.finditer(strip_code_fences(page.read_text()))]


class Report:
    def __init__(self, docs_root: Path):
        self.docs_root = docs_root
        self.pages: list[Path] = []
        self.dangling: list[tuple[Path, str]] = []
        self.escaping: list[tuple[Path, str]] = []
        self.reached_via: dict[Path, set[str]] = {}
        self.orphans: list[Path] = []
        self.unclaimed_reference: list[Path] = []
        self.unmarked_reference: list[Path] = []
        self.publish: dict = {"status": "unknown", "detail": "not checked"}


def collect(docs_root: Path) -> Report:
    report = Report(docs_root)
    report.pages = sorted(docs_root.rglob("*.md"))

    # Refuse-to-report-zero, in all three shapes it can take. A tree this script
    # cannot read is not a tree with no problems, and every number below would be
    # a confident zero derived from having looked at nothing.
    if not docs_root.is_dir():
        raise TreeUnreadable(f"{docs_root} does not exist")
    if not report.pages:
        raise TreeUnreadable(f"{docs_root} contains no markdown pages")
    index = docs_root / "index.md"
    if not index.is_file():
        raise TreeUnreadable(
            f"{index} is missing — it is the root every reachability answer is measured from, "
            f"so without it every page would report as an orphan"
        )

    resolved_root = docs_root.resolve()
    for page in report.pages:
        for raw in page_links(page):
            target = raw.split("#", 1)[0].strip()
            if not target or target.startswith(EXTERNAL_PREFIXES):
                continue
            candidate = page.parent / target
            if not candidate.exists():
                report.dangling.append((page, raw))
                continue
            absolute = candidate.resolve()
            if resolved_root != absolute and resolved_root not in absolute.parents:
                report.escaping.append((page, raw))

    if not page_links(index):
        raise TreeUnreadable(
            f"{index} yielded no links at all. Either it is a stub or the link parser no "
            f"longer matches this tree's markdown — reporting every page as an orphan would "
            f"be a confident answer derived from a broken parse"
        )

    _walk_reach(report, index, resolved_root)
    _inspect_reference_tree(report)
    return report


def _walk_reach(report: Report, index: Path, resolved_root: Path) -> None:
    """Breadth-first from index.md, remembering which door each page came through.

    Reach is transitive on purpose. A guide linked from a door's own child page is
    genuinely reachable, and a direct-links-only measure would report most of the
    tree as orphaned while a reader navigates it without difficulty.
    """
    start = index.resolve()
    report.reached_via[start] = {"index"}
    queue: deque[tuple[Path, set[str]]] = deque([(index, {"index"})])
    seen = {start}

    while queue:
        page, doors = queue.popleft()
        for raw in page_links(page):
            target = raw.split("#", 1)[0].strip()
            if not target or target.startswith(EXTERNAL_PREFIXES):
                continue
            candidate = page.parent / target
            if not candidate.is_file() or candidate.suffix != ".md":
                continue
            absolute = candidate.resolve()
            if resolved_root != absolute and resolved_root not in absolute.parents:
                continue
            # A page directly under docs/doors/ names the route; everything it
            # reaches inherits that name.
            inherited = doors
            if candidate.parent.name == "doors":
                inherited = {candidate.stem}
            if absolute in seen:
                report.reached_via.setdefault(absolute, set()).update(inherited)
                continue
            seen.add(absolute)
            report.reached_via[absolute] = set(inherited)
            queue.append((candidate, inherited))

    report.orphans = [p for p in report.pages if p.resolve() not in seen]


def _inspect_reference_tree(report: Report) -> None:
    reference = report.docs_root / "reference"
    if not reference.is_dir():
        return
    for path in sorted(reference.rglob("*")):
        if not path.is_file():
            continue
        relative = path.relative_to(reference)
        subtree = relative.parts[0] if len(relative.parts) > 1 else relative.name
        if subtree not in CLAIMED_REFERENCE_SUBTREES:
            report.unclaimed_reference.append(path)
            continue
        if path.suffix == ".md" and not GENERATED_MARKER_RE.search(path.read_text()):
            report.unmarked_reference.append(path)


def check_publish(report: Report, timeout: float) -> None:
    """Compare the committed tree against what the site actually serves.

    A FILE-level check only. `llms.txt` lists published pages and their folder,
    which is enough to say "this page did not land" — and says nothing about the
    navigation tree, which is where the known problem lives. See the module
    docstring.
    """
    try:
        with urllib.request.urlopen(LLMS_TXT_URL, timeout=timeout) as response:
            body = response.read().decode("utf-8", "replace")
    except (urllib.error.URLError, OSError, TimeoutError) as exc:
        # Never a failure. The site being unreachable says nothing about the tree,
        # and a blocking gate that depends on a third party's uptime is a gate
        # people learn to re-run until it passes.
        report.publish = {
            "status": "unknown",
            "detail": f"could not fetch {LLMS_TXT_URL}: {exc}",
        }
        return

    published: set[str] = set()
    for line in body.splitlines():
        match = re.match(r"^-\s+(?:(\S+)\s+)?\[([^\]]*)\]\(([^)]+)\)", line)
        if match:
            folder = match.group(1) or ""
            published.add(f"{folder}/{match.group(2)}" if folder else match.group(2))

    if not published:
        report.publish = {
            "status": "unknown",
            "detail": f"fetched {LLMS_TXT_URL} but parsed no entries from it",
        }
        return

    # Match on FOLDER, not on title: a page's committed filename and its published
    # title are different strings (`self-hosting.md` publishes as "Self-Hosting
    # Temper"), so only the folder is comparable without a mapping nothing owns.
    published_folders: dict[str, int] = {}
    for entry in published:
        folder = entry.split("/", 1)[0] if "/" in entry else ""
        published_folders[folder] = published_folders.get(folder, 0) + 1

    committed_folders: dict[str, int] = {}
    for page in report.pages:
        relative = page.relative_to(report.docs_root)
        folder = relative.parts[0] if len(relative.parts) > 1 else ""
        committed_folders[folder] = committed_folders.get(folder, 0) + 1

    report.publish = {
        "status": "compared",
        "published_total": len(published),
        "published_folders": published_folders,
        "committed_folders": committed_folders,
    }


def print_report(report: Report, strict: bool) -> None:
    print("── Reach " + "─" * 71)
    print("  Which door routes to each page, walking transitively from docs/index.md.")
    print("  An orphan is NOT a defect and never affects the exit code: a page may be")
    print("  reached from the site's own navigation rather than from a door.")
    print()
    by_door: dict[str, int] = {}
    for doors in report.reached_via.values():
        for door in doors:
            by_door[door] = by_door.get(door, 0) + 1
    for door, count in sorted(by_door.items()):
        print(f"  {count:4d} page(s) reachable via {door}")
    print(f"  {len(report.reached_via)}/{len(report.pages)} pages reachable from index.md")
    print()
    if report.orphans:
        print(f"  {len(report.orphans)} ORPHAN page(s) — present in docs/, no route from index.md:")
        for page in report.orphans:
            print(f"    ORPHAN  {page}")
    else:
        print("  no orphans")
    print()

    print("── Dangling links " + "─" * 62)
    print("  A link whose target does not exist. The ONLY --strict failure, because it is")
    print("  the only category here that is a plain fact rather than a judgement.")
    print()
    if report.dangling:
        for page, raw in report.dangling:
            print(f"  DANGLING  {page} -> {raw}")
    else:
        print("  none")
    print()

    print("── Links that escape the published tree " + "─" * 40)
    print("  These resolve on disk and are DEAD on the published site: Apidog publishes")
    print("  docs/** and nothing else, so a link to ../../DEPLOYING.md reaches nothing.")
    print("  Reported, never adjudicated — this set predates the check, and a gate that")
    print("  fails on arrival is one people learn to skip.")
    print()
    if report.escaping:
        for page, raw in report.escaping:
            print(f"  ESCAPES  {page} -> {raw}")
        print(f"  {len(report.escaping)} escaping link(s)")
    else:
        print("  none")
    print()

    print("── Generated reference tree " + "─" * 52)
    print("  Every path under docs/reference/ should belong to a subtree some generator")
    print("  owns, and every page should carry its GENERATED marker. A subtree nothing")
    print("  claims is content no drift gate regenerates or checks.")
    print()
    for subtree, owner in sorted(CLAIMED_REFERENCE_SUBTREES.items()):
        print(f"  claimed: docs/reference/{subtree}/  <- {owner}")
    if report.unclaimed_reference:
        for path in report.unclaimed_reference:
            print(f"  UNCLAIMED  {path} — no generator owns this")
    if report.unmarked_reference:
        for path in report.unmarked_reference:
            print(f"  UNMARKED   {path} — inside a generated subtree without a GENERATED marker")
    if not report.unclaimed_reference and not report.unmarked_reference:
        print("  every file under docs/reference/ is claimed and marked")
    print()

    print("── Publish coverage " + "─" * 60)
    publish = report.publish
    if publish["status"] != "compared":
        print(f"  UNKNOWN — {publish['detail']}")
        print("  Not counted clean. Whether the committed tree reached the site is simply")
        print("  not established by this run.")
    else:
        print(f"  {publish['published_total']} page(s) currently published, by folder:")
        for folder, count in sorted(publish["published_folders"].items()):
            print(f"    {count:4d}  {folder or '(root)'}")
        print("  committed, by folder:")
        for folder, count in sorted(publish["committed_folders"].items()):
            print(f"    {count:4d}  {folder or '(root)'}")
        print()
        print("  Compared against PRODUCTION, which serves the last publish of `main`. On a")
        print("  branch, pages the branch adds legitimately read as unpublished. Differences")
        print("  are informational and never affect the exit code.")
    print()

    print("── Navigation " + "─" * 66)
    print("  UNKNOWN, and structurally so. Apidog reconciles pages but leaves empty folder")
    print("  nodes behind, and orders the sidebar itself; neither is observable from here.")
    print("  Empty folders contribute no lines to llms.txt, and Apidog's public API has no")
    print("  endpoint for navigation nodes or ordering — its whole surface is four")
    print("  operations, and twenty plausible route names all returned redirect-to-help.")
    print("  Pruning and ordering are manual actions in the Apidog UI.")
    print()
    print("  A green run of this script does NOT mean the site navigates correctly.")
    print()

    if strict:
        print(
            f"--strict: {len(report.dangling)} dangling link(s) counted. "
            f"Orphans ({len(report.orphans)}), escaping links ({len(report.escaping)}), "
            f"unclaimed/unmarked reference files "
            f"({len(report.unclaimed_reference) + len(report.unmarked_reference)}) and the "
            f"unknown navigation state are reported and NOT counted."
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--docs", type=Path, default=Path("docs"))
    parser.add_argument(
        "--strict",
        action="store_true",
        help="exit non-zero on dangling links, and ONLY on dangling links. Orphans, links that "
        "escape the published tree, unclaimed reference subtrees and the unknown navigation "
        "state are all reported and never counted. Said explicitly because --strict reads like "
        "'fail on everything', and it deliberately does not.",
    )
    parser.add_argument(
        "--no-network",
        action="store_true",
        help="skip the llms.txt publish check. It is reported as UNKNOWN either way, so this "
        "only saves the round trip; it never turns an unknown into a clean.",
    )
    parser.add_argument("--timeout", type=float, default=10.0)
    parser.add_argument("--json", action="store_true", help="emit the report as JSON")
    args = parser.parse_args()

    try:
        report = collect(args.docs)
    except TreeUnreadable as exc:
        print(f"ERROR: {exc}.", file=sys.stderr)
        print(
            "       Refusing to report coverage over a tree this script cannot read — every\n"
            "       count would be a confident zero derived from having looked at nothing.",
            file=sys.stderr,
        )
        return 2

    if args.no_network:
        report.publish = {"status": "unknown", "detail": "skipped (--no-network)"}
    else:
        check_publish(report, args.timeout)

    if args.json:
        print(
            json.dumps(
                {
                    "pages": len(report.pages),
                    "reachable": len(report.reached_via),
                    "orphans": [str(p) for p in report.orphans],
                    "dangling": [{"page": str(p), "link": raw} for p, raw in report.dangling],
                    "escaping": [{"page": str(p), "link": raw} for p, raw in report.escaping],
                    "unclaimed_reference": [str(p) for p in report.unclaimed_reference],
                    "unmarked_reference": [str(p) for p in report.unmarked_reference],
                    "publish": report.publish,
                    "navigation": "unknown: not observable from llms.txt, and Apidog's API "
                    "exposes no navigation endpoint",
                },
                indent=2,
            )
        )
    else:
        print_report(report, args.strict)

    if args.strict and report.dangling:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
