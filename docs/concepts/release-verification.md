# Release Verification

**For operators and users** — anyone who wants to verify that the `temper` binary on their
machine is the one the release workflow published, and to understand what each verification
check does and does not prove.

## What ships in a release

A Temper release archive is self-contained: the `temper` binary, a bundled `libonnxruntime`
(the ONNX Runtime library for the local embedding pipeline), and a copy of the project
`LICENSE`. Alongside the archive, each macOS and Linux release publishes a **per-file
manifest** — a JSON document carrying the SHA-256 and size of every file the archive ships.
The manifest answers a narrower, stronger question than "did the archive download intact?":
it lets you check that the exact binary and library sitting on your disk are the ones the
release actually shipped.

## Three verdicts, not two

Every verification surface in Temper — the installer, `temper version --verify`, and
`temper update` — reports one of three verdicts, never a bare pass/fail:

| Verdict | Meaning |
|---|---|
| `verified` | Every file matched the manifest. |
| `mismatch` | At least one file disagreed — the verdict names the file(s). |
| `unverifiable` | There is nothing to check against, or the check itself could not run. |

**`unverifiable` is not `mismatch`.** A binary built from source has no manifest beside it;
a network hiccup means a check never ran; a Windows install ships no manifest today; a
release that predates manifests has none to fetch. None of these say anything about whether
your binary is wrong — they say the question could not be answered. Rendering "we cannot
tell" as "it is wrong" would be its own kind of dishonesty, so Temper never collapses the
two. Absence and disagreement are different answers.

## Two kinds of verification, two kinds of trust

Temper separates **internal consistency** from **provenance**.

Offline verification (`temper version --verify`) checks the binary, the ONNX Runtime
library, and the model against the manifest installed *beside them* in the same directory.
It is real, and it catches real problems — corruption, a partial extraction, a hand-edited
file, local drift. It is not adversarially meaningful: an actor who could replace your
binary could replace the manifest sitting next to it too. Treat a `verified` result here as
"this install is internally consistent," not as proof of provenance.

Online verification (`temper version --verify --online`) re-fetches the *published* manifest
for your exact version and platform from GitHub — rather than trusting the copy beside your
binary — and, once that comparison agrees, verifies GitHub's build-provenance attestation
over the digest of that fetched manifest against a pinned **Sigstore trust root** (the
cryptographic root Temper trusts to sign attestations). This is the check that answers
*"is the temper on my machine byte-identical to what the release workflow published?"* — a
compromised manifest beside a compromised binary can no longer hide behind a same-directory
comparison, because the comparison object is fetched fresh and independently checked against
a signature GitHub's workflow produced, not anything on your disk.

The two attestation checks cover deliberately different objects: online verification checks
the attestation over the digest of the *fetched manifest* — the exact bytes just compared —
whereas `temper update` checks it over the *archive's* digest, because on that path the
archive is the object being installed. They are different checks over different objects, not
two views of the same one.

A failure anywhere in the online chain — network, an unusable pinned trust root, or a bundle
that does not vouch for this artifact — renders `unverifiable`, never a false `verified`. On
success it also plants the offline baseline (if none exists), so offline verification works
from then on without a network. It never overwrites an existing baseline: one that disagrees
with the published manifest is a signal you should see, not something to quietly repair.

## The pre-manifest upgrade hop

A self-update out of a release that predates manifests leaves no baseline behind — the
installer embedded in an older binary predates the manifest machinery, so the archive
checksum is verified and the new binary installed, but no baseline is written. Offline
verification then reports `unverifiable`, correctly. A single online verification plants the
baseline; from then on both offline verification and every subsequent update maintain it.
This affects only the one upgrade hop out of a pre-manifest binary.

## What build-provenance attestation does and does not prove

The attestation binds **the builder and the tag, never the source.** A genuine signature over
a genuinely-built artifact says nothing about whether the commit behind the tag is one you
would approve of. That limit is inherent to build provenance, not specific to Temper: the
signature proves *what was built and by which workflow*, not *whether what was built is what
you wanted*. Two further trusts sit outside the signature chain — you trust GitHub's
attestation service, and you trust the pinned Sigstore trust root Temper ships. The
out-of-band audit below removes the second from the picture.

## Atomic swap with rollback

The installer never touches your live install directory until every extracted file has
matched the manifest. If anything disagrees, the install aborts and **your existing install
is left untouched** — the same atomic-swap-with-rollback machinery that guards a binary that
fails to run at all also guards a file that fails to match. The manifest is written into your
install directory only after a successful, verified install, so later offline verification
has something to check against.

## Windows: a deliberate, stated gap

Windows installs verify the archive checksum but write no per-file manifest and have no
attestation-verified update path today. Offline verification on Windows therefore always
reports `unverifiable` — it can never report `verified`, because there is nothing installed
to check against. This is a stated, deliberate gap, not a silent hole: nothing here implies
Windows gets the same guarantee macOS and Linux do.

## Out-of-band audit

Every release archive's build-provenance attestation is independently checkable with GitHub's
own `gh` CLI, with no dependency on Temper or its pinned trust root:

```sh
gh attestation verify temper-v0.3.0-aarch64-apple-darwin.tar.gz --repo tasker-systems/temper
```

Download the archive for your platform from the releases page, then run that command against
it. This is the check to reach for if you do not want to trust Temper's own verification code
at all — it goes straight to GitHub's attestation service and removes the pinned trust root
from the picture too. It does not prove anything Temper's own check does not: `gh` verifies
the same build-provenance predicate over the same subject, so it carries the same boundary —
the builder and the tag, never the source.

## Further reading

- **The flag-level reference for `temper version`:**
  [`temper version`](../reference/cli/version.md).
- **The flag-level reference for `temper update`:**
  [`temper update`](../reference/cli/update.md).
- **The trust boundary this verification sits before:**
  [The Trust Boundary](./trust-boundary.md).
- **Using Temper from the CLI:**
  [temperkb.io/using-temper](https://temperkb.io/using-temper).
