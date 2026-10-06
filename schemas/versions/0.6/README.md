# Pin 0.6 — the wire contract of minor version 0.6

**Source:** tag `v0.6.0` (commit `9953380c2cc8352751b9b5e58ef0eca8e2170dce`),
`openapi.json` at that tag, copied byte-identical
(sha256 `c6bf0ff7d30811141797a776d404ae6ba376f6e59487c949c2381193e3c5fa45`).
Cut 2026-10-06, after the release, from the released contract. The 0.6.0 release PR
did not carry it.

**The rule this pin enforces:** within the era this pin anchors, the committed
`openapi.json` may only GROW. A client built against this shape interoperates
with every later release of the era, in both skew directions. The gate is
`.github/scripts/check-openapi-pin.sh` (`cargo make openapi-pin-check`); its
additive standard is the declared-class gate's — shapes only grow, tolerant in both
skew directions — computed by the same comparator (`wire-shape-lib.jq`). The
release-time discipline is documented in [RELEASING.md](../../RELEASING.md),
"Versioning: 0.M.P, the era level, and the declared-class gate".

**Relation to pin 0.5:** this pin holds everything pin 0.5 does, plus the additive growth
of the 0.5 era (v0.5.1 → v0.6.0). Pin selection takes the greatest pin ≤ `VERSION`'s
minor, so from this commit the 0.6 era is judged against this shape, not the 0.5 founding
shape.

**How the next pin is cut** (release checklist, at the next era release): copy the
*release candidate's* `openapi.json` to `schemas/versions/<M.m>/openapi.json` with a
README beside it naming the source commit and date, in the same PR as the `VERSION`
bump.
