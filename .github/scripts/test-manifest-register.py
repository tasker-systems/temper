#!/usr/bin/env python3
"""Harness for tools/manifest/manifest.py's `register` family — prove it fails where it must and
passes where it may.

Every failing arm is a MUTATION of a well-formed fixture register: break exactly one thing the
reader claims to catch, and require exit 1 with the offending file AND key named. A reader that
fails for the wrong reason would pass a bare exit-code check, so each arm asserts the message too.

Witnesses from the spec (temper-artifacts:specs/2026-10-11-release-register-entries-design.md):
  W1  two PRs' entries never conflict (and the old one-file layout does, against the same script)
  W3  the vocabulary is data: a token absent from vocabulary.toml fails, and passes once listed
  W4  entry-level semantics hold (each mutation below)
  W11 the reader fails loudly below its Python floor

Run: python3 .github/scripts/test-manifest-register.py
"""

import os
import runpy
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
READER = REPO / "tools" / "manifest" / "manifest.py"
VOCABULARY = REPO / "release-register" / "vocabulary.toml"
TEMPLATE = REPO / "release-register" / "_template.toml"

passed = 0
failed = 0

GOOD = """title = "A good entry"
pr = "self"
classes = ["additive"]
surfaces = ["http"]
status = "signal-only"
body = '''
What changed, who observes it, why it matters.
'''
"""


def ok(name):
    global passed
    passed += 1
    print("  PASS: %s" % name)


def bad(name, *detail):
    global failed
    failed += 1
    print("  FAIL: %s" % name)
    for d in detail:
        for line in str(d).splitlines():
            print("    %s" % line)


def reader(*args, cwd=None):
    r = subprocess.run([sys.executable, str(READER), *args], capture_output=True, text=True, cwd=cwd)
    return r.returncode, r.stdout + r.stderr


def fixture(tmp, entries=None, vocabulary=None):
    """A register directory with the real vocabulary and template, and the given entries."""
    reg = Path(tmp) / "release-register"
    (reg / "unreleased").mkdir(parents=True)
    (reg / "unreleased" / ".gitkeep").write_text("")
    (reg / "vocabulary.toml").write_text(vocabulary if vocabulary is not None else VOCABULARY.read_text())
    shutil.copyfile(TEMPLATE, reg / "_template.toml")
    for rel, text in (entries or {"unreleased/good-entry.toml": GOOD}).items():
        p = reg / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)
    return reg


def expect_fail(name, entries, needle_file, needle_key, vocabulary=None):
    with tempfile.TemporaryDirectory() as tmp:
        reg = fixture(tmp, entries, vocabulary)
        rc, out = reader("register", "validate", "--dir", str(reg))
        if rc == 1 and needle_file in out and needle_key in out:
            ok(name)
        else:
            bad(name, "exit=%d, expected 1 naming %r and %r" % (rc, needle_file, needle_key), out)


def mutate(old, new):
    assert old in GOOD, old
    return GOOD.replace(old, new)


print("test-manifest-register")
print()

# ── The boring case ─────────────────────────────────────────────────────────────────────────────
with tempfile.TemporaryDirectory() as tmp:
    reg = fixture(tmp)
    rc, out = reader("register", "validate", "--dir", str(reg))
    if rc == 0 and "1 entries" in out:
        ok("a well-formed entry validates")
    else:
        bad("a well-formed entry validates", "exit=%d" % rc, out)

# ── W3: the vocabulary is data ──────────────────────────────────────────────────────────────────
CUSTOM = mutate('classes = ["additive"]', 'classes = ["additive", "experimental"]')
expect_fail("W3: a class absent from vocabulary.toml fails, naming file and key",
            {"unreleased/custom.toml": CUSTOM}, "custom.toml", "classes: 'experimental' is not one of")
with tempfile.TemporaryDirectory() as tmp:
    vocab = VOCABULARY.read_text().replace('"spec-correction"]', '"spec-correction", "experimental"]')
    assert '"experimental"' in vocab, "the vocabulary's classes line moved; update this arm"
    reg = fixture(tmp, {"unreleased/custom.toml": CUSTOM}, vocab)
    rc, out = reader("register", "validate", "--dir", str(reg))
    if rc == 0:
        ok("W3: the same entry passes once the fixture's vocabulary.toml lists the class")
    else:
        bad("W3: the same entry passes once the fixture's vocabulary.toml lists the class", "exit=%d" % rc, out)

# ── W4: entry-level semantics, one mutation each ────────────────────────────────────────────────
W4 = [
    ("retirement-train on its own", 'classes = ["additive"]', 'classes = ["retirement-train"]',
     "classes: 'retirement-train' is a routing"),
    ("spec-correction with shape-breaking", 'classes = ["additive"]', 'classes = ["spec-correction", "shape-breaking"]',
     "contradict"),
    ("blocked: with no class", 'status = "signal-only"', 'status = "blocked:"', "status: 'blocked:' names no release class"),
    ("an unknown status", 'status = "signal-only"', 'status = "pending"', "status: 'pending'"),
    ("an unknown surface", 'surfaces = ["http"]', 'surfaces = ["grpc"]', "surfaces: 'grpc' is not one of"),
    ("empty classes", 'classes = ["additive"]', "classes = []", "classes: must be a non-empty array"),
    ("a repeated class", 'classes = ["additive"]', 'classes = ["additive", "additive"]', "listed more than once"),
    ("a missing key", 'status = "signal-only"\n', "", "status: missing"),
    ("an unknown key", 'status = "signal-only"\n', 'status = "signal-only"\nseverity = "high"\n',
     "severity: is not an entry key"),
    ("a TAB in the title", 'title = "A good entry"', 'title = "A good\\tentry"', "title: must be one line with no TAB"),
    ("an empty title", 'title = "A good entry"', 'title = ""', "title: must be a non-empty string"),
    ("pr = true", 'pr = "self"', "pr = true", "pr: True"),
    ("pr = 0", 'pr = "self"', "pr = 0", "pr: 0"),
    ("an unknown pr sentinel", 'pr = "self"', 'pr = "me"', "pr: 'me'"),
    ("an empty body", "What changed, who observes it, why it matters.\n", "", "body: must be a non-empty string"),
    ("malformed TOML", 'status = "signal-only"', 'status = "signal-only', "toml:"),
]
for name, old, new, needle in W4:
    expect_fail("W4: %s fails" % name, {"unreleased/mutated.toml": mutate(old, new)}, "mutated.toml", needle)

expect_fail("W4: a bad id (the file stem) fails", {"unreleased/Bad_Id.toml": GOOD}, "Bad_Id.toml", "id: 'Bad_Id'")
expect_fail("W4: an id reused across windows fails",
            {"unreleased/twice.toml": GOOD, "v0.6.0/twice.toml": GOOD}, "twice.toml", "already the id of")
expect_fail("W4: a non-.toml file among the entries fails",
            {"unreleased/good-entry.toml": GOOD, "unreleased/row.md": "- **a row**\n"}, "row.md", "layout: is not an entry")
expect_fail("W4: an unknown directory in the register fails",
            {"unreleased/good-entry.toml": GOOD, "shipped/x.toml": GOOD}, "shipped", "layout: is not an entry directory")
expect_fail("W4: a vocabulary key the reader does not know fails",
            {"unreleased/good-entry.toml": GOOD}, "vocabulary.toml", "severity: is not a vocabulary key",
            vocabulary=VOCABULARY.read_text() + '\nseverity = ["high"]\n')

with tempfile.TemporaryDirectory() as tmp:
    reg = fixture(tmp)
    (reg / "vocabulary.toml").unlink()
    rc, out = reader("register", "validate", "--dir", str(reg))
    if rc == 1 and "vocabulary.toml" in out and "missing" in out:
        ok("W4: a register with no vocabulary.toml fails — the vocabulary is never read from code")
    else:
        bad("W4: a register with no vocabulary.toml fails", "exit=%d" % rc, out)

with tempfile.TemporaryDirectory() as tmp:
    rc, out = reader("register", "validate", "--dir", str(Path(tmp) / "nowhere"))
    if rc == 1 and "never zero entries" in out:
        ok("a missing register directory fails, rather than reading as zero entries")
    else:
        bad("a missing register directory fails", "exit=%d" % rc, out)

# ── rows: the stream the bash consumers read ────────────────────────────────────────────────────
with tempfile.TemporaryDirectory() as tmp:
    second = (mutate('classes = ["additive"]', 'classes = ["additive", "behavioral"]')
              .replace('title = "A good entry"', 'title = "Another entry"')
              .replace('pr = "self"', "pr = 1114")
              .replace('status = "signal-only"', 'status = "blocked:temper-rb"'))
    reg = fixture(tmp, {"unreleased/b-entry.toml": second, "unreleased/a-entry.toml": GOOD, "v0.6.0/old-entry.toml": GOOD})
    rc, out = reader("register", "rows", "--dir", str(reg))
    want = ("self\tadditive\thttp\tsignal-only\tA good entry\ta-entry\n"
            "1114\tadditive, behavioral\thttp\tblocked:temper-rb\tAnother entry\tb-entry\n")
    if rc == 0 and out == want:
        ok("rows: the unreleased window, sorted by id, six TAB columns, the rolled window excluded")
    else:
        bad("rows: the unreleased window as six TAB columns", "exit=%d" % rc, repr(out), "want " + repr(want))

    rc, out = reader("register", "rows", "--dir", str(reg), "--window", "v0.6.0")
    if rc == 0 and out.endswith("\told-entry\n") and out.count("\n") == 1:
        ok("rows --window v0.6.0 reads only the rolled window")
    else:
        bad("rows --window v0.6.0", "exit=%d" % rc, out)

    loose = Path(tmp) / "added" / "pr-entry.toml"
    loose.parent.mkdir()
    loose.write_text(GOOD)
    rc, out = reader("register", "rows", "--dir", str(reg), "--files", str(loose))
    if rc == 0 and out == "self\tadditive\thttp\tsignal-only\tA good entry\tpr-entry\n":
        ok("rows --files reads exactly the named entries (the PR gate's seam)")
    else:
        bad("rows --files", "exit=%d" % rc, out)

    # resolve searches every window: a correction outlives its entry's roll.
    rc, out = reader("register", "resolve", "old-entry", "--dir", str(reg))
    if rc == 0 and "v0.6.0/old-entry.toml\tadditive" in out:
        ok("resolve finds an entry in a rolled window")
    else:
        bad("resolve finds an entry in a rolled window", "exit=%d" % rc, out)
    rc, out = reader("register", "resolve", "no-such-entry", "--dir", str(reg))
    if rc == 1 and "no-such-entry" in out:
        ok("resolve of an unknown id fails, naming it")
    else:
        bad("resolve of an unknown id fails", "exit=%d" % rc, out)

with tempfile.TemporaryDirectory() as tmp:
    reg = fixture(tmp, {"unreleased/a-entry.toml": GOOD, "unreleased/broken.toml": mutate('pr = "self"', "pr = 0")})
    rc, out = reader("register", "rows", "--dir", str(reg))
    if rc == 1 and "broken.toml" in out and "\ta-entry" not in out:
        ok("rows refuses a malformed register — it never yields rows from one")
    else:
        bad("rows refuses a malformed register", "exit=%d" % rc, out)

# ── new: the scaffold is not a valid entry until filled in ──────────────────────────────────────
with tempfile.TemporaryDirectory() as tmp:
    reg = fixture(tmp)
    rc, out = reader("register", "new", "my-change", "--dir", str(reg))
    made = reg / "unreleased" / "my-change.toml"
    if rc == 0 and made.is_file():
        ok("new scaffolds unreleased/<id>.toml from the template")
    else:
        bad("new scaffolds unreleased/<id>.toml", "exit=%d" % rc, out)
    rc, out = reader("register", "validate", "--dir", str(reg))
    if rc == 1 and "my-change.toml" in out and "title" in out and "classes" in out:
        ok("an unedited scaffold fails validation")
    else:
        bad("an unedited scaffold fails validation", "exit=%d" % rc, out)
    rc, out = reader("register", "new", "good-entry", "--dir", str(reg))
    if rc == 1 and "taken" in out:
        ok("new refuses an id that is already taken")
    else:
        bad("new refuses a taken id", "exit=%d" % rc, out)

# ── render ──────────────────────────────────────────────────────────────────────────────────────
with tempfile.TemporaryDirectory() as tmp:
    reg = fixture(tmp)
    rc, out = reader("register", "render", "--dir", str(reg))
    if rc == 0 and "- **A good entry**" in out and "  What changed, who observes it" in out:
        ok("render prints the window as markdown")
    else:
        bad("render prints the window as markdown", "exit=%d" % rc, out)

# ── W11: the floor is loud ──────────────────────────────────────────────────────────────────────
probe = ("import runpy, sys; sys.version_info = (3, 10, 0, 'final', 0); "
         "sys.argv = ['manifest.py', 'register', 'validate']; runpy.run_path(%r, run_name='__main__')" % str(READER))
r = subprocess.run([sys.executable, "-c", probe], capture_output=True, text=True)
if r.returncode == 1 and "needs Python 3.11" in r.stderr:
    ok("W11: below its Python floor the reader exits 1 naming the floor")
else:
    bad("W11: below the floor the reader exits 1 naming it", "exit=%d" % r.returncode, r.stderr)

# ── W1: two PRs' entries never conflict ─────────────────────────────────────────────────────────
# Against the same script, the old layout conflicts: that is what makes this a witness and not a
# tautology. Both branches start from one base and each records its own declaration.
def git(cwd, *args):
    env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t", GIT_COMMITTER_NAME="t",
               GIT_COMMITTER_EMAIL="t@t", GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_SYSTEM=os.devnull)
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, env=env)


def two_branches_merge(layout_old):
    with tempfile.TemporaryDirectory() as tmp:
        git(tmp, "init", "-q", "-b", "main")
        if layout_old:
            Path(tmp, "REGISTER.md").write_text("# Register\n\n## Since v1 — unreleased\n- **older row**\npr: 1\n")
        else:
            fixture(tmp, {"unreleased/older.toml": GOOD})
        git(tmp, "add", "-A")
        git(tmp, "commit", "-q", "-m", "base")
        for branch in ("pr-a", "pr-b"):
            git(tmp, "checkout", "-q", "-b", branch, "main")
            if layout_old:
                p = Path(tmp, "REGISTER.md")
                p.write_text(p.read_text().replace("unreleased\n", "unreleased\n- **%s row**\npr: self\n" % branch))
            else:
                Path(tmp, "release-register", "unreleased", "%s.toml" % branch).write_text(GOOD)
            git(tmp, "add", "-A")
            git(tmp, "commit", "-q", "-m", branch)
        git(tmp, "checkout", "-q", "main")
        first = git(tmp, "merge", "-q", "--no-edit", "pr-a")
        second = git(tmp, "merge", "-q", "--no-edit", "pr-b")
        return first.returncode == 0 and second.returncode == 0


if two_branches_merge(layout_old=False) and not two_branches_merge(layout_old=True):
    ok("W1: two PRs' entries merge cleanly; the same two declarations in one file conflict")
else:
    bad("W1: entries merge cleanly where one file conflicts",
        "entries merged: %s; one-file merged: %s" % (two_branches_merge(False), two_branches_merge(True)))

# ── The real register is live, not just fixture-true ────────────────────────────────────────────
rc, out = reader("register", "validate")
if rc == 0:
    ok("the repo's own release-register/ validates")
else:
    bad("the repo's own release-register/ validates", "exit=%d" % rc, out)

print()
print("  %d passed, %d failed" % (passed, failed))
sys.exit(1 if failed else 0)
