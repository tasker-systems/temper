# Test fixture keys

**Test-only. No production trust relationship. Deliberately committed.**

Nothing in production verifies against these keys. The API validates real JWTs against the
IdP's JWKS or the AS's `/oauth/jwks`; these pairs exist so the test suites can mint and
verify tokens locally. Publishing the private halves is what makes the fixtures usable —
they are not secrets that leaked, they are inputs.

| File | Private? | Annotated in-file |
|---|---|---|
| `test_rsa.key` | yes | yes |
| `test_rsa.pub` | no | — |
| `test_ed25519.key` | yes | **no — see below** |
| `test_ed25519.pub` | no | — |

The Ed25519 pair cannot carry an in-file comment: `packages/temper-cloud/tests/auth.test.ts`
reaches into this directory by relative path and feeds the private half to jose
(`importPKCS8`), which requires the PEM string to *begin* at `-----BEGIN` and base64-decodes
the entire remainder after stripping markers — a comment in any position fails it. That
cross-language consumer is also why these keys are committed rather than generated per run:
per-suite generation would leave the TS suite and the Rust suite minting with different
pairs, and the shared fixture is what lets either side's tokens be read by the other's
verifier. (RSA keygen at test time would additionally tax the suites for no property this
fixture needs.)

Rules for changes:

- A fresh clone must never need a setup step because of these files — no regeneration
  ritual, ever. Generation-at-test-time would be a redesign, not a chore.
- If a key must actually change (algorithm change, format change), regenerate deliberately
  with `openssl genpkey` / `openssl pkey -pubout` and update every consumer listed in the
  grep for the file name — the TS suites reach into these directories by relative path.
