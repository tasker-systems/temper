# wire-shape.jq — the openapi shape verdict (shared-semver-policy spec §4, D-S4).
#
# The definitions (broke_node, verdict) live in wire-shape-lib.jq — the one
# definition home, shared since 2026-09-16 with wire-pin-movements.jq (the pin
# gate's per-movement naming). This file is the standalone verdict caller:
#
#   jq -n -r -f wire-shape.jq -L <this dir> --slurpfile base base.json --slurpfile head head.json
#
# Output: "moved" or "grew". Callers MUST pass -L pointing at this directory:
# jq does not resolve `include` relative to the program file's own directory
# (measured, jq 1.7.1) — the crosscheck's derive_shape passes it for the same
# reason.
include "wire-shape-lib";
verdict
