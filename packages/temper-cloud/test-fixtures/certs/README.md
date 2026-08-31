# SAML test-fixture certificates

**Test-only. No production trust relationship, deliberately committed.**

Nothing verifies against these. Production validates SAML assertions against the certificate in
`kb_saml_idp`, which an operator supplies; these exist so the test suite can *sign* assertions and
watch the SP accept or refuse them. Publishing the private keys is what makes the fixtures usable —
they are not secrets that leaked, they are inputs.

| File | Role in the tests |
|---|---|
| `idp-cert.pem` / `idp-key.pem` | The IdP's current signing key. |
| `idp-cert-secondary.pem` / `idp-key-secondary.pem` | The *incoming* key of a signing-key rollover, and — in the states where it is not configured — the unknown-signer case. Two pairs are the minimum that can tell "either configured cert is accepted" apart from "any certificate is accepted". |

Both are self-signed RSA-2048, `CN=test-idp.example.com`, generated with `openssl req -x509`.
They are checked in rather than generated per run because a SAML fixture needs a matching
*certificate*, not just a keypair, and making a fresh clone mint one would be a setup step
contributors have to know about. Each private key also carries this statement in-file (a
leading comment — the consumer here is xml-crypto/OpenSSL, which skips text before `BEGIN`;
jose-based consumers cannot tolerate comments at all).
