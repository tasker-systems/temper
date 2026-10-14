# Release-verdict register

The durable home of the declared-class gate (shared semver policy, D-S4 — spec of record:
`temper-artifacts/specs/2026-09-09-shared-semver-policy-design.md`, §4.1, as amended by the
compat-deprecation regime, `temper-artifacts/specs/2026-09-22-compat-semver-policy-amendment-design.md`).
Every wire-touching PR lands its declaration here, in the same PR. The release checklist consults
this register before any release: a release whose surface class has an open gated entry waits.

Since 2026-10-11 the register is **one TOML file per entry**
(`temper-artifacts/specs/2026-10-11-release-register-entries-design.md`). Before that it was a
single markdown file every PR prepended to, so any two wire PRs conflicted; that file's shipped
history is frozen, unread by any tool, in [HISTORY.md](HISTORY.md).

## Adding your entry

```bash
python3 tools/manifest/manifest.py register new <id>     # copies _template.toml to unreleased/<id>.toml
$EDITOR release-register/unreleased/<id>.toml
python3 tools/manifest/manifest.py register validate
```

- **The id is the file name.** It is lowercase kebab-case, at most 64 characters, unique across
  every window. Never rename it once merged: `corrections.json` cites entries by id.
- **One PR, one or more new files.** The gate (`wire-class-crosscheck.sh`) counts only entries the PR
  **adds** under `unreleased/` whose `pr` is `"self"` or the PR's own number. Editing or renaming
  someone else's entry is not a declaration.
- `cargo make register` prints the unreleased window as markdown. Never commit that output: a
  committed rendering is a single file again, and the conflicts come back.

## An entry

```toml
title    = "One line: the change, stated as what is now true"
pr       = "self"                       # or the PR number
classes  = ["additive", "behavioral"]
surfaces = ["http", "cli-stdout"]
status   = "signal-only"
body = '''
What changed behind which shape · who observes it · user-visibility · release relevance.
'''
```

All six keys are required, and nothing else is allowed. **The tokens each key may take are
defined once, in [vocabulary.toml](vocabulary.toml)**, which every reader validates against. This
file explains what the tokens mean; it never restates the lists.

- **`classes`:**
  - `additive`: shapes only grow, tolerant in both skew directions; the floor P bump at the next
    release.
  - `shape-breaking`: a rename, a removal, a type change, or an envelope change.
  - `behavioral`: the meaning behind an unchanged shape changed. This is review-owned, because CI
    is structurally barred from certifying it.
  - `spec-correction`: see below.
  - `retirement-train`: see the routing rule below.
- **`surfaces`:** the surfaces this entry speaks for. The release calculator intersects them with
  the surfaces a release ships. `erasure-policy` is the one surface that is not a door: it names
  the versioned erasure-policy manifest in `policy/erasure/`, which classifies what erasure covers.
  A PR whose diff touches that directory must add an entry listing it, and the declared-class gate
  fails the PR otherwise. It asks for the declaration only, never a class, because the policy is
  not wire shape.
- **`status`:** where the entry stands for the next release.
  - `open` gates the release until carried.
  - `signal-only` records without gating.
  - `satisfied` has been carried.
  - `blocked:<release-class>` refuses a client-skin release without the verdict owner's explicit
    override. The class after `blocked:` is free text, never tokenized.
- **`pr`:** the PR number, or `self` (the number is unknowable before push), `goal`, or
  `pre-policy`.

**Routing (D-C2).** A `shape-breaking` entry below the era level names its routing by adding
`retirement-train` to `classes`. The movement either converts to the deprecation path, or waits for
the retirement release train:
- **Converting** means re-declaring the entry `additive` + `behavioral`. The re-declaration is the
  record, so `converted` needs no token.
- **A bare `shape-breaking` entry** fails the declared-class gate on a main-bound PR.
- **`retirement-train` on its own** is malformed: it is a routing, never a class.
- **Deprecation entries** (the D-C3 records) carry the retirement horizon: the era release the
  record names.

**`spec-correction`** declares that `openapi.json` described a request or answer the server never
made, and that the movement brings the contract to what the server always did.
- **Review-owned.** The crosscheck passes it on a moved shape with a note, because CI cannot check
  the claim. Like `behavioral`, the entry must name what the server always did and why both skew
  directions hold.
- **What it never does.** It never rides with `shape-breaking`, and it does not answer growth, which
  is still `additive`'s.
- **The pin's record.** The pin gate judges the tree rather than the PR, so each corrected operation,
  or component schema (an enum a response reaches through `$ref`, say), is also recorded in
  `corrections.json` beside the current pin (`schemas/versions/<M.m>/`), with its corrected shape
  and this entry's **id** as `register_entry`. The pin gate resolves that id
  across every window and checks the entry declares `spec-correction`.

## Windows and the roll

- **`unreleased/`** is the window since the last release. The release calculator reads only this
  window.
- **`v<version>/`** holds the entries a release shipped. After a release is tagged,
  `tools/scripts/release/roll-register.sh v<version>` moves exactly the entries present in the
  tag's tree. An entry merged after the tag stays in `unreleased/`.
- **A skipped roll fails loudly.** The calculator refuses an `unreleased/` entry that the current
  release's tag already contains, rather than counting it twice.

## Readers

Every reader goes through `tools/manifest/manifest.py` (Python ≥ 3.11, standard library only):
- the PR gate (`wire-class-crosscheck.sh`);
- the release calculator and `release-prepare.sh`, through `register_window_rows`;
- the pin gate (`check-openapi-pin.sh`).

No reader parses an entry by hand.
