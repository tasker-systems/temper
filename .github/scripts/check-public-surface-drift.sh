#!/usr/bin/env bash
#
# Fail when the public prose surface drifts from the machine-owned facts it restates.
#
# Three checks, each mechanically decidable from this checkout:
#
#   1. cli-claims — every fenced `temper …` invocation in README.md and docs/ resolves to a
#      real command page with real flags. The authority is the committed docs/reference/cli
#      tree, which check-cli-reference-drift.sh keeps byte-equal to the tree-under-review's
#      binary (it re-emits from a freshly built binary on any tree edit). Prose == tree (here)
#      and tree == binary (there) compose to prose == binary, which is why this gate reads the
#      tree and needs no toolchain — and can live in the ungated guard-tests job. Excluded
#      from scanning: the generated reference trees themselves (each owned by its own gate).
#      Only long flags (`--flag`) are checked; short-flag claims are unchecked, stated here.
#      What a fence claims is its temper SEGMENT: a pipe, separator, or redirection ends the
#      invocation, and comments (quote-aware, via shlex) are not claims at all.
#   2. counts — route/cron/function counts stated in the self-host playbook must equal what
#      vercel.json actually derives (routes count `src`-entries; a `handle` directive is not
#      a route). The page names vercel.json as its authority; this makes the numbers provable
#      instead of restated.
#   3. roadmap sweep — a NEW tracked file under internal/ or design-system/ may not carry
#      roadmap verbs. This repo is public: undone work must not read as a promise to a reader
#      who cannot ask the team. Diff-scoped to files ADDED since PSD_BASE (default HEAD~1) so
#      existing files are grandfathered and the gate cannot go red on arrival.
#
# What this CANNOT see, stated rather than assumed away:
#
#   - the SERVED surface (docs.temperkb.io, temperkb.io). This gate reads the checkout only;
#     a stale publish is invisible here, and a green run never means the served site matches.
#   - package registries. Counts that name a registry cannot be verified offline; only
#     counts with an in-repo authority are checked, everything else is reported unchecked.
#   - the site repo's pages. Not in this checkout, not checked here.
#   - multi-commit pushes. The sweep diffs one commit deep, so files added earlier inside a
#     pushed range are seen on the range's next push. On a pull request the checkout is the
#     merge ref and HEAD~1 is the base tip, so the whole PR delta is seen.
#
# Reports only: this gate never writes, generates, or lands anything.
#
# Usage: bash .github/scripts/check-public-surface-drift.sh
#
# Seams (no CI job sets them; test-check-public-surface-drift.sh does):
#   PSD_REPO_ROOT — synthetic repo root for the guard test
#   PSD_BASE      — diff base for the roadmap sweep (default HEAD~1)

set -euo pipefail

DEFAULT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ROOT="${PSD_REPO_ROOT:-$DEFAULT_ROOT}"
BASE="${PSD_BASE:-HEAD~1}"

# The verb list the sweep enforces. The guard test derives this assignment from this
# file rather than restating it — a verb dropped here must go unenforced LOUDLY, not
# silently keep being tested by a stale copy of the list.
ROADMAP_VERBS='not yet|planned|upcoming|when it ships|proposed|deferred|unbuilt'

FAILURES=0

fail() {
    echo "FAIL: $1" >&2
    FAILURES=$((FAILURES + 1))
}

ok() {
    echo "OK: $1"
}

cd "$ROOT"

# --- Vacuity guards: refuse to report clean on a scan that checked nothing ---

for required in README.md vercel.json docs/reference/cli; do
    if [ ! -e "$required" ]; then
        fail "required authority '$required' is absent — refusing to check nothing"
        AUTHORITIES_MISSING=1
    fi
done

TMPDIR_GATE="${TMPDIR:-/tmp}"
CLI_OUT_FILE="$(mktemp "${TMPDIR_GATE}/psd-cli.XXXXXX")"
COUNTS_OUT_FILE="$(mktemp "${TMPDIR_GATE}/psd-counts.XXXXXX")"
trap 'rm -f "$CLI_OUT_FILE" "$COUNTS_OUT_FILE"' EXIT

# --- Check 1: fenced cli-claims resolve to real commands and real flags ---

CLI_RC=0
if [ "${AUTHORITIES_MISSING:-0}" -eq 0 ]; then
python3 - >"$CLI_OUT_FILE" <<'PYEOF' || CLI_RC=$?
import os, re, shlex, sys

root = os.getcwd()
tree = os.path.join(root, "docs", "reference", "cli")
root_page = os.path.join(tree, "README.md")
with open(root_page, encoding="utf-8") as f:
    global_flags_text = f.read()

files = [os.path.join(root, "README.md")]
for dirpath, dirnames, filenames in os.walk(os.path.join(root, "docs")):
    rel = os.path.relpath(dirpath, root)
    if rel == os.path.join("docs", "reference"):
        dirnames[:] = [d for d in dirnames if d not in ("cli", "config")]
    for name in filenames:
        if name.endswith(".md"):
            files.append(os.path.join(dirpath, name))

fence_re = re.compile(r"^\s*(```|~~~)")
line_re = re.compile(r"^\s*temper(\s+.*)$")
checked = 0
failures = []

def check_invocation(path, lineno, line):
    global checked
    try:
        # comments=True: a `#` starts a comment only when UNQUOTED, so a trailing
        # comment is not part of the invocation and a quoted "#…" argument is
        # content — both decided by shlex, not by a split this function guesses at.
        tokens = shlex.split(line, comments=True)
    except ValueError:
        failures.append((path, lineno, "unparseable invocation (unbalanced quote?)"))
        return
    if not tokens or tokens[0] != "temper":
        return
    body = tokens[1:]
    command = []
    flags = []
    for tok in body:
        # Shell metacharacters end the invocation: everything after a pipe,
        # separator, or redirection is another command's business and makes no
        # claim about temper's surface (`temper team list | jq …` claims
        # `temper team list`, not `jq`).
        if tok in ("|", "||", "&&", ";", "&") or tok[0] in "<>":
            break
        if tok.startswith("--"):
            flags.append(tok.split("=", 1)[0])
        elif not tok.startswith("-") and not flags:
            command.append(tok)
    checked += 1
    if command:
        page = os.path.join(tree, command[0] + ".md")
        label = " ".join(command)
    else:
        page = root_page
        label = "(root)"
    if not os.path.exists(page):
        failures.append((path, lineno, f"'temper {label}' — no reference page for '{command[0] if command else ''}'"))
        return
    with open(page, encoding="utf-8") as f:
        page_text = f.read()
    # Walk the invocation's command path against the page. Tokens past the top-level
    # command are either a documented subcommand (the page carries a `### `temper …`
    # heading for the extended path) or a positional argument (the current node's
    # `Usage:` line shows a placeholder other than <COMMAND> — that one means a
    # subcommand is REQUIRED, so an unknown token there is drift). First positional
    # ends the path; everything after it is argument text.
    # A flag-only invocation (`temper --help`) has NO command path: the page branch
    # above already selected the root page for it, and only the flag check below
    # applies. Indexing command[0] here was the crash a swallowed exit code turned
    # into a clean banner.
    if command:
        prefix = [command[0]]
        args_started = False
        for tok in command[1:]:
            if args_started:
                continue
            heading = "### `temper " + " ".join(prefix + [tok]) + "`"
            if heading in page_text:
                prefix.append(tok)
                continue
            usage = re.search(
                rf"(?m)^Usage: temper {' '.join(re.escape(t) for t in prefix)}\s+(\S.*)$",
                page_text,
            )
            remainder = usage.group(1) if usage else ""
            placeholders = [t for t in remainder.split()
                            if t.startswith("<") or t.startswith("[")]
            positional = any(t != "[OPTIONS]" and t != "<COMMAND>" for t in placeholders)
            if positional:
                args_started = True
                continue
            failures.append((path, lineno,
                             f"'temper {' '.join(prefix)} {tok}' — '{tok}' is neither a documented"
                             f" subcommand nor a positional argument of '{' '.join(prefix)}'"))
            return
    for flag in flags:
        if flag not in page_text and flag not in global_flags_text:
            failures.append((path, lineno, f"'temper {label} {flag}' — flag absent from its reference page"))

for path in files:
    in_fence = False
    logical = []  # (first-physical-lineno, joined text) — `\`-continuations joined
    buf = None
    buf_start = 0
    with open(path, encoding="utf-8") as f:
        for lineno, raw in enumerate(f, 1):
            if fence_re.match(raw):
                in_fence = not in_fence
                if buf is not None:
                    logical.append((buf_start, buf))
                    buf = None
                continue
            if not in_fence:
                continue
            line = raw.rstrip("\n")
            if buf is not None:
                if buf.endswith("\\"):
                    buf = buf[:-1].rstrip()
                buf = buf + " " + line.strip()
                if not buf.endswith("\\"):
                    logical.append((buf_start, buf))
                    buf = None
            elif line_re.match(line):
                if line.endswith("\\"):
                    buf = line
                    buf_start = lineno
                else:
                    logical.append((lineno, line))
        if buf is not None:
            logical.append((buf_start, buf))
    for lineno, text in logical:
        text = text.strip()
        if text.endswith("\\"):
            text = text[:-1]
        check_invocation(os.path.relpath(path, root), lineno, text)

print(f"cli-claims: {checked} fenced invocations checked, {len(failures)} failures")
for path, lineno, msg in failures:
    print(f"FAIL: {path}:{lineno}: {msg}")
sys.exit(1 if failures else 0)
PYEOF

echo "$CLI_OUT_FILE contents:" >/dev/null
sed -n 's/^cli-claims: /cli-claims: /p' "$CLI_OUT_FILE"
grep '^FAIL:' "$CLI_OUT_FILE" >&2 || true
# A completed checker ALWAYS reaches its summary line (printed before exit), and exits
# non-zero only when it printed FAIL lines. RC≠0 with no summary is a CRASH — and
# counting FAIL lines out of output a crashed process never wrote is exactly how a
# crash used to read as a clean banner. Refuse: an incomplete scan checks nothing.
if ! grep -q '^cli-claims: ' "$CLI_OUT_FILE"; then
    fail "cli-claims checker did not complete (rc=${CLI_RC}) — refusing to report clean on a scan that may not have run"
elif [ "$CLI_RC" -ne 0 ]; then
    FAILURES=$((FAILURES + $(grep -c '^FAIL:' "$CLI_OUT_FILE" || true)))
fi
fi

# --- Check 2: stated counts equal what vercel.json derives ---

COUNTS_RC=0
if [ "${AUTHORITIES_MISSING:-0}" -eq 0 ]; then
python3 - >"$COUNTS_OUT_FILE" <<'PYEOF' || COUNTS_RC=$?
import json, re, sys

with open("vercel.json", encoding="utf-8") as f:
    manifest = json.load(f)
derived = {
    "crons": len(manifest.get("crons", [])),
    "routes": sum(1 for r in manifest.get("routes", []) if "src" in r),
    "functions": len(manifest.get("functions", {})),
}

page_path = "docs/playbooks/self-host-temper.md"
with open(page_path, encoding="utf-8") as f:
    page = f.read()

words = {"one": 1, "two": 2, "three": 3, "four": 4, "five": 5, "six": 6, "seven": 7,
         "eight": 8, "nine": 9, "ten": 10, "eleven": 11, "twelve": 12, "thirteen": 13,
         "fourteen": 14, "fifteen": 15, "sixteen": 16, "seventeen": 17, "eighteen": 18,
         "nineteen": 19, "twenty": 20}
word_re = "|".join(words)

failures = []
stated = 0
for noun in ("routes", "crons", "functions"):
    singular = noun.rstrip("s")
    found = []
    found += [(m.group(1), m.start()) for m in
              re.finditer(rf"\b(\d+)\s*(?:configured\s+|real\s+|declared\s+)?{noun}\b", page)]
    found += [(m.group(1), m.start()) for m in
              re.finditer(rf"\b({word_re})\s*(?:configured\s+|real\s+|declared\s+)?{noun}\b", page, re.IGNORECASE)]
    found += [(m.group(1), m.start()) for m in
              re.finditer(rf"\b{singular}s?\s*\(\s*[×x]\s*(\d+)\s*\)", page, re.IGNORECASE)]
    for raw, pos in found:
        value = words.get(raw.lower(), raw)
        value = int(value)
        stated += 1
        if value != derived[noun]:
            line = page.count("\n", 0, pos) + 1
            failures.append((page_path, line, f"stated {value} {noun} vs vercel.json's {derived[noun]}"))

print(f"counts: {stated} stated figures checked against vercel.json, {len(failures)} failures")
for path, line, msg in failures:
    print(f"FAIL: {path}:{line}: {msg}")
sys.exit(1 if failures else 0)
PYEOF

sed -n 's/^counts: /counts: /p' "$COUNTS_OUT_FILE"
grep '^FAIL:' "$COUNTS_OUT_FILE" >&2 || true
# Same completion invariant as the cli-claims check: no summary line means the
# checker died (a malformed vercel.json dies at json.load) — never a clean scan.
if ! grep -q '^counts: ' "$COUNTS_OUT_FILE"; then
    fail "counts checker did not complete (rc=${COUNTS_RC}) — refusing to report clean on a scan that may not have run"
elif [ "$COUNTS_RC" -ne 0 ]; then
    FAILURES=$((FAILURES + $(grep -c '^FAIL:' "$COUNTS_OUT_FILE" || true)))
fi
fi

# --- Check 3: roadmap sweep over NEW tracked files under internal/ and design-system/ ---

if ! git rev-parse --verify "$BASE" >/dev/null 2>&1; then
    fail "roadmap sweep: base '$BASE' is not a commit — refusing to sweep nothing"
else
    NEW_FILES="$(git diff --name-only --diff-filter=A "$BASE" -- internal design-system \
        | grep -E '\.(md|mdx|txt)$' || true)"
    SWEPT=0
    while IFS= read -r f; do
        [ -z "$f" ] && continue
        [ -f "$f" ] || continue
        SWEPT=$((SWEPT + 1))
        while IFS= read -r hit; do
            verb="$(echo "$hit" | grep -oiE "\b(${ROADMAP_VERBS})\b" | head -1)"
            fail "roadmap sweep: $f carries roadmap language ('${verb}') — state shipped behavior, or relocate the file"
        done < <(grep -inE "\b(${ROADMAP_VERBS})\b" "$f" || true)
    done <<< "$NEW_FILES"
    ok "roadmap sweep: ${SWEPT} new file(s) swept against the verb list, base ${BASE}"
fi

# --- Summary ---

echo ""
if [ "$FAILURES" -gt 0 ]; then
    echo "public-surface drift: ${FAILURES} failure(s). This gate reads the CHECKOUT only —"
    echo "it cannot see the served surface, package registries, or the site repo's pages."
    exit 1
fi
echo "public-surface drift: clean. This gate reads the CHECKOUT only — it cannot see the"
echo "served surface, package registries, or the site repo's pages; a green run never"
echo "means the served site matches."
