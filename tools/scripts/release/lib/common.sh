#!/usr/bin/env bash
# tools/scripts/release/lib/common.sh
# Shared functions for Temper release tooling.
#
# EXTEND of tasker-core tools/scripts/release/lib/common.sh (structure, logging,
# sed_i, bump_patch) and of temper's prior-generation common.sh (update_version_file,
# update_cargo_version, bump_minor/bump_major — kept verbatim). Added here for the
# shared-semver-policy spine (spec of record:
# temper-artifacts/specs/2026-09-09-shared-semver-policy-design.md §3, §5):
#   semver_ge                  — floor comparison for the leaf-float rule
#   update_ruby_version        — clients/temper-rb skin version
#   update_python_version      — clients/temper-py skin version
#   update_package_json_version — temper-ts / temper-ui / temper-cloud npm packages
#   update_lockfile_version    — the bun.lock / package-lock.json record of those packages
#   register_window_rows       — release-verdict register parser (spec §4.1)
#
# Source this from other release scripts:
#   source "$(dirname "$0")/lib/common.sh"
#
# Expects callers to set DRY_RUN=true|false before calling file-update functions.

set -euo pipefail

# Resolve repo root relative to this file (lib/ -> release/ -> scripts/ -> tools/ -> repo root)
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

# ---------------------------------------------------------------------------
# Logging
# ---------------------------------------------------------------------------
log_info()    { echo "  [info] $*"; }
log_warn()    { echo "  [warn] $*" >&2; }
log_error()   { echo "  [error] $*" >&2; }
log_header()  { echo ""; echo "== $* =="; echo ""; }
log_section() { echo ""; echo "-- $* --"; }

die() { log_error "$*"; exit 1; }

confirm() {
    read -p "  $1 (y/N) " -n 1 -r
    echo
    [[ $REPLY =~ ^[Yy]$ ]] || exit 1
}

# ---------------------------------------------------------------------------
# Portable sed -i (GNU vs BSD/macOS)
# ---------------------------------------------------------------------------
sed_i() {
    if sed --version 2>/dev/null | grep -q 'GNU'; then
        sed -i "$@"
    else
        sed -i '' "$@"
    fi
}

# ---------------------------------------------------------------------------
# Version arithmetic
# ---------------------------------------------------------------------------

bump_patch() {
    local version="$1"
    local major minor patch
    IFS='.' read -r major minor patch <<< "$version"
    echo "${major}.${minor}.$((patch + 1))"
}

bump_minor() {
    local version="$1"
    local major minor _patch
    IFS='.' read -r major minor _patch <<< "$version"
    echo "${major}.$((minor + 1)).0"
}

bump_major() {
    local version="$1"
    local major _minor _patch
    IFS='.' read -r major _minor _patch <<< "$version"
    echo "$((major + 1)).0.0"
}

# Compare semver: returns 0 (true) if $1 >= $2, 1 (false) otherwise.
# String comparison fails for multi-digit components ("0.1.10" < "0.1.3" is
# lexically true but numerically false) — same need as tasker-core.
semver_ge() {
    local a_major a_minor a_patch b_major b_minor b_patch
    IFS='.' read -r a_major a_minor a_patch <<< "$1"
    IFS='.' read -r b_major b_minor b_patch <<< "$2"
    (( a_major > b_major )) && return 0
    (( a_major < b_major )) && return 1
    (( a_minor > b_minor )) && return 0
    (( a_minor < b_minor )) && return 1
    (( a_patch >= b_patch )) && return 0
    return 1
}

# ---------------------------------------------------------------------------
# File update helpers (all respect DRY_RUN from caller scope)
# ---------------------------------------------------------------------------

update_version_file() {
    local version="$1"
    local file="${REPO_ROOT}/VERSION"
    if [[ "${DRY_RUN:-false}" == "true" ]]; then
        log_info "Would update VERSION -> $version"
    else
        echo "$version" > "$file"
        log_info "Updated VERSION -> $version"
    fi
}

update_cargo_version() {
    local file="$1" version="$2"

    [[ "$file" != /* ]] && file="${REPO_ROOT}/${file}"

    if [[ ! -f "$file" ]]; then
        log_warn "File not found: $file"
        return
    fi

    if [[ "${DRY_RUN:-false}" == "true" ]]; then
        local current
        current=$(grep -m1 '^version = ' "$file" | sed 's/version = "\(.*\)"/\1/')
        log_info "Would update $file version: $current -> $version"
    else
        local line_num
        line_num=$(grep -n -m1 '^version = ' "$file" | cut -d: -f1)
        if [[ -n "$line_num" ]]; then
            sed_i "${line_num}s/^version = \".*\"/version = \"${version}\"/" "$file"
        fi
        log_info "Updated $file -> $version"
    fi
}

# ---------------------------------------------------------------------------
# Leaf version writers (spec §3: client skins and npm packages float P above
# the shared floor, never below it; the floors' sites were verified on disk
# before wiring — SG-6)
# ---------------------------------------------------------------------------

# clients/temper-rb/lib/temper/version.rb — `  VERSION = '...'`
update_ruby_version() {
    local version="$1"
    local file="${REPO_ROOT}/clients/temper-rb/lib/temper/version.rb"

    if [[ ! -f "$file" ]]; then
        log_warn "Ruby version file not found: $file"
        return
    fi

    if [[ "${DRY_RUN:-false}" == "true" ]]; then
        local current
        current=$(grep -m1 "VERSION = " "$file" | sed "s/.*VERSION = '\([^']*\)'.*/\1/")
        log_info "Would update $file version: $current -> $version"
    else
        sed_i "s/\(  VERSION = '\)[^']*'/\1${version}'/" "$file"
        log_info "Updated Ruby version -> $version"
    fi
}

# clients/temper-py/temper/version.py — `__version__ = "..."`
update_python_version() {
    local version="$1"
    local file="${REPO_ROOT}/clients/temper-py/temper/version.py"

    if [[ ! -f "$file" ]]; then
        log_warn "Python version file not found: $file"
        return
    fi

    if [[ "${DRY_RUN:-false}" == "true" ]]; then
        local current
        current=$(grep -m1 '^__version__' "$file" | sed 's/__version__ = "\(.*\)"/\1/')
        log_info "Would update $file version: $current -> $version"
    else
        local line_num
        line_num=$(grep -n -m1 '^__version__' "$file" | cut -d: -f1)
        if [[ -n "$line_num" ]]; then
            sed_i "${line_num}s/^__version__ = \".*\"/__version__ = \"${version}\"/" "$file"
        fi
        log_info "Updated Python version -> $version"
    fi
}

# Any package.json's own version (first "version" field — the package version,
# never a dependency version). Used for temper-ts, temper-ui, temper-cloud.
update_package_json_version() {
    local file="$1" version="$2"

    [[ "$file" != /* ]] && file="${REPO_ROOT}/${file}"

    if [[ ! -f "$file" ]]; then
        log_warn "package.json not found: $file"
        return
    fi

    if [[ "${DRY_RUN:-false}" == "true" ]]; then
        local current
        current=$(grep -m1 '"version"' "$file" | sed 's/.*"version": "\([^"]*\)".*/\1/')
        log_info "Would update $file version: $current -> $version"
    else
        local line_num
        line_num=$(grep -n -m1 '"version"' "$file" | cut -d: -f1)
        if [[ -n "$line_num" ]]; then
            sed_i "${line_num}s/\"version\": \"[^\"]*\"/\"version\": \"${version}\"/" "$file"
        fi
        log_info "Updated $file -> $version"
    fi
}

# A package's version is also recorded in the lockfile that resolves it: the root bun.lock for a
# workspace member (packages/temper-ui, packages/temper-cloud), its own package-lock.json for a
# client (clients/temper-ts, clients/temper-telemetry-ts) or a published package outside the
# workspace (packages/temper-design). `bun install --frozen-lockfile` and
# `npm ci` both accept a stale record, so the bump has to move it; check-lockfile-versions.sh
# holds the tree to it.
#
# lockfile_of DIR — the lockfile that records the package in DIR (repo-relative): bun.lock when it
# carries a workspace entry for DIR, else DIR/package-lock.json. A workspace member is always read
# from bun.lock, so a stray package-lock.json beside it is never mistaken for the record.
lockfile_of() {
    local dir="$1"
    if [[ -f "${REPO_ROOT}/bun.lock" ]] && grep -qxF "    \"${dir}\": {" "${REPO_ROOT}/bun.lock"; then
        echo "${REPO_ROOT}/bun.lock"
    else
        echo "${REPO_ROOT}/${dir}/package-lock.json"
    fi
}

# _lockfile_versions FILE DIR [VERSION] — the one place the lockfile shapes are written down.
# Without VERSION, print each version FILE records for DIR, one per line. With VERSION, print FILE
# with exactly those values replaced. The sites: bun.lock's `workspaces` entry keyed by DIR; a
# package-lock.json's top-level "version" and its root package's (packages[""]) "version".
_lockfile_versions() {
    local file="$1" dir="$2" version="${3:-}" kind=npm
    [[ "$file" == */bun.lock ]] && kind=bun
    awk -v kind="$kind" -v key="$dir" -v v="$version" '
        function site(line) {
            if (v == "") { split(line, f, "\""); print f[4]; return line }
            sub(/"version": "[^"]*"/, "\"version\": \"" v "\"", line)
            return line
        }
        kind == "bun" && $0 == "    \"" key "\": {" { scope = 1 }
        kind == "npm" && /^  "version": "/ { $0 = site($0) }
        kind == "npm" && /^    "": \{$/ { scope = 1 }
        scope && /^      "version": "/ { $0 = site($0); scope = 0 }
        scope && /^    \}/ { scope = 0 }
        v != "" { print }
    ' "$file"
}

# lockfile_recorded_versions DIR — print every version the lockfile records for the package in
# DIR, one per line; nothing if no lockfile records it.
lockfile_recorded_versions() {
    local file
    file="$(lockfile_of "$1")"
    [[ -f "$file" ]] || return 0
    _lockfile_versions "$file" "$1"
}

# update_lockfile_version DIR VERSION — move the lockfile's record of the package in DIR to
# VERSION, then read it back.
update_lockfile_version() {
    local dir="$1" version="$2" file
    file="$(lockfile_of "$dir")"

    local recorded
    recorded="$(lockfile_recorded_versions "$dir")"
    [[ -n "$recorded" ]] || die "No lockfile records ${dir}'s version — the lockfile moved or changed shape; update this writer."

    if [[ "${DRY_RUN:-false}" == "true" ]]; then
        log_info "Would update ${file#"${REPO_ROOT}/"} record of ${dir}: $(sort -u <<< "$recorded" | tr '\n' ' ')-> ${version}"
        return
    fi

    local tmp
    tmp="$(mktemp)"
    _lockfile_versions "$file" "$dir" "$version" > "$tmp" || { rm -f "$tmp"; die "rewriting ${file#"${REPO_ROOT}/"} failed"; }
    cat "$tmp" > "$file"
    rm -f "$tmp"

    local stale
    stale="$(lockfile_recorded_versions "$dir" | grep -vxF "$version" || true)"
    [[ -z "$stale" ]] || die "${file#"${REPO_ROOT}/"} still records ${dir} at ${stale} after the bump to ${version}"
    log_info "Updated ${file#"${REPO_ROOT}/"} record of ${dir} -> ${version}"
}

# ---------------------------------------------------------------------------
# Release-verdict register (spec §4.1 — the register is in the tree, so the
# release calculator reads it; PR bodies are not). Since 2026-10-11 the register
# is one TOML file per entry under release-register/, and every reader goes
# through ONE parser, tools/manifest/manifest.py (spec of record:
# temper-artifacts:specs/2026-10-11-release-register-entries-design.md).
# ---------------------------------------------------------------------------

MANIFEST_PY="${REPO_ROOT}/tools/manifest/manifest.py"
REGISTER_DIR_DEFAULT="${REPO_ROOT}/release-register"

# Stream one register window's entries as TAB-separated lines, sorted by id:
#   pr <TAB> classes <TAB> surfaces <TAB> status <TAB> title <TAB> id
#
# $1 = the register directory; $2 = the window (default: unreleased).
# The reader validates the whole register first and refuses to stream rows from
# a malformed one. Every field is non-empty and none holds a TAB, so the
# `IFS=$'\t' read` callers below cannot shift a column — the defect the old
# markdown parser had: an entry with no `status:` streamed an empty field, the
# read collapsed it, and the row fell out of every count (three rows of the
# v0.6.0 window, found during the conversion).
register_window_rows() {
    local dir="$1" window="${2:-unreleased}"
    if [[ ! -d "$dir" ]]; then
        # The register is the durable home of the declared-class gate (spec
        # §4.1); the calculator is the last line before a release. A missing
        # register must not render as "zero rows" — that would silence both
        # gates. The --register flag exists for harness fixtures.
        log_error "Release register not found: $dir"
        return 1
    fi
    python3 "$MANIFEST_PY" register rows --dir "$dir" --window "$window"
}

# True if a comma/space-separated field list contains $2 as a token.
field_has_token() {
    local list="$1" token="$2"
    local compact="${list// /}"
    case ",${compact}," in
        *",${token},"*) return 0 ;;
        *) return 1 ;;
    esac
}

# True if a register row's surfaces list intersects a space-separated
# release-surface set ($2).
surfaces_intersect() {
    local row_surfaces="$1" release_surfaces="$2"
    local s
    for s in ${row_surfaces//,/ }; do
        if [[ " ${release_surfaces} " == *" ${s} "* ]]; then
            return 0
        fi
    done
    return 1
}

# ---------------------------------------------------------------------------
# The last release — ONE derivation (conventions COMPAT-2)
# ---------------------------------------------------------------------------
# The release calculator (detect-changes.sh opens its window here) and the declared-class
# gate (wire-class-crosscheck.sh measures against it) both call this; neither keeps a copy.
# Two derivations of "the last release" drift, and then the gate and the calculator disagree
# about which window a change belongs to.
#
#   last_release <commit> [remote]      (remote default: $RELEASE_REMOTE, else origin)
#
# Prints eval-safe KEY=VALUE on stdout:
#   LAST_RELEASE_REF=<tag|commit>
#   LAST_RELEASE_IS_FALLBACK=true|false
#   LAST_RELEASE_REASON=<why it is a fallback; empty when it is not>
#
# LAST_RELEASE_IS_FALLBACK=false only when every condition holds:
#   * a tag in the release form, v<MAJOR>.<MINOR>.<PATCH>, is reachable from <commit>: the
#     newest by version among them (other tag forms are not releases and are skipped);
#   * it is an ancestor of <commit>;
#   * the history holds it: the clone is not shallow (a shallow clone is cut short, and a
#     tag-less checkout cannot see any tag);
#   * the remote lists the tag on the same commit (a tag nobody pushed is not a release);
#   * the remote lists no other release tag that <commit> contains and that is a descendant of
#     it or a higher version; a listed tag whose commit this clone cannot place, or a remote whose tags
#     cannot be listed, cannot show that, so it is a fallback too.
# Otherwise LAST_RELEASE_IS_FALLBACK=true. LAST_RELEASE_REF is then the newest release-form
# ancestor tag if one was found (the calculator's best guess at its window, flagged), else the root commit. A fallback
# is NEVER a release: a consumer that needs one must not use LAST_RELEASE_REF when the flag is
# true, and a consumer that cannot read the flag treats it as true.
last_release() {
    local at="${1:-HEAD}" remote="${2:-${RELEASE_REMOTE:-origin}}"
    local tag="" commit="" listed="" sha name c
    local form='^v[0-9]+\.[0-9]+\.[0-9]+$'

    _last_release_emit() { # $1 = ref, $2 = is-fallback, $3 = reason
        printf 'LAST_RELEASE_REF=%q\n' "$1"
        printf 'LAST_RELEASE_IS_FALLBACK=%s\n' "$2"
        printf 'LAST_RELEASE_REASON=%q\n' "$3"
    }
    _last_release_fallback() { # $1 = reason
        local ref="$tag"
        [[ -n "$ref" ]] || ref="$(git rev-list --max-parents=0 "$at" 2>/dev/null | head -n1)"
        _last_release_emit "$ref" true "$1"
    }

    if ! git cat-file -e "${at}^{commit}" 2>/dev/null; then
        _last_release_emit "" true "${at} is not a commit in this clone"
        return 0
    fi
    # The newest release-form tag among the ancestors of <commit>, by version, not `describe`'s
    # nearest: a nearer maintenance-line tag (v1.0.1 merged after v1.1.0) or a pre-release tag
    # (v1.1.0-rc1) must not stand in for the newest release.
    tag="$(git tag --merged "$at" --list 'v*' 2>/dev/null | grep -E "$form" \
        | sed 's/^v//' | sort -t. -k1,1n -k2,2n -k3,3n | tail -n1)"
    [[ -z "$tag" ]] || tag="v${tag}"
    if [[ "$(git rev-parse --is-shallow-repository 2>/dev/null)" != "false" ]]; then
        _last_release_fallback "the clone is shallow: its history is cut short, so no tag found in it is provably the last release (git fetch --unshallow --tags)"
        return 0
    fi
    if [[ -z "$tag" ]]; then
        _last_release_fallback "no v<MAJOR>.<MINOR>.<PATCH> tag is reachable from ${at} (were tags fetched?)"
        return 0
    fi
    commit="$(git rev-parse "${tag}^{commit}")"
    if ! git merge-base --is-ancestor "$commit" "$at" 2>/dev/null; then
        _last_release_fallback "${tag} is not an ancestor of ${at}"
        return 0
    fi
    if ! listed="$(git ls-remote --tags "$remote" 'refs/tags/v*' 2>/dev/null)"; then
        _last_release_fallback "the tags on ${remote} could not be listed, so a newer release cannot be ruled out"
        return 0
    fi

    # One `<tag> <commit>` line per release-form tag on the remote; for an annotated tag the
    # peeled `^{}` line follows its tag line, so the last line for a name is its commit.
    # (Plain lines, not an associative array: bash 3.2, macOS's, has none.)
    local pairs
    pairs="$(while IFS=$'\t' read -r sha name; do
        name="${name#refs/tags/}"; name="${name%^\{\}}"
        [[ "$name" =~ $form ]] && printf '%s %s\n' "$name" "$sha"
    done <<< "$listed" | awk '{ c[$1] = $2 } END { for (n in c) print n, c[n] }')"

    if [[ "$(awk -v t="$tag" '$1 == t { print $2 }' <<< "$pairs")" != "$commit" ]]; then
        _last_release_fallback "${remote} does not list ${tag} on ${commit}: a tag the remote does not hold is not a release"
        return 0
    fi
    while read -r name c; do
        [[ -n "$name" && "$c" != "$commit" ]] || continue
        if ! git cat-file -e "${c}^{commit}" 2>/dev/null; then
            _last_release_fallback "${remote} lists ${name} on ${c}, which this clone cannot place (git fetch --tags)"
            return 0
        fi
        git merge-base --is-ancestor "$c" "$at" || continue
        if git merge-base --is-ancestor "$commit" "$c" || ! semver_ge "${tag#v}" "${name#v}"; then
            _last_release_fallback "${remote} lists ${name}, a newer release than ${tag} that ${at} contains"
            return 0
        fi
    done <<< "$pairs"
    _last_release_emit "$tag" false ""
}
