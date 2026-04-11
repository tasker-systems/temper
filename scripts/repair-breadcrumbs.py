#!/usr/bin/env python3
"""Repair vault files damaged by the reconstitution breadcrumb bug.

The bug replaced markdown headings with breadcrumb-style text:
  "## Decision" became "Decision"
  "### Rationale" became "Decision > Rationale"

This script detects breadcrumb lines and restores them as markdown headings.
A line is considered a breadcrumb if it:
  1. Stands alone (preceded and followed by blank lines)
  2. Matches the pattern: Title-cased segments joined by " > "
  3. Is short (< 120 chars) — headings, not prose

Usage:
  python3 scripts/repair-breadcrumbs.py ~/projects/kb-vault  # dry run
  python3 scripts/repair-breadcrumbs.py ~/projects/kb-vault --fix
"""

import re
import sys
from pathlib import Path

# Known top-level section headings in temper session/research docs.
# These appear as bare words on their own line and should be ## headings.
KNOWN_H2 = {
    "Goal", "What Happened", "Decisions", "Connections", "Next Steps",
    "Acceptance Criteria", "Summary", "Implementation", "Context",
    "Related", "Background", "Scope", "Design", "Architecture",
    "Observations", "Reproduction", "References",
}

BREADCRUMB_RE = re.compile(r'^([A-Z][^>\n]+?)( > [A-Z][^>\n]+)+$')
STANDALONE_HEADING_RE = re.compile(r'^[A-Z][A-Za-z ]+$')


def is_breadcrumb_line(line: str, prev_line: str, next_line: str) -> bool:
    """Detect if a line is a breadcrumb that replaced a heading."""
    stripped = line.strip()
    if not stripped or len(stripped) > 120:
        return False
    # Must be surrounded by blank lines (heading context)
    if prev_line.strip() or next_line.strip():
        return False
    return bool(BREADCRUMB_RE.match(stripped))


def is_in_code_block(lines: list[str], index: int) -> bool:
    """Check if a line is inside a fenced code block."""
    fence_count = 0
    for i in range(index):
        if lines[i].strip().startswith("```"):
            fence_count += 1
    return fence_count % 2 == 1


def is_standalone_heading(line: str, prev_line: str, next_line: str) -> bool:
    """Detect a bare title-cased line that should be a ## heading."""
    stripped = line.strip()
    if not stripped or len(stripped) > 80:
        return False
    if prev_line.strip() or next_line.strip():
        return False
    if not STANDALONE_HEADING_RE.match(stripped):
        return False
    # Only match known session/research headings to avoid false positives
    return stripped in KNOWN_H2


def repair_line(line: str) -> str:
    """Convert a breadcrumb line to its proper markdown heading."""
    stripped = line.strip()
    parts = [p.strip() for p in stripped.split(" > ")]
    depth = len(parts) + 1  # +1 because these are sub-headings under ##
    depth = min(depth, 6)
    title = parts[-1]  # innermost heading
    return f"{'#' * depth} {title}"


def repair_standalone(line: str) -> str:
    """Convert a standalone heading to ## heading."""
    return f"## {line.strip()}"


def process_file(path: Path, fix: bool = False) -> list[str]:
    """Process a single file, returning list of changes found."""
    content = path.read_text()
    lines = content.split('\n')
    changes = []
    new_lines = []

    for i, line in enumerate(lines):
        prev_line = lines[i - 1] if i > 0 else ""
        next_line = lines[i + 1] if i < len(lines) - 1 else ""

        # Skip lines inside fenced code blocks
        if is_in_code_block(lines, i):
            new_lines.append(line)
            continue

        if is_breadcrumb_line(line, prev_line, next_line):
            repaired = repair_line(line)
            changes.append(f"  L{i+1}: {line.strip()!r} -> {repaired!r}")
            new_lines.append(repaired)
        elif is_standalone_heading(line, prev_line, next_line):
            repaired = repair_standalone(line)
            changes.append(f"  L{i+1}: {line.strip()!r} -> {repaired!r}")
            new_lines.append(repaired)
        else:
            new_lines.append(line)

    if fix and changes:
        path.write_text('\n'.join(new_lines))

    return changes


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)

    vault_root = Path(sys.argv[1])
    fix = "--fix" in sys.argv

    if not vault_root.is_dir():
        print(f"Error: {vault_root} is not a directory")
        sys.exit(1)

    total_changes = 0
    affected_files = 0

    for md_file in sorted(vault_root.rglob("*.md")):
        # Skip .temper directory
        if ".temper" in md_file.parts:
            continue
        changes = process_file(md_file, fix=fix)
        if changes:
            affected_files += 1
            total_changes += len(changes)
            rel = md_file.relative_to(vault_root)
            print(f"\n{'FIXED' if fix else 'WOULD FIX'}: {rel} ({len(changes)} headings)")
            for c in changes:
                print(c)

    print(f"\n{'Fixed' if fix else 'Found'}: {total_changes} breadcrumb headings in {affected_files} files")
    if not fix and total_changes > 0:
        print("Run with --fix to apply repairs")


if __name__ == "__main__":
    main()
