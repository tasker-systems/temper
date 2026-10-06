#!/usr/bin/env bash
# audit-privileged-action-pins.sh — every action a privileged job runs is pinned by full commit SHA.
#
# WHY THIS EXISTS
# ---------------
# An action referenced by tag or branch can be repointed upstream (the `tj-actions/changed-files`
# class, March 2025) and then runs inside the job with everything the job holds. A full commit SHA
# cannot be repointed. "Privileged" here means a job whose token can do damage that outlives it:
#   * `id-token: write` — crates.io, npm, RubyGems and PyPI trusted publishing each exchange the
#     job's OIDC token for a publish credential;
#   * `attestations: write` — mints a genuine Sigstore signature;
#   * `contents: write` — creates tags and Releases. release-summary uploads the archives AND the
#     `.sha256` files install.sh verifies them against, so a matched swap there passes install.sh;
#   * `actions: write` — dispatches workflows, the scope that completes the tag-then-release chain
#     ci.yml's header describes;
#   * `packages: write` — publishes to GitHub Packages.
#
# build-cli-binaries.yml took this posture first; release.yml's publish lanes did not, and nothing
# noticed. This makes the posture a property of the tree rather than of whoever last edited a job.
#
# WHAT IS CHECKED
# ---------------
# For every job in .github/workflows/*.yml whose EFFECTIVE permissions grant one of those five
# scopes (or `write-all`):
#   * every `uses:` is a local path (`./…`), a `docker://…@sha256:` image, or
#     `owner/repo[/path]@<40-hex SHA>` carrying a `# <tag>` trailer, which Dependabot rewrites with
#     the SHA so a reader can tell which release is pinned;
#   * a local composite action (`./.github/actions/<name>`) is followed into its action.yml and held
#     to the same rule, since its steps run inside the privileged job.
#
# Effective permissions: a job's own `permissions:` block when it has one, else the workflow-level
# block. A reusable workflow called by a privileged job is checked with the CALLER's grant as the
# default for its permission-less jobs — that is the grant they actually run under. A workflow with
# no permissions block at all is treated as unprivileged: GitHub's default token never carries
# `id-token` or `attestations`.
#
# A scan that finds NO privileged job fails rather than passing: today there are several, so zero
# means the parser stopped seeing them.
#
# NOT CHECKED, stated so a green tick is not read as covering it:
#   * what the job's own `run:` steps execute (build scripts, `npm ci`, `uv build` all run with the
#     token requestable);
#   * a job whose credential arrives as a SECRET rather than a token scope — release.yml's
#     update-homebrew-tap pushes with a deploy key while declaring `contents: read`. Its checkout
#     is pinned by hand; nothing here would notice it unpinned;
#   * `security-events: write` (CodeQL's upload grant) and the other write scopes not listed above;
#   * workflows outside .github/workflows.
#
# Usage: bash .github/scripts/audit-privileged-action-pins.sh
# The two directory variables exist for test-audit-privileged-action-pins.sh's fixtures; nothing
# else sets them.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WORKFLOWS_DIR="${PIN_AUDIT_WORKFLOWS_DIR:-${REPO_ROOT}/.github/workflows}"
ACTIONS_ROOT="${PIN_AUDIT_ACTIONS_ROOT:-${REPO_ROOT}}"

exec python3 - "$WORKFLOWS_DIR" "$ACTIONS_ROOT" <<'PY'
import os, re, sys, glob

workflows_dir, actions_root = sys.argv[1], sys.argv[2]
PRIV = re.compile(r'^\s*(id-token|attestations|contents|actions|packages):\s*write\s*$')
USES = re.compile(r'^(\s*)(?:-\s+)?uses:\s*([^\s#]+)\s*(#\s*\S.*)?$')
SHA_REF = re.compile(r'^[A-Za-z0-9_.-]+/[A-Za-z0-9_./-]+@[0-9a-f]{40}$')

def indent(line):
    return len(line) - len(line.lstrip(' '))

def perm_block(lines, i, key_indent):
    """Given lines[i] is a `permissions:` key at key_indent, return 'priv' / 'unpriv'."""
    inline = lines[i].split(':', 1)[1].strip()
    if inline:
        inline = inline.split('#', 1)[0].strip()
        return 'priv' if inline == 'write-all' else 'unpriv'
    j = i + 1
    while j < len(lines):
        l = lines[j]
        if l.strip() and not l.lstrip().startswith('#'):
            if indent(l) <= key_indent:
                break
            if PRIV.match(l):
                return 'priv'
        j += 1
    return 'unpriv'

def parse_workflow(path):
    """-> (workflow_perm or None, {job: {'perm': .., 'uses': [(lineno, ref, comment)]}})"""
    lines = open(path).read().split('\n')
    wf_perm, jobs, in_jobs, job = None, {}, False, None
    for i, l in enumerate(lines):
        if not l.strip() or l.lstrip().startswith('#'):
            continue
        ind = indent(l)
        if ind == 0:
            in_jobs = l.startswith('jobs:')
            job = None
            if l.startswith('permissions:'):
                wf_perm = perm_block(lines, i, 0)
            continue
        if not in_jobs:
            continue
        if ind == 2 and re.match(r'^  [A-Za-z0-9_-]+:\s*$', l):
            job = l.strip()[:-1]
            jobs[job] = {'perm': None, 'uses': []}
            continue
        if job is None:
            continue
        if ind == 4 and l.strip().startswith('permissions:'):
            jobs[job]['perm'] = perm_block(lines, i, 4)
        m = USES.match(l)
        if m:
            jobs[job]['uses'].append((i + 1, m.group(2), m.group(3)))
    return wf_perm, jobs

def bad_ref(ref, comment):
    if ref.startswith('./'):
        return None
    if ref.startswith('docker://'):
        return None if '@sha256:' in ref else 'docker image not pinned by digest'
    if not SHA_REF.match(ref):
        return 'not pinned to a full commit SHA'
    if not comment:
        return 'SHA pin has no `# <tag>` trailer naming the release it was taken from'
    return None

def composite_uses(ref):
    path = os.path.join(actions_root, ref[2:])
    for name in ('action.yml', 'action.yaml'):
        f = os.path.join(path, name)
        if os.path.isfile(f):
            out = []
            for n, l in enumerate(open(f).read().split('\n'), 1):
                m = USES.match(l)
                if m and not l.lstrip().startswith('#'):
                    out.append((f, n, m.group(2), m.group(3)))
            return out
    return None  # a reusable-workflow path, or missing

files = sorted(glob.glob(os.path.join(workflows_dir, '*.yml')) + glob.glob(os.path.join(workflows_dir, '*.yaml')))
parsed = {os.path.basename(f): (f, *parse_workflow(f)) for f in files}

# Pass 1: which local reusable workflows are called by a privileged job (they inherit its grant).
def effective(job, wf_perm, default):
    return job['perm'] or wf_perm or default

inherited = set()
for name, (f, wf_perm, jobs) in parsed.items():
    for job in jobs.values():
        if effective(job, wf_perm, 'unpriv') != 'priv':
            continue
        for _, ref, _ in job['uses']:
            m = re.match(r'^\./\.github/workflows/([^@]+)$', ref)
            if m:
                inherited.add(m.group(1))

privileged, failures = 0, []
for name, (f, wf_perm, jobs) in parsed.items():
    default = 'priv' if name in inherited else 'unpriv'
    for jname, job in jobs.items():
        if effective(job, wf_perm, default) != 'priv':
            continue
        privileged += 1
        rel = os.path.relpath(f, os.getcwd())
        for n, ref, comment in job['uses']:
            why = bad_ref(ref, comment)
            if why:
                failures.append(f'{rel}:{n}  job `{jname}`  {ref}  — {why}')
            if ref.startswith('./.github/actions/'):
                inner = composite_uses(ref)
                if inner is None:
                    failures.append(f'{rel}:{n}  job `{jname}`  {ref}  — composite action has no action.yml to check')
                    continue
                for cf, cn, cref, ccomment in inner:
                    why = bad_ref(cref, ccomment)
                    if why:
                        failures.append(f'{os.path.relpath(cf, os.getcwd())}:{cn}  (run by privileged job `{jname}` in {rel})  {cref}  — {why}')

if privileged == 0:
    print(f'FAIL: no job in {workflows_dir} holds a privileged write scope.', file=sys.stderr)
    print('  Several do today, so this means the parser stopped seeing them — a scan that finds', file=sys.stderr)
    print('  nothing must fail, not pass.', file=sys.stderr)
    sys.exit(1)

if failures:
    print('FAIL: a privileged job (id-token/attestations/contents/actions/packages write) runs an action that can be repointed:', file=sys.stderr)
    for x in failures:
        print('  ' + x, file=sys.stderr)
    print('', file=sys.stderr)
    print('  Pin each to the full commit SHA of a release, with the tag as a trailer:', file=sys.stderr)
    print('    uses: owner/repo@<40-hex sha> # vX.Y.Z', file=sys.stderr)
    print('  A branch ref (e.g. dtolnay/rust-toolchain@stable) has no release to pin; replace it', file=sys.stderr)
    print('  (build-cli-binaries.yml installs the toolchain with rustup directly).', file=sys.stderr)
    sys.exit(1)

print(f'audit-privileged-action-pins: OK — {privileged} privileged job(s), every action pinned by SHA.')
PY
