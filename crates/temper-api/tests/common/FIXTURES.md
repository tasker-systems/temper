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

The pair is committed rather than generated per run: RSA keygen at test time would tax the
suites for no property this fixture needs.

Rules for changes:

- A fresh clone must never need a setup step because of these files — no regeneration
  ritual, ever. Generation-at-test-time would be a redesign, not a chore.
- If a key must actually change (algorithm change, format change), regenerate deliberately
  with `openssl genpkey` / `openssl pkey -pubout` and update every consumer listed in the
  grep for the file name.
