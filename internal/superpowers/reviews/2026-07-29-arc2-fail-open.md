# Arc-2 adversarial review — FAIL-OPEN HUNT

**Branch:** `jct/binary-attestation-manifest-verification` (PR #573)
**Lens:** every path where verification can be *skipped* rather than *failed*.
**Method:** read-only. No `cargo`. All claims below are backed by probes run against the real
`scripts/install/install.sh` and `.github/scripts/release/emit-manifest.sh` in a scratch dir, or by
line citations. Nothing in the repo was modified.

**Verdict:** the Rust attestation layer (`attest.rs`, `attestation_fetch.rs`,
`version.rs::finish_online_verdict`) fails closed everywhere I could reach it — I could not construct
a single input that turns an attestation *error* into a pass. The fail-opens are all in the
**manifest** layer, and they share one root cause: **an empty entry list is indistinguishable from a
clean verification.** Two of them (F1, F2) let a demonstrably-tampered binary install with exit 0
and the words `Verifying file manifest...` printed.

---

## Findings

### F1 — HIGH — `install.sh`'s manifest parser degrades to a silent no-op on any manifest not laid out in jq's default pretty-print

`manifest_pairs()` (`install.sh:246-262`) is a two-rule awk pass that depends on `"path":` and
`"sha256":` landing on **separate lines**, in that order. The `/"path":/` rule ends in `next`, so on
a **single-line** manifest the `"sha256":` rule never fires and the function emits **zero pairs**.
`verify_manifest_against_dir` then runs its `while read` loop zero times, `CHECK_FAILED` stays `0`,
the function returns success, and the install proceeds.

Parser behaviour, isolated (extracted verbatim from `install.sh`):

| Manifest shape | pairs emitted |
|---|---|
| jq pretty (`{ "path": …\n "sha256": … }`) | 2 (correct) |
| compact one-line JSON | **0** |
| `"files": []` | **0** |
| zero-byte file | **0** |
| an HTML error page | **0** |
| keys reordered (`sha256` before `path`) | 1, with `path` = the *previous* entry's path |

End-to-end probe. Archive contains a `temper` that prints `PWNED`; the manifest lists a sha256 of
completely different content (`honest-bytes`). Only the JSON *layout* differs between probes 1 and 4:

```
== PROBE 1: compact (one-line) manifest listing a sha that does NOT match the archive ==
     Extracting...
     Verifying file manifest...
     Verifying the new binary runs...
     Installing to .../inst1...
   ✓ Installed temper v0.3.0 to .../inst1
   exit=0
   >>> INSTALLED ANYWAY: echo PWNED >&2

== PROBE 4 (control): SAME manifest content, jq-pretty layout ==
   error: temper has sha256 2a89b1b7… expected 460531c0…
   error: file manifest verification failed; your existing install was left untouched.
   >>> correctly refused
```

Probe 3 (zero-byte manifest) installs identically.

**Reachability.** Not attacker-reachable through today's GitHub download path: `curl -fsSL` rejects
non-2xx so an error page never lands, TLS covers the transport, and `emit-manifest.sh` uses jq's
default pretty output. The danger is a **producer-side regression**: one `jq -c`, one
`--compact-output`, one jq release that changes default formatting, or one added key that shifts
line order, and the per-file gate becomes a no-op for **every user, silently, forever** — same
stdout, same exit 0, no warning. `test-install.sh` cannot see it: its tampered-archive tests all
build the manifest through `emit-manifest.sh`, so they only ever exercise the one layout that works.
A gate whose failure mode is "prints the same thing and passes" is the exact shape this lens exists
to find.

**Fix shape.** Cross-check the count: `grep -c '"path":' "$MANIFEST"` against the number of pairs the
loop actually consumed, and hard-fail if they disagree **or if either is zero**. That turns every
parse degradation — present and future — into a loud failure instead of a vacuous pass.

---

### F2 — HIGH — a manifest with `files: []` reads as `Verified` at every layer, including the attested one

There is **no** `files.is_empty()` guard anywhere in the chain (`rg 'is_empty|files\.len'` over
`manifest.rs`, `update.rs`, `version.rs`, `install.sh`, `emit-manifest.sh` returns nothing relevant).

- `manifest::verify_dir` (`manifest.rs:84-110`): the `for entry in &manifest.files` loop runs zero
  times, `mismatches.is_empty()` is true, → `Verdict::Verified`.
- `install.sh`: zero pairs (F1's table), → pass.
- `emit-manifest.sh` on an empty staging dir exits **0** and emits, without complaint:
  ```json
  { "version": "0.3.0", "target": "t", "files": [] }
  ```

Consequence, and this is the part attestation does *not* save you from: `build-cli-binaries.yml:206`
attests the manifest as its own `subject-path` subject. An empty manifest is attested **faithfully**.
So a producer-side regression (a `STAGING` path change, a `find` that matches nothing) publishes an
empty manifest and then:

- `install.sh` installs any archive contents (probe 2: installed a `PWNED` binary),
- `update.rs::verify_archive_against_manifest` returns `Verdict::Verified` → `Ok(())`,
- `temper version --verify --online` fetches it, `verify_dir` → `Verified`,
  `verify_manifest_attestation` succeeds over its real digest, `finish_online_verdict` → **`verified`**.

Every gate green, nothing checked, and the strongest surface in the design (`--online`, the one
`ONLINE_VERIFY_NOTE` says "carries provenance") is the one reporting it. Note that F2 needs no
attacker at all — a build bug is sufficient, and the output is indistinguishable from success.

**Fix shape.** Three cheap guards, none of which can regress a real release:
1. `emit-manifest.sh`: hard-fail if `ENTRIES` is empty (a release with zero files is never valid).
2. `manifest::verify_dir`: `files.is_empty()` → `Verdict::Unverifiable { reason: "manifest lists no
   files" }`. Not `Mismatch` — "we cannot tell" is the honest verdict, and it keeps the trichotomy's
   own rule.
3. `install.sh`: zero pairs → `error:` + exit 1 (subsumed by F1's count check).

---

### F3 — MEDIUM — manifest paths are joined with no containment check, so a manifest can aim verification outside the install dir

`install.sh:271` uses `"$CHECK_DIR/$REL"`; `manifest.rs:87` uses `dir.join(&entry.path)`. Neither
rejects `..` components, and `Path::join` with an **absolute** `path` (`"/bin/sh"`) discards the base
entirely.

Probe 5 — a manifest whose sole entry is `../outside/decoy`, pointing at a file the attacker
pre-placed with a matching hash:

```
== PROBE 5: manifest path escapes the install dir (../) ==
   >>> INSTALLED: manifest 'verified' a file OUTSIDE the install dir; temper itself never hashed
```

**Impact.** Confined to the **offline** `temper version --verify` verdict and to a caller who already
supplies a manifest. It lets an attacker with install-dir write forge `verified` by editing only the
`path` strings — cheaper than recomputing hashes, and it never has to touch the binary's entry at
all. `OFFLINE_VERIFY_NOTE` already disclaims "an attacker, who could replace both", so this is inside
the stated model, but it widens the cheapest forgery. On `--online` the manifest is attested, so it
is not reachable there.

**Fix shape.** Reject an entry whose path is absolute or contains a `..` component — in
`ReleaseManifest` parsing (parse-don't-validate: a `ManifestPath` newtype), and with a `case "$REL"
in /*|*..*) error ;; esac` in the shell loop.

---

### F4 — MEDIUM — files present but unlisted are ignored, so `verified` never means "only what we shipped"

Documented as deliberate (`manifest.rs:81-83`: "Files present in `dir` but absent from the manifest
are ignored"). Probe 6 confirms the install path agrees — an archive carrying an extra
`lib/libonnxruntime.dylib` that the manifest does not list installs clean and verifies clean.

This is defensible as a *policy* (users may put files beside the binary), but it is load-bearing for
what the verdict can honestly claim, because the ORT loader picks the **first existing** candidate:

- `temper-ingest/src/embed.rs:94-96` tries `<exe_dir>/lib/libonnxruntime.dylib` **before**
  `libonnxruntime.so`. A Linux archive ships the `.so`, so an unlisted `.dylib` planted in `lib/`
  wins the search while `--verify --online` still reports `verified`.
- Sharper, and **pre-existing rather than introduced here**: the Linux release is built
  `--features embed,extract` (`build-cli-binaries.yml:88`), which selects the
  `#[cfg(all(target_os = "linux", feature = "embed", not(feature = "embed-download")))]` branch
  (`embed.rs:126-146`). That branch loads `/tmp/libonnxruntime.so` and writes the bundled bytes only
  `if !lib_path.exists()` — so a **pre-planted `/tmp/libonnxruntime.so` is loaded verbatim**, outside
  every manifest and every attestation, on a `verified` install.

Not a defect in this PR's code. Flagged because it caps what a `verified` verdict may be read to mean,
and the notes currently do not mention it. **Recommendation** (not a finding): either extend the
verdict with an `unexpected_files` list, or add one clause to `OFFLINE_VERIFY_NOTE`/`ONLINE_VERIFY_NOTE`
stating that the check covers the listed files only, not the absence of others.

---

### F5 — LOW (design/implementation drift) — the fresh-install door has *no* attestation leg, and does not say so

The design's flow diagram
(`docs/superpowers/specs/2026-07-29-…-design.md:104`) shows the `install.sh` path as
`archive .sha256 — MANDATORY / per-file manifest — MANDATORY / attestation — best-effort via gh`.
`rg 'gh |attestation|cosign' scripts/install/install.sh` returns two hits, **both inside comments**
(lines 129, 328). There is no `gh attestation verify` call, best-effort or otherwise.

So `curl … | sh` — by volume the most-used door — reduces to "GitHub release integrity + TLS": the
archive, its `.sha256`, and the manifest all come from the same origin, and an actor with
release-asset write satisfies all three. `temper update` and `--verify --online` are strictly
stronger. The design explicitly *rejects* mandatory attestation at fresh install (correctly — it
would hard-depend on `gh`), but under the active goal *"no door offers less than another without
saying so"* the asymmetry is observable and unstated: install.sh prints nothing about it.

**Recommendation.** Either implement the best-effort `gh` leg the design promises, or print one line
at the end of a fresh install: *"This install is hash-verified. Run `temper version --verify --online`
for attestation-backed provenance."* One line, and the door declares itself.

---

### F6 — LOW — `--archive` + `--manifest` is a public flag pair that reduces verification to self-consistency

On the `--archive` path (`install.sh:126-137`) the `.sha256` sidecar is never fetched or checked; the
only check is archive-vs-manifest, both supplied by the caller. That is correct for `temper update`
(which verified the attestation first — `update.rs:216`, before `run_installer` at :224) and the
TOCTOU reasoning behind it is sound. But the flags are documented in `--help` as ordinary options on
a `curl | sh` installer, with no statement that they carry no provenance. Requires local exec, so
not an escalation. **Recommendation:** one clause in the `--help` text.

---

### F7 — INFORMATIONAL — `temper update`'s post-install read-back downgrades to a warning

`update.rs:230-242`: `read_installed_version` returns `None` on **any** failure (`.ok()?`, non-zero
exit, empty stdout), and both the `None` arm and the version-mismatch arm emit
`crate::output::warning` and then fall through to a success report with exit 0. This is correct — the
installer already gated on runnability *and* re-verified the manifest against the live `INSTALL_DIR`
(`install.sh:334`) before dropping `OLD`, and the swap is already committed by this point, so failing
here would be worse than warning. Recorded only because "verification failure → warning → proceed" is
literally the shape this lens hunts, and a future reader should know it was examined and is not one.

Minor, non-security: `Some(v) if v.contains(target_version)` is a substring test — `"0.3.0"` matches
an installed `"10.3.01"`.

---

## Paths that correctly fail closed

Stated plainly, because a fail-open review that only lists holes misrepresents the change.

**`attest.rs`.** Every `?` and `map_err` lands in an `AttestError` variant; there is no arm that
returns `Ok` on a failed check, and no fallback if the embedded root fails to parse. The pinned root
is `include_str!` with no network path, so there is nothing to degrade *to*.
`classify_verify_error`'s string matching (`attest.rs:185-196`) can only mis-*label* a failure as
`TrustRootUnusable` vs `NotOurs` — both are `Err`; misclassification never becomes a pass.
`VerificationPolicy::default()` is used unmodified, so `skip_tlog()` is genuinely never called.

**`attestation_fetch.rs`.** 404 → `NotOurs`; 403 → `Network`; any other non-2xx → `Network`;
`attestations: []` → `NotOurs`; no SLSA predicate among the bundles → `NotOurs`; a record with
neither `bundle` nor `bundle_url` → `NotOurs`. `select_slsa_provenance_bundle`'s `if let Ok(...)`
(:292) skips unparseable entries — but the loop falls through to `Err`, so the skip can never become
a pass. The one wart is over-strict, not lax: `fetch_release_attestation_bundle` (:314) `?`s on
*every* record's resolve, so a broken sibling bundle fails the whole lookup.

**`version.rs::finish_online_verdict`** (:423-436). Attestation can only ever **downgrade**
`Verified` → `Unverifiable`; `Ok(())` cannot rescue a `Mismatch` (the `other => other` arm), and the
`if !matches!(…, Verified)` short-circuit at :497 skips the lookup rather than letting it override.
Both directions are pinned by tests. `manifest_attestation_identity` (:388) hashes the exact
`manifest_bytes` that `verify_dir` just consumed — no re-fetch, and the manifest's digest, not the
archive's. That reasoning is right and the module docs argue it correctly.

**`version.rs` degradations are all toward `Unverifiable`, never `Verified`:** unmapped host (:457),
tokio runtime build failure (:463), HTTP client build failure (:473), manifest fetch failure (:482),
manifest parse failure (:487-492), attestation failure of any class (:501).

**The two `canonicalize(&exe).unwrap_or(exe)` fallbacks** (`update.rs:263`, `version.rs:174`) look
like classic `.ok()` fail-opens but are not: on failure `.parent()` resolves to `~/.local/bin`, which
carries no `.temper-manifest.json` and no `lib/libonnxruntime.*` — so `version --verify` yields
`Unverifiable` and `update` yields the `Cargo` refusal. Both closed.

**`install.sh` ordering.** extract → manifest-verify `$STAGING` → run-gate → move `OLD` aside →
swap → symlink → run-gate **and** manifest re-verify against the live `INSTALL_DIR` → *only then*
`cp` the manifest in and `rm -rf "$OLD"`. Nothing installs before verification, the rollback path is
single-sourced, and `OLD` is deliberately excluded from the EXIT trap. Probe 4 confirms the
post-extract gate bites and leaves the prior install untouched; `test-install.sh` isolates the two
gates from each other, which is a genuinely good test.

**`install.sh` shell hygiene.** `sh -n` clean. The `while read` loop is fed by a **heredoc**, not a
pipe, so it runs in the current shell and `CHECK_FAILED` survives — the textbook subshell fail-open
was correctly avoided. The only `|| true` is `chmod +x … || true` (:295), and a failed chmod still
trips the run-gate immediately after. `--archive` without `--manifest` is a hard error, not a silent
skip (:190), and the harness asserts it. `set -u` means a missing `--version` value aborts.

> One latent note, benign today: `verify_manifest_against_dir` is always invoked inside an `if`/`&&`
> condition, which per POSIX disables `set -e` for its **entire body**, including everything it
> calls. Safe now because every failure path sets `CHECK_FAILED` explicitly rather than relying on
> errexit — but a future check added inside that function that expects `set -e` to abort will not.

---

## Recommended order of work

1. **F2 guards** — three one-line changes, and they close the only fail-open that survives a valid
   attestation.
2. **F1 count cross-check** — converts the whole class of parser degradations into a loud failure,
   and makes F2's shell half redundant.
3. **F3 path containment** — cheap, and removes the cheapest offline forgery.
4. **F5 / F6 / F4 notes** — wording, not logic.

A regression test for F1/F2 belongs in `scripts/install/test-install.sh` and must **not** build its
manifest through `emit-manifest.sh` — that is precisely why the existing suite cannot see either bug.
Feed it a hand-written compact manifest and a `"files": []` manifest, and assert both are *rejected*.
