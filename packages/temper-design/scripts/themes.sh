#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

usage() {
  cat <<USAGE
usage:
  themes.sh build                     regenerate every <theme>/theme.css, contract/tailwind.css and themes.json
  themes.sh check <theme.json>...     contract + contrast floors + distinctness + generated CSS in sync
  themes.sh contrast <theme.json>     print every contrast pair the contract checks, with its floor
  themes.sh slots <theme.json>        the slot canvas: within-family and slot-vs-chrome/state ΔE
  themes.sh --self-check              contract valid + every theme passes + broken fixtures refused + brand marks in currentColor

exit: 0 pass, 1 check failure, 2 usage/setup error
USAGE
}

die() { printf 'themes: %s\n' "$1" >&2; exit 2; }

# Python that can import jsonschema: the themes venv when present, else PATH.
if [ -x "$ROOT/.venv/bin/python3" ]; then
  PYTHON="$ROOT/.venv/bin/python3"
else
  PYTHON="python3"
fi

if ! "$PYTHON" -c 'import jsonschema' >/dev/null 2>&1; then
  die "module 'jsonschema' not importable by $PYTHON - run 'uv sync' in packages/temper-design/"
fi

case "${1:-}" in
  build | check | contrast | slots | --self-check) exec "$PYTHON" "$SCRIPT_DIR/themes.py" "$@" ;;
  -h | --help) usage ;;
  *) usage >&2; exit 2 ;;
esac
