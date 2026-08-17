"""Read goals and tasks from the knowledge base, through the `temper` CLI.

**Using the PATH binary is the right call here, not a compromise.** It is rebuilt from the working
branch and is the current client; a tool that wants live vault data should use it. (Do not reach for
`./target/debug/temper` — that is the one binary that goes stale, and only because nothing rebuilds
it.) This is unlike the repo's other generated-artifact gates, which build from source because they
project REPO content and must compare against the tree under review. This projection reads a remote
system that has no in-tree representation at all, so there is nothing to build from.

That asymmetry has a consequence worth stating where the calls are made: **a diff in the output can
mean the remote changed rather than the repository** — the opposite of what every other drift gate
here means by drift.
"""

from __future__ import annotations

import json
import shutil
import subprocess
from dataclasses import dataclass, field


class SourceUnavailable(RuntimeError):
    """The knowledge base could not be reached. Distinct from 'it answered, and said nothing'."""


@dataclass
class Task:
    id: str
    title: str
    stage: str
    open_meta: dict = field(default_factory=dict)

    def citations(self, goal_id: str) -> list[tuple[str, list[str]]]:
        """(kind, clauses) for each `witnesses`/`enables` block naming this goal.

        A task declares one or the other, never both — `witnesses` claims to BE evidence, `enables`
        builds the mechanism that makes evidence possible. Reading them as one number is the false
        positive the discipline forbids, so they stay separate all the way into the artifact.
        """
        out = []
        for kind in ("witnesses", "enables"):
            block = self.open_meta.get(kind)
            if isinstance(block, dict) and block.get("goal") == goal_id:
                out.append((kind, list(block.get("clauses") or [])))
        return out

    def cites_anything(self) -> bool:
        return any(isinstance(self.open_meta.get(k), dict) for k in ("witnesses", "enables"))


@dataclass
class Goal:
    id: str
    title: str
    context_ref: str
    status: str
    body: str


def available() -> bool:
    return shutil.which("temper") is not None


def _temper(args: list[str]) -> dict:
    if not available():
        raise SourceUnavailable("`temper` is not on PATH")
    proc = subprocess.run(
        ["temper", *args, "--format", "json"], capture_output=True, text=True
    )
    if proc.returncode != 0:
        raise SourceUnavailable(
            f"temper {' '.join(args)} failed ({proc.returncode}): {proc.stderr.strip()}"
        )
    try:
        # `temper` may append output after the JSON document; decode the first complete value.
        return json.JSONDecoder().raw_decode(proc.stdout)[0]
    except json.JSONDecodeError as exc:
        raise SourceUnavailable(f"temper {' '.join(args)} returned unparseable output: {exc}")


def _rows(payload: dict, what: str) -> list[dict]:
    if payload.get("truncated"):
        # A partial page presented as a whole one is the failure this whole tool exists to prevent.
        raise SourceUnavailable(f"{what} listing was truncated; refusing to project a partial page")
    return payload.get("rows", [])


def client_version() -> str:
    proc = subprocess.run(["temper", "--version"], capture_output=True, text=True)
    return proc.stdout.strip() or "unknown"


def active_goals(context_ref: str) -> tuple[list[Goal], int]:
    """Active goals in the context, and the TOTAL number of goals seen.

    The total is returned because a count over a subset needs its denominator stated. A projection
    covering 30 of 61 goals that reports only "30" invites being read as "all of them".
    """
    everything = _temper(["resource", "list", "--type", "goal", "--context", context_ref, "--all"])
    all_rows = _rows(everything, "goal")
    goals = []
    for row in all_rows:
        if (row.get("managed_meta") or {}).get("temper-status") != "active":
            continue
        full = _temper(["resource", "show", row["id"]])
        goals.append(
            Goal(
                id=row["id"],
                title=row.get("title", ""),
                context_ref=row.get("context_ref", context_ref),
                status="active",
                body=full.get("content") or "",
            )
        )
    goals.sort(key=lambda g: g.id)
    return goals, len(all_rows)


def tasks_in_context(context_ref: str) -> list[Task]:
    payload = _temper(
        ["resource", "list", "--type", "task", "--context", context_ref, "--with", "open-meta", "--all"]
    )
    tasks = [
        Task(
            id=r["id"],
            title=r.get("title", ""),
            stage=(r.get("managed_meta") or {}).get("temper-stage", "-"),
            open_meta=r.get("open_meta") or {},
        )
        for r in _rows(payload, "task")
    ]
    tasks.sort(key=lambda t: t.id)
    return tasks


def advancing_tasks(goal_id: str) -> list[Task]:
    """Tasks linked to the goal by an `advances` edge.

    **This is the axis the citation cannot see.** Goal membership has two spellings and nothing ties
    them: the edge is what `list --goal` filters on, the `open_meta` block is a separate claim. A
    task can carry either without the other, so both are read and neither is treated as the truth.
    """
    payload = _temper(
        ["resource", "list", "--type", "task", "--goal", goal_id, "--with", "open-meta", "--all"]
    )
    tasks = [
        Task(
            id=r["id"],
            title=r.get("title", ""),
            stage=(r.get("managed_meta") or {}).get("temper-stage", "-"),
            open_meta=r.get("open_meta") or {},
        )
        for r in _rows(payload, "advancing task")
    ]
    tasks.sort(key=lambda t: t.id)
    return tasks
