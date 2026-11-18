#!/usr/bin/env bash
# out-of-line-test-modules.sh — sourced, never run: which Rust files are wholly test code.
#
# A test module declared out of line, `#[cfg(test)]` / `#[cfg(all(test, …))]` on a `mod NAME;`
# line, compiles only into a test build, so the file it declares is test code, and so is every
# module that file declares in turn. Ruling R24 moves temper-services' DB test modules into such
# sibling files. A tripwire that skips inline test modules by structure must skip these files too,
# or every move reads as a new production site.
#
# The opener is the inline skip's predicate: `any(…)`, `not(…)` and a feature-only gate compile
# into some production build, so a module under one is NOT test code here.
#
# Sourced by audit-team-membership-sinks.sh and audit-raw-standing-writers.sh. Defines:
#   out_of_line_test_files ROOT   print every *.rs under ROOT/**/src/ that is wholly test code
#
# Text, not the module tree: a `#[path]` attribute or a macro-generated `mod` is not followed.

# The file `mod NAME;` in file $1 resolves to (lib.rs, main.rs and mod.rs declare siblings; any
# other foo.rs declares into foo/), or nothing.
child_module_file() {
  local parent="$1" name="$2" base
  case "$(basename "$parent")" in
    lib.rs | main.rs | mod.rs) base="$(dirname "$parent")" ;;
    *) base="${parent%.rs}" ;;
  esac
  if [[ -f "$base/$name.rs" ]]; then echo "$base/$name.rs"; elif [[ -f "$base/$name/mod.rs" ]]; then echo "$base/$name/mod.rs"; fi
}

# NAME for each test-gated `mod NAME;` in file $1 (with `all`=1, for each `mod NAME;` at all: the
# children of a file that is already test code).
declared_modules() {
  awk -v all="${2:-0}" '
    function opener(l) {
      return l ~ /^[[:space:]]*#\[cfg\(test\)\]/ ||
        (l ~ /^[[:space:]]*#\[cfg\(all\(/ && l ~ /[(,[:space:]]test[),[:space:]]/ && l !~ /any\(/ && l !~ /not\(/)
    }
    {
      if (pending && ($0 ~ /^[[:space:]]*#\[/ || $0 ~ /^[[:space:]]*$/ || $0 ~ /^[[:space:]]*\/\//)) next
      if ((pending || all) && match($0, /^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?mod[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*;/)) {
        m = substr($0, RSTART, RLENGTH); sub(/;.*/, "", m); sub(/.*mod[[:space:]]+/, "", m); sub(/[[:space:]]+$/, "", m)
        print m
      }
      pending = opener($0)
    }
  ' "$1"
}

# Every file under $1 that is wholly test code by an out-of-line test gate.
out_of_line_test_files() {
  local root="$1" f name child queue=() seen=" "
  while read -r f; do
    while read -r name; do
      [[ -n "$name" ]] && child="$(child_module_file "$f" "$name")" && [[ -n "$child" ]] && queue+=("$child")
    done < <(declared_modules "$f")
  done < <(find "$root" -type f -name '*.rs' -path '*/src/*' 2>/dev/null | sort)
  while [[ ${#queue[@]} -gt 0 ]]; do
    f="${queue[0]}"; queue=("${queue[@]:1}")
    [[ "$seen" == *" $f "* ]] && continue
    seen+="$f "
    echo "$f"
    while read -r name; do
      [[ -n "$name" ]] && child="$(child_module_file "$f" "$name")" && [[ -n "$child" ]] && queue+=("$child")
    done < <(declared_modules "$f" 1)
  done
}
