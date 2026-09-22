# Pin 0.5 — the wire contract of minor version 0.5

**Source:** tag `v0.5.1` (commit `108f0a64fdc996bdc305e9bade71715813af0724`),
`openapi.json` at that tag, copied byte-identical
(sha256 `8037667507455709aa2981c67166595f2d7219aa81706532728b5c31c9f77ba1`).
Cut 2026-09-16, the founding pin — the wire-shape baseline of the widest current
enterprise adoption.

**The rule this pin enforces:** within the era this pin anchors, the committed
`openapi.json` may only GROW. A client built against this shape interoperates
with every later release of the era, in both skew directions. The gate is
`.github/scripts/check-openapi-pin.sh` (`cargo make openapi-pin-check`); its
additive standard is the declared-class gate's — shapes only grow, tolerant in both
skew directions — computed by the same comparator (`wire-shape-lib.jq`). The
release-time discipline (when the next pin is cut, and how the retirement release
executes against it) is documented in [RELEASING.md](../../RELEASING.md),
"Versioning: 0.M.P, the era level, and the declared-class gate".

**How the next pin is cut** (release checklist, at the retirement release): copy the
*release candidate's* `openapi.json` to `schemas/versions/<M.m>/openapi.json` with a
README beside it naming the source commit and date, in the same PR as the `VERSION`
bump. Until that PR merges, this pin stays in force and a retirement movement
correctly reads as pin-breaking — green returns when the era release cuts the new
pin.
