# Arc-2 review — THREAT MODEL lens

**Branch:** `jct/binary-attestation-manifest-verification` (PR #573)
**Question:** does the chain resist what it claims, and where does residual trust still live?

The individual links are good. `attest.rs` pins the right root, scopes identity to repo +
workflow + tag, keeps Rekor inclusion proofs, and fails closed in two distinguishable
classes. `version.rs`'s `--verify --online` already fixed the earlier "two verifications,
different objects" defect by attesting the **manifest's own digest**. The findings below
are about **composition**, and the top two are the same shape as that earlier defect,
one level over.

---

## F1 — HIGH. An empty `files` list is a fully-valid, fully-attestable `verified` that proves nothing

**Empty scope falls open at every consumer, producer included.**

- `manifest.rs:84-110` — `verify_dir` iterates `manifest.files`; with `files: []` the
  `mismatches` vec stays empty and it returns `Verdict::Verified`.
- `install.sh:253-272` — `verify_manifest_against_dir` reads pairs from an awk pass; with
  no pairs the `while` body never executes, `CHECK_FAILED` stays `0`, and it returns
  success.
- `.github/scripts/release/emit-manifest.sh:36-48` — an empty `find` yields
  `ENTRIES=""`, and `printf '%s' "" | jq -s '{…files: .}'` emits `{"files":[]}` with
  **exit 0**. Verified by running it against an empty staging dir.
- Nothing anywhere asserts the manifest lists `temper`. `rg 'files\.is_empty|files\.len'`
  over `manifest.rs`, `install.sh`, `emit-manifest.sh` → zero hits.

**Demonstrated, not theorised.** Driving the real `install.sh` with an arbitrary archive
and `{"version":"0.3.0","target":"x86_64-unknown-linux-gnu","files":[]}`:

```
  Verifying file manifest...
  Verifying the new binary runs...
✓ Installed temper v0.3.0 to …/install
--- installed temper ---
#!/bin/sh
echo "0.3.0"
# EVIL PAYLOAD
--- persisted manifest ---
{"version":"0.3.0","target":"x86_64-unknown-linux-gnu","files":[]}
```

**Failure scenario A (no attacker required).** A future edit drifts the `STAGING` env in
`build-cli-binaries.yml:172-179`, or `find` misses the tree. `emit-manifest.sh` exits 0
with `files: []`. `upload-artifact`'s `if-no-files-found: error` is satisfied (the file
exists). `attest-build-provenance` **attests the vacuous manifest** — so
`/attestations/sha256:{vacuous_digest}` resolves and `temper version --verify --online`
returns a genuine, signature-backed **`verified`** over a manifest that covers zero files.
Every link is cryptographically valid; the composition guarantees nothing. CI stays green:
`test-emit-manifest.sh` only exercises a populated staging dir.

**Failure scenario B (release-asset write, cannot forge attestations).** Replace only the
published `*.manifest.json` with `{"files":[]}`. `temper update` still installs the
genuine attested archive — but see F2 for what it then writes to disk.

**Fix shape:** a floor at all three consumers — the manifest MUST list `temper` (and on
unix `lib/libonnxruntime.*`), or the verdict is `Mismatch`/hard error, never `Verified`.

---

## F2 — HIGH/MEDIUM. `temper update` attests the archive but never the manifest it persists as the durable baseline

The two surfaces disagree about which object needs provenance, and the one that writes
permanent state to disk is the one that skips it.

- `version.rs:395-413` deliberately attests the **manifest's** digest, with an explicit
  rationale: *"an attacker … who tampers the manifest … must not be able to borrow the
  archive's real attestation to vouch for a manifest it says nothing about."*
- `update.rs:513-523` downloads **both** archive and manifest, then attests **only** the
  archive: `let digest = sha256_of_file(&archive_path)?; …
  verify_release_attestation_online(&client, &digest, tag)`. The manifest is never hashed
  and never looked up, even though `build-cli-binaries.yml:215-218` lists it as its own
  `subject-path` entry precisely so it can be.
- `install.sh:332` — `cp "$TMPDIR/$MANIFEST" "$INSTALL_DIR/$MANIFEST_FILENAME"`. That
  unattested manifest becomes the **sole baseline** for every future
  `temper version --verify` (`version.rs:191-202` → `manifest::load_from_dir`).

**Failure scenario.** Attacker gets release-asset write for one hour. They cannot touch
the archive (`update.rs` would 404 on the attestation lookup for a swapped digest, and a
re-uploaded genuine archive from another tag fails `expected_identity` — that part works).
They replace only the manifest with `files: []`. Every machine that runs `temper update`
in that window installs **genuine, attested bytes** — and persists a vacuous
`.temper-manifest.json`. From then on, offline `temper version --verify` reports
**`verified`** on that host regardless of any local tampering, permanently, long after
the attacker is gone and the real manifest is restored. Threat 1's tamper-detection is
silently destroyed by an attacker who never successfully tampered with anything.

**Fix shape:** one more call in `download_and_verify_release` —
`verify_release_attestation_online(&client, &sha256_of_file(&manifest_path)?, tag)`.
The helper already exists; the workflow already attests the subject.

---

## F3 — MEDIUM. Threat 4: the model composes; the ORT lib does not

The two guards on the **model** genuinely compose. `temper-cli`'s `embed` feature maps to
`temper-ingest/embed-download` (`crates/temper-cli/Cargo.toml:64`), so the shipped CLI
takes `embed.rs:613-639`, which resolves the model binary-adjacent and calls
`verify_model_file(&model_path)` (`embed.rs:631`) before load; the constant comes from
`build.rs` hashing the same committed file the workflow copies into `staging/models/`
(`build-cli-binaries.yml:147-154`). Same commit, same bytes — no gap. Manifest coverage is
real corroboration (`emit-manifest.sh` globs the whole staging tree, so
`models/model_quantized.onnx` is listed), and it moves detection to install time as the
design claims.

**The ORT lib is the asymmetry, and it is the one that is executable code.**

- `embed.rs:180-183` — `ort::init_from(path)` on `<exe_dir>/lib/libonnxruntime.*` with
  **no hash check at any point**.
- `embed.rs:162` — the `ORT_DYLIB_PATH` env override is likewise unverified, while the
  *model* env override four hundred lines later is verified with the comment
  (`embed.rs:511-514`): *"A path handed in by env is exactly as capable of being the wrong
  model as one found on disk — and an unchecked override is a hole straight through the
  lock."* The identical reasoning was not applied to the dylib.

**Failure scenario.** A local attacker with no privileges beyond write access to
`~/.local/share/temper/lib/` replaces `libonnxruntime.so`. It is `dlopen`'d into temper's
process on the next embed. Nothing in the runtime notices; the model's compiled-in pin
does not cover it. Detection requires the user to *manually* run `temper version --verify`.
Swapping the **model** in the same directory is caught at load and refused. Data has two
guards; code has one, and it is a one-shot install-time check.

---

## F4 — LOW/MEDIUM. `ReleaseManifest.version` and `.target` look like bindings and are not

`rg '\.target|\.version' crates/temper-cli/src/{manifest.rs,commands/version.rs,commands/update.rs}`
→ the only hits are in `#[cfg(test)]` blocks. In production the two fields are parsed and
discarded. The F1 demo above installed on **aarch64-apple-darwin** and persisted a manifest
declaring `"target": "x86_64-unknown-linux-gnu"`; nothing objected.

**Failure scenario.** Attestation identity (`attest.rs:167-171`) binds repo + workflow file
+ tag — **not the matrix target**. All three targets of one tag share one identity string.
An attacker with release-asset write serves the genuine *darwin* archive **and** its
genuine *darwin* manifest under the linux asset names for the same tag. Both attest
cleanly on `temper update`; the per-file check passes (archive and manifest agree with each
other). The only thing that stops it is `install.sh:289` — `"$STAGING/temper" --version`
failing to exec a Mach-O on Linux. That is a DoS, not a compromise, so the severity is
bounded — but the exec-gate is a runnability test standing in for a provenance check, and
`manifest.target` is sitting right there unread.

---

## F5 — LOW. The digest binding — the whole point of `verify_release_attestation` — is untested

`attest.rs`'s suite proves the chain-to-pinned-root (`fixture_verifies_against_pinned_root…`),
identity scoping (`wrong_repo_identity_is_rejected`), error classification, and identity
string shape. There is **no test that a correct-identity bundle with a wrong digest is
rejected** — feasible today under the permissive-policy fixture. If a `sigstore-verify`
bump ever stopped checking DSSE subjects, every test in the file still passes.

---

## Not findings — defensible as designed

- **Fresh `curl | sh` does not resist threat 2.** The `.sha256` sidecar is still
  self-asserted and the manifest download (`install.sh:181-184`) is unverified — an actor
  with release-asset write can swap archive + sidecar + a *coherent* manifest and
  install.sh will happily verify their artifact against their own hashes. This is declared
  in the design ("Rejected: mandatory attestation verification at fresh install") for a
  real reason (bootstrapping a verifier is its own unsolved trust problem), and the docs
  route provenance to `--verify --online` and out-of-band `gh attestation verify`.
  *Recommendation only:* `install.md:30-40`'s "4. Verifies the checksum" carries no caveat
  that the checksum sits beside the object it describes. The design's own §"Current state"
  makes exactly this point; the user-facing doc should too. Note also that the design's
  mermaid promised install.sh "attestation — best-effort via gh" and no such call exists —
  it became a manual doc step. That is a defensible narrowing, but it is an undeclared
  divergence from the approved design.
- **Windows `unverifiable`** — emergent from `load_from_dir → None`, stated in
  `WINDOWS_REFUSAL`, `install.md`, and the module docs. Correct.
- **Offline `--verify`'s limitation** — disclaimed in the payload itself
  (`OFFLINE_VERIFY_NOTE`) and tested.
- **Pinned trust root over TUF** — cost stated, failure classes distinguishable, never
  degrades to a warning. Correct call.
- **Downgrade on update** — `is_strictly_newer` (`update.rs:654`) blocks the unpinned path
  from rolling back to an older "latest". Real protection, tested.
- **install.sh's awk manifest parser fails closed** on compact (non-jq-pretty) JSON: the
  `^[^"]*"path": *"` anchor does not match, the pair emitted is garbage, and the file check
  errors. A format coupling with the Rust consumer (which accepts any valid JSON), but it
  fails in the safe direction.

---

## Trace: "GitHub built an artifact" → "these bytes are on my disk"

| Hop | Verified | Assumed | Substitution possible? |
|---|---|---|---|
| source → tag | — | that the tag points at reviewed source, and that `build-cli-binaries.yml` **at that tag** is the file you read on `main` | **Yes**, with repo write. Nothing downstream can tell. |
| tag → build | Fulcio cert SAN = repo + workflow file + `refs/tags/{tag}`; issuer = GitHub OIDC (`attest.rs:152-158`) | GitHub's OIDC, Fulcio, Rekor | No, for release-asset-write-only |
| build → attestation | DSSE sig + Rekor inclusion, chain to **pinned** root (`attest.rs:139-159`) | the pinned root blob is genuine (hand-committed `gh attestation trusted-root` output; no CI check that it still matches upstream) | Only with repo write |
| release assets → client | **update:** archive digest ✓ attested; **manifest ✗ never attested (F2)**; **fresh install:** archive vs a self-asserted sidecar, manifest not verified at all | GitHub TLS; `api.github.com` attestations-by-digest | **Yes** for the manifest on update; **yes** for everything on fresh install |
| archive → extracted files | every manifest-listed file re-hashed (`update.rs:448`, `install.sh:275`) | the manifest is complete (**F1**: it need not be) | Yes, via a truncated/empty manifest |
| extracted → installed | exec gate + post-swap re-check + atomic rollback (`install.sh:289-343`) | — | No — this part is genuinely solid |
| installed → later | **nothing** | that no one touched the directory since | **Yes** (F3) |

**Does installed state stay bound to verified state? No — and by construction it cannot.**
The manifest is written once (`install.sh:332`) and never re-consulted at runtime. `temper`
does not self-verify on startup. Post-install drift is detected only by an on-demand
`temper version --verify`, whose baseline is a file in the same directory as the thing it
measures — and, per F2, a file whose provenance was never checked on the update path.
This is honestly disclaimed for the *offline* case; it is not disclaimed that the baseline
itself arrived unattested.

## Threat verdicts

| # | Threat | Verdict |
|---|---|---|
| 1 | Tampered/corrupt install | **Partial.** Real for corruption and drift. The verdict's *completeness* is defined by a manifest with no minimum coverage (F1) whose provenance is unchecked on the update path (F2). |
| 2 | Compromised release artifact | **Resisted on `temper update`** for the archive — tag + workflow identity means release-asset write alone cannot forge it, and cross-tag replay is caught. **Not resisted on fresh install** (declared). **Not resisted for the manifest object on update** (F2). |
| 3 | Compliance / third-party audit | **Resisted.** `gh attestation verify` needs nothing of ours; `--verify --online` attests the manifest independently. Bounded by what an attestation can mean at all (see below). |
| 4 | Model + ORT integrity | **Model: resisted**, two guards that genuinely compose. **ORT lib: install-time only** — executable code with no load-time pin and an unverified env override (F3). |

## Where residual trust lives

**After all of this, the user still takes on faith that the workflow file at that tag was
the one you reviewed.** The attestation binds *builder* and *tag*; it never binds *source*.
Anyone with repo write can push a tag whose `build-cli-binaries.yml` compiles a backdoor,
and it will verify perfectly — pinned root, Rekor proof, exact identity — on every path,
forever. That is inherent to build provenance and not a defect here, but it is the honest
ceiling on what a `verified` verdict earns, and it is the one thing none of these three
surfaces says out loud.

Three smaller residuals sit under it: the bootstrap (`curl raw.githubusercontent.com/main/
install.sh | sh` is TLS-only, unsigned, unpinned — and is what *every* trust-root failure
message names as the recovery); the pinned trust root blob (hand-committed, with
`releasing.md:118` telling a human to re-derive it and no CI check that it still matches
upstream); and the manifest's completeness (F1 — nothing asserts it covers anything).
