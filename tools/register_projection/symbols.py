"""Resolve a witness token against this repository.

A register cites its evidence by name. This answers one question about each citation — **does the
thing it names exist here** — and deliberately not the harder one, whether it *bites*. Existence is
the weaker check and it already finds real defects; force is the register's own standing remainder.

## The search domain is part of the answer, and it is recorded for that reason

`[measured — 2026-08-17]` A first pass indexed `crates/` only and reported **seven** unresolved
witnesses on one register. Four of them were live tests in `tests/e2e/`. Those four were not stale
citations; they were the instrument's blind spot presented as findings — and a tool that reports
false defects is one people stop running, which is the failure mode that matters most here. The true
count was three.

So the domain is repo-wide, and `SymbolIndex.domain` carries what was actually searched into the
artifact. A reader who sees `unresolved` must be able to check whether the name was merely somewhere
this never looked.
"""

from __future__ import annotations

import re
import subprocess
from dataclasses import dataclass

# `fn name` / `async fn name`. Covers `#[test]`, `#[tokio::test]` and `#[sqlx::test]` alike, because
# what is being asked is only whether a function of that name exists — not whether it is a test. A
# register citing a non-test function as its witness is making a claim this tool does not adjudicate.
_FN_RE = re.compile(r"(?:async\s+)?fn\s+([a-z_][A-Za-z0-9_]*)")

_EXCLUDED = ("target", "node_modules", ".git", ".venv", "__pycache__", "dist", "public")

# ── Token shape ─────────────────────────────────────────────────────────────────────────────────
#
# A witness cell is prose with backticks in it, not a list of names. Alongside real citations it
# carries type names, env vars, migration numbers and product names, all backticked. `[measured —
# 2026-08-17]` On a first pass those became "unresolved witnesses": `ApiError`, `temper`,
# `temper-client`, `VERCEL_GIT_COMMIT_SHA` and `20260730000010` were all reported as missing
# evidence. They are not evidence at all, and reporting them as defects is how a tool earns the
# reputation that stops it being run.
#
# So shape is classified BEFORE resolution, and a token that is not witness-shaped is never counted
# as an unresolved witness. It is not discarded either — it is carried into the artifact under its
# own key, because a token this could not classify is exactly the thing that must not vanish
# silently.
_PATH_EXTENSIONS = (".rs", ".sh", ".yml", ".yaml", ".py", ".ts", ".js", ".sql", ".toml", ".md")

NOT_WITNESS_SHAPE = "not_witness_shape"


def classify(token: str) -> str:
    """"function", "path", or NOT_WITNESS_SHAPE. Conservative by design.

    A **path** contains a separator or ends in a known source extension.
    A **function** is lowercase snake_case: it must contain an underscore, and must not carry an
    uppercase letter (a type name) or be all digits (a migration number). A single lowercase word or
    a kebab-case product name is neither.
    """
    if "/" in token or token.endswith(_PATH_EXTENSIONS):
        return "path"
    if token.isdigit():
        return NOT_WITNESS_SHAPE
    if "_" in token and token == token.lower() and re.fullmatch(r"[a-z0-9_]+", token):
        return "function"
    return NOT_WITNESS_SHAPE


@dataclass(frozen=True)
class Resolution:
    token: str
    kind: str  # "function" | "path" | NOT_WITNESS_SHAPE
    resolved: bool
    found_in: str | None = None
    matched_by: str | None = None  # "exact" | "basename" — how strong the match is


@dataclass
class SymbolIndex:
    functions: dict[str, str]  # name -> first file it was seen in
    paths: frozenset[str]
    basenames: dict[str, list[str]]  # basename -> every path carrying it
    domain: list[str]  # what was searched, verbatim, for the artifact to carry

    def resolve(self, token: str) -> Resolution:
        kind = classify(token)
        if kind is NOT_WITNESS_SHAPE:
            return Resolution(token, NOT_WITNESS_SHAPE, False)
        if kind == "path":
            if token in self.paths:
                return Resolution(token, "path", True, token, "exact")
            # **Basename fallback, and it is not laxity.** Registers routinely cite a script by its
            # filename alone — `sqlx-wire-diff.sh` — or by a path that was accurate before a move.
            # `[measured — 2026-08-17]` Four such citations were reported as missing evidence while
            # the files sat in `.github/scripts/`. A basename hit is a WEAKER claim than an exact
            # one and says so, so a reader can tell "the file moved" from "the file is here".
            hits = self.basenames.get(token.rsplit("/", 1)[-1], [])
            if len(hits) == 1:
                return Resolution(token, "path", True, hits[0], "basename")
            if len(hits) > 1:
                return Resolution(token, "path", True, f"{len(hits)} candidates", "basename")
            return Resolution(token, "path", False)
        where = self.functions.get(token)
        return Resolution(token, "function", where is not None, where, "exact" if where else None)


def _run(args: list[str], cwd: str) -> str:
    proc = subprocess.run(args, cwd=cwd, capture_output=True, text=True)
    if proc.returncode not in (0, 1):  # rg exits 1 on "no matches", which is not an error
        raise SystemExit(f"{' '.join(args[:2])} failed ({proc.returncode}): {proc.stderr.strip()}")
    return proc.stdout


def build_index(repo_root: str) -> SymbolIndex:
    """Index every Rust function name and every tracked file path in the repository."""
    excludes: list[str] = []
    for name in _EXCLUDED:
        excludes += ["-g", f"!{name}"]

    fn_out = _run(
        ["rg", "--no-ignore-vcs", "--with-filename", "--no-heading", "-o",
         "--glob", "*.rs", *excludes, _FN_RE.pattern],
        repo_root,
    )
    functions: dict[str, str] = {}
    for line in fn_out.splitlines():
        # `path:match` — split once, since a match cannot contain a colon.
        path, _, match = line.partition(":")
        m = _FN_RE.search(match)
        if not m:
            continue
        functions.setdefault(m.group(1), path)

    # `--hidden` is required, not optional. Without it every path under `.github/` is invisible, and
    # the most-cited script witness in the corpus lives there — so a real, present file resolved as
    # MISSING while the identifiers beside it resolved fine. A partial index that answers confidently
    # is worse than one that refuses.
    files_out = _run(["rg", "--files", "--no-ignore-vcs", "--hidden", *excludes], repo_root)
    paths = frozenset(files_out.split())

    basenames: dict[str, list[str]] = {}
    for p in sorted(paths):
        basenames.setdefault(p.rsplit("/", 1)[-1], []).append(p)

    return SymbolIndex(
        functions=functions,
        paths=paths,
        basenames=basenames,
        domain=[
            "**/*.rs (Rust function declarations)",
            "all files (path citations)",
            f"excluded: {', '.join(_EXCLUDED)}",
        ],
    )
