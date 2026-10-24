#!/usr/bin/env python3
"""manifest.py — the one reader for temper's hand-authored manifests.

WHY THIS EXISTS
---------------
The release register used to be one markdown file, RELEASE_REGISTER.md, read by three
independent parsers with three different tolerances (the PR gate's awk, the release
calculator's bash `case`, the pin check's title awk) and written by a fourth piece of awk.
Every wire PR prepended its row at one shared spot, so two wire PRs conflicted. The spec of
record is temper-artifacts:specs/2026-10-11-release-register-entries-design.md; the survey
behind it is temper-artifacts:spikes/2026-10-11-structured-manifests-and-their-parsers.md.

Now each register entry is its own TOML file under release-register/, and every reader goes
through this script. It is standard library only, so it runs in the toolchain-free guard job,
in the pre-push hook and in the release scripts alike.

WHAT IT OWNS, AND WHAT IT DOES NOT
---------------------------------
It owns everything that can be said about ONE entry, or about the register as a set:
  * the file layout (entry directories, ids, stray files);
  * the six required keys and their types;
  * membership in the vocabulary, which is DATA, read from release-register/vocabulary.toml.
    No reader holds its own copy. A token added there is accepted everywhere at once;
  * the three entry-level rules that are semantics, not vocabulary:
      - `retirement-train` rides only a `shape-breaking` entry (D-C2);
      - `spec-correction` never rides with `shape-breaking`;
      - `blocked:<release-class>` names its class (free text, never tokenized);
  * id uniqueness across `unreleased/` and every rolled `v<version>/` directory.

It does NOT judge an entry against a diff. Whether a PR declared at all, and whether its
classes survive its own openapi.json movement, stay in wire-class-crosscheck.sh. The release
gates (retirement-train refuses a patch, blocked rows refuse a client release) stay in
calculate-versions.sh. This script hands both of them rows they can trust.

USAGE
  manifest.py register validate [--dir D]
  manifest.py register rows     [--dir D] [--window unreleased|v<x.y.z>] [--files F ...]
  manifest.py register resolve  <id> [--dir D]
  manifest.py register new      <id> [--dir D]
  manifest.py register render   [--dir D] [--window unreleased|v<x.y.z>]

`rows` prints one TAB-separated line per entry, sorted by id:
  pr <TAB> classes <TAB> surfaces <TAB> status <TAB> title <TAB> id
The first five columns are exactly the stream the old register_window_rows printed, so the bash
downstream of it reads the same thing. Every field is non-empty and none can hold a TAB or a
newline, so `IFS=$'\\t' read` (which collapses an empty field) cannot shift a column.

Exit 0 = success. Exit 1 = the register (or an entry) is malformed, or a lookup failed; every
problem is printed as `<path>: <key>: <message>`. Exit 2 = a usage error.
"""

import sys

# The floor comes first, before anything that needs it: `tomllib` is 3.11+, and an ImportError
# naming a module tells the reader nothing about what to install.
PYTHON_FLOOR = (3, 11)
if sys.version_info < PYTHON_FLOOR:
    sys.stderr.write(
        "manifest.py: needs Python {}.{} or newer (for the standard library's tomllib); "
        "this is {}.{}.\n"
        "  Install a newer python3 and put it first on PATH.\n".format(
            *PYTHON_FLOOR, *sys.version_info[:2]
        )
    )
    sys.exit(1)

import argparse  # noqa: E402
import re  # noqa: E402
import shutil  # noqa: E402
import tomllib  # noqa: E402
from pathlib import Path  # noqa: E402

DEFAULT_REGISTER_DIR = Path(__file__).resolve().parents[2] / "release-register"

ENTRY_KEYS = ("title", "pr", "classes", "surfaces", "status", "body")
VOCABULARY_KEYS = ("classes", "surfaces", "status", "pr_sentinels")
UNRELEASED = "unreleased"
ROLLED_WINDOW = re.compile(r"^v[0-9]+\.[0-9]+\.[0-9]+$")
ENTRY_ID = re.compile(r"^[a-z0-9][a-z0-9-]{0,63}$")
# Files the register directory may hold besides its entry directories.
REGISTER_FILES = {"README.md", "HISTORY.md", "vocabulary.toml", "_template.toml"}


class Problems:
    """Collects `<path>: <key>: <message>` lines; a register is valid only when this stays empty."""

    def __init__(self):
        self.lines = []

    def add(self, path, key, message):
        self.lines.append(f"{_display(path)}: {key}: {message}")

    def __bool__(self):
        return bool(self.lines)


def _display(path):
    try:
        return str(Path(path).resolve().relative_to(Path.cwd().resolve()))
    except ValueError:
        return str(path)


# ── Vocabulary ──────────────────────────────────────────────────────────────────────────────────


def load_vocabulary(register_dir, problems):
    path = register_dir / "vocabulary.toml"
    if not path.is_file():
        problems.add(
            path,
            "vocabulary",
            "missing: the register's vocabulary is read from this file, never from code",
        )
        return None
    try:
        vocab = tomllib.loads(path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as exc:
        problems.add(path, "toml", str(exc))
        return None
    ok = True
    for key in VOCABULARY_KEYS:
        value = vocab.get(key)
        if (
            not isinstance(value, list)
            or not value
            or not all(isinstance(t, str) and t for t in value)
        ):
            problems.add(path, key, "must be a non-empty array of non-empty strings")
            ok = False
    for key in sorted(set(vocab) - set(VOCABULARY_KEYS)):
        problems.add(
            path, key, "is not a vocabulary key (expected: {})".format(", ".join(VOCABULARY_KEYS))
        )
        ok = False
    return vocab if ok else None


# ── One entry ───────────────────────────────────────────────────────────────────────────────────


def check_entry(path, vocab, problems):
    """Parse and validate one entry file. Returns the entry dict, or None when it is malformed."""
    before = len(problems.lines)
    entry_id = path.stem
    if not ENTRY_ID.match(entry_id):
        problems.add(
            path,
            "id",
            f"'{entry_id}' — an entry id (the file stem) is lowercase kebab-case, "
            "[a-z0-9][a-z0-9-]*, at most 64 characters",
        )
    try:
        data = tomllib.loads(path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as exc:
        problems.add(path, "toml", str(exc))
        return None
    except UnicodeDecodeError as exc:
        problems.add(path, "toml", f"not UTF-8: {exc}")
        return None

    for key in ENTRY_KEYS:
        if key not in data:
            problems.add(
                path, key, "missing — every entry carries all of: {}".format(", ".join(ENTRY_KEYS))
            )
    for key in sorted(set(data) - set(ENTRY_KEYS)):
        problems.add(path, key, "is not an entry key (expected: {})".format(", ".join(ENTRY_KEYS)))

    title = data.get("title")
    if "title" in data:
        if not isinstance(title, str) or not title.strip():
            problems.add(path, "title", "must be a non-empty string")
        elif "\t" in title or "\n" in title or "\r" in title:
            problems.add(
                path, "title", "must be one line with no TAB — it is a column in the rows stream"
            )

    pr = data.get("pr")
    # bool is an int subclass; `pr = true` is a typo, not PR #1.
    if "pr" in data and (
        isinstance(pr, bool)
        or not (
            (isinstance(pr, int) and pr > 0)
            or (isinstance(pr, str) and pr in vocab["pr_sentinels"])
        )
    ):
        problems.add(
            path,
            "pr",
            "{!r} — expected a positive PR number or one of: {}".format(
                pr, ", ".join(vocab["pr_sentinels"])
            ),
        )

    for key in ("classes", "surfaces"):
        if key not in data:
            continue
        value = data[key]
        if not isinstance(value, list) or not value or not all(isinstance(t, str) for t in value):
            problems.add(
                path, key, "must be a non-empty array of strings — silence is not a classification"
            )
            continue
        for token in value:
            if token not in vocab[key]:
                problems.add(
                    path, key, "'{}' is not one of: {}".format(token, ", ".join(vocab[key]))
                )
        for token in sorted({t for t in value if value.count(t) > 1}):
            problems.add(path, key, f"'{token}' is listed more than once")

    classes = data.get("classes") if isinstance(data.get("classes"), list) else []
    if "retirement-train" in classes and "shape-breaking" not in classes:
        problems.add(
            path,
            "classes",
            "'retirement-train' is a routing, never a class on its own — "
            "it rides a shape-breaking entry (D-C2)",
        )
    if "spec-correction" in classes and "shape-breaking" in classes:
        problems.add(
            path,
            "classes",
            "'spec-correction' and 'shape-breaking' contradict — a correction brings "
            "the contract to what the server always did; a break changes what it does",
        )

    status = data.get("status")
    if "status" in data:
        if not isinstance(status, str):
            problems.add(path, "status", "must be a string")
        elif status.startswith("blocked:"):
            blocked = status[len("blocked:") :]
            if not blocked.strip():
                problems.add(
                    path,
                    "status",
                    "'blocked:' names no release class — everything after "
                    "'blocked:' is the class name, free text, never tokenized",
                )
            elif "\t" in blocked or "\n" in blocked:
                problems.add(
                    path, "status", "the blocked release class must be one line with no TAB"
                )
        elif status not in vocab["status"]:
            problems.add(
                path,
                "status",
                "'{}' — expected one of: {}, or blocked:<release-class>".format(
                    status, ", ".join(vocab["status"])
                ),
            )

    body = data.get("body")
    if "body" in data and (not isinstance(body, str) or not body.strip()):
        problems.add(
            path,
            "body",
            "must be a non-empty string — what changed, who observes it, and why it matters",
        )

    if len(problems.lines) != before:
        return None
    return {
        "id": entry_id,
        "path": path,
        "title": title,
        "pr": str(pr),
        "classes": classes,
        "surfaces": data["surfaces"],
        "status": status,
        "body": body,
    }


# ── The register as a set ───────────────────────────────────────────────────────────────────────


def window_dirs(register_dir):
    """The entry directories: `unreleased/` first, then every rolled `v<x.y.z>/`, by name."""
    dirs = []
    if (register_dir / UNRELEASED).is_dir():
        dirs.append(register_dir / UNRELEASED)
    dirs.extend(
        sorted(p for p in register_dir.iterdir() if p.is_dir() and ROLLED_WINDOW.match(p.name))
    )
    return dirs


def load_register(register_dir, problems):
    """Validate the whole register. Returns ({window: [entries]}, vocab); entries only when clean."""
    if not register_dir.is_dir():
        problems.add(
            register_dir,
            "register",
            "no such directory — a missing register is an error, never zero entries",
        )
        return {}, None
    vocab = load_vocabulary(register_dir, problems)
    if not (register_dir / UNRELEASED).is_dir():
        problems.add(
            register_dir / UNRELEASED,
            "register",
            "missing — the open window is always present, even empty",
        )

    for child in sorted(register_dir.iterdir()):
        if child.name.startswith("."):
            continue
        if child.is_dir():
            if child.name != UNRELEASED and not ROLLED_WINDOW.match(child.name):
                problems.add(
                    child, "layout", "is not an entry directory (expected unreleased/ or v<x.y.z>/)"
                )
        elif child.name not in REGISTER_FILES:
            problems.add(
                child, "layout", "is not a register file — entries live in unreleased/<id>.toml"
            )

    if vocab is None:
        return {}, None

    windows = {}
    seen = {}
    for wdir in window_dirs(register_dir):
        entries = []
        for path in sorted(wdir.iterdir()):
            if path.name.startswith(".") or path.name.startswith("_"):
                continue
            if path.is_dir() or path.suffix != ".toml":
                problems.add(path, "layout", "is not an entry — entries are <id>.toml files")
                continue
            if path.stem in seen:
                problems.add(
                    path,
                    "id",
                    f"'{path.stem}' is already the id of {_display(seen[path.stem])} — an id is unique across every window",
                )
            seen.setdefault(path.stem, path)
            entry = check_entry(path, vocab, problems)
            if entry is not None:
                entries.append(entry)
        windows[wdir.name] = entries
    return windows, vocab


def _fail(problems, what):
    # stderr: `rows` is redirected into a file by its callers, and a problem written there would
    # vanish with the rows it refused to produce.
    err = sys.stderr
    err.write(f"manifest.py: {what}\n\n")
    for line in problems.lines:
        err.write(f"  {line}\n")
    err.write(
        "\nThe vocabulary is release-register/vocabulary.toml; the contract is release-register/README.md.\n"
    )
    return 1


def _row(entry):
    return "\t".join(
        [
            entry["pr"],
            ", ".join(entry["classes"]),
            ", ".join(entry["surfaces"]),
            entry["status"],
            entry["title"],
            entry["id"],
        ]
    )


# ── Commands ────────────────────────────────────────────────────────────────────────────────────


def cmd_validate(args):
    problems = Problems()
    windows, _ = load_register(args.dir, problems)
    if problems:
        return _fail(problems, f"the register ({_display(args.dir)}) is malformed.")
    count = sum(len(e) for e in windows.values())
    unreleased = len(windows.get(UNRELEASED, []))
    sys.stdout.write(f"manifest.py: register OK — {count} entries, {unreleased} in unreleased/.\n")
    return 0


def cmd_rows(args):
    problems = Problems()
    if args.files:
        # The PR gate's seam: just these entries (the ones a diff adds), validated against the
        # register's vocabulary. The gate runs `validate` over the whole register separately.
        vocab = load_vocabulary(args.dir, problems)
        entries = []
        if vocab is not None:
            for name in args.files:
                path = Path(name)
                if not path.is_file():
                    problems.add(path, "entry", "no such file")
                    continue
                entry = check_entry(path, vocab, problems)
                if entry is not None:
                    entries.append(entry)
        if problems:
            return _fail(problems, "an entry is malformed.")
    else:
        windows, _ = load_register(args.dir, problems)
        if problems:
            return _fail(problems, f"the register ({_display(args.dir)}) is malformed.")
        if args.window not in windows:
            problems.add(args.dir / args.window, "window", "no such window")
            return _fail(problems, f"no window '{args.window}'.")
        entries = windows[args.window]
    for entry in sorted(entries, key=lambda e: e["id"]):
        sys.stdout.write(_row(entry) + "\n")
    return 0


def cmd_resolve(args):
    problems = Problems()
    windows, _ = load_register(args.dir, problems)
    if problems:
        return _fail(problems, f"the register ({_display(args.dir)}) is malformed.")
    for entries in windows.values():
        for entry in entries:
            if entry["id"] == args.id:
                sys.stdout.write(
                    "{}\t{}\n".format(_display(entry["path"]), ", ".join(entry["classes"]))
                )
                return 0
    sys.stderr.write(
        f"manifest.py: no register entry has the id '{args.id}' (searched unreleased/ and every v<x.y.z>/).\n"
    )
    return 1


def cmd_new(args):
    problems = Problems()
    if not ENTRY_ID.match(args.id):
        sys.stderr.write(
            f"manifest.py: '{args.id}' is not an entry id — lowercase kebab-case, [a-z0-9][a-z0-9-]*, "
            "at most 64 characters.\n"
        )
        return 1
    template = args.dir / "_template.toml"
    if not template.is_file():
        problems.add(template, "template", "missing")
        return _fail(problems, "cannot scaffold an entry.")
    for wdir in window_dirs(args.dir):
        existing = wdir / (args.id + ".toml")
        if existing.exists():
            sys.stderr.write(
                f"manifest.py: the id '{args.id}' is taken by {_display(existing)} — choose another.\n"
            )
            return 1
    target = args.dir / UNRELEASED / (args.id + ".toml")
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(template, target)
    sys.stdout.write(f"{_display(target)}\n")
    return 0


def cmd_render(args):
    problems = Problems()
    windows, _ = load_register(args.dir, problems)
    if problems:
        return _fail(problems, f"the register ({_display(args.dir)}) is malformed.")
    if args.window not in windows:
        sys.stderr.write(f"manifest.py: no window '{args.window}'.\n")
        return 1
    heading = "Unreleased" if args.window == UNRELEASED else f"Shipped in {args.window}"
    out = [f"# Release register — {heading}", ""]
    for entry in sorted(windows[args.window], key=lambda e: e["id"]):
        out.append("- **{}**".format(entry["title"]))
        for line in entry["body"].strip("\n").split("\n"):
            out.append(("  " + line) if line else "")
        out.append(
            "  `{}` · pr: {} · classes: {} · surfaces: {} · status: {}".format(
                entry["id"],
                entry["pr"],
                ", ".join(entry["classes"]),
                ", ".join(entry["surfaces"]),
                entry["status"],
            )
        )
        out.append("")
    sys.stdout.write("\n".join(out))
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="manifest.py", description="The one reader for temper's hand-authored manifests."
    )
    families = parser.add_subparsers(dest="family", required=True)

    register = families.add_parser("register", help="the release register (release-register/)")
    commands = register.add_subparsers(dest="command", required=True)

    def with_dir(p):
        p.add_argument(
            "--dir",
            type=Path,
            default=DEFAULT_REGISTER_DIR,
            help="the register directory (default: %(default)s)",
        )
        return p

    with_dir(
        commands.add_parser("validate", help="validate every entry, the layout and the vocabulary")
    ).set_defaults(fn=cmd_validate)

    rows = with_dir(
        commands.add_parser("rows", help="print a window's entries as TAB-separated rows")
    )
    rows.add_argument("--window", default=UNRELEASED, help="unreleased (default) or v<x.y.z>")
    rows.add_argument(
        "--files", nargs="+", help="read exactly these entry files instead of a window"
    )
    rows.set_defaults(fn=cmd_rows)

    resolve = with_dir(
        commands.add_parser("resolve", help="find an entry by id across every window")
    )
    resolve.add_argument("id")
    resolve.set_defaults(fn=cmd_resolve)

    new = with_dir(
        commands.add_parser("new", help="scaffold unreleased/<id>.toml from the template")
    )
    new.add_argument("id")
    new.set_defaults(fn=cmd_new)

    render = with_dir(
        commands.add_parser("render", help="print a window as markdown (never commit the output)")
    )
    render.add_argument("--window", default=UNRELEASED, help="unreleased (default) or v<x.y.z>")
    render.set_defaults(fn=cmd_render)

    args = parser.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
