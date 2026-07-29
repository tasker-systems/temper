# Installing Temper

Temper is distributed as a self-contained binary for macOS (Apple Silicon),
Linux (x86_64), and Windows (x86_64). The installer drops a `temper` binary
and a bundled ONNX Runtime library into your home directory and adds `temper`
to your PATH.

No Rust toolchain, no system package manager, no homebrew tap required.

## Quick install

### macOS and Linux

```sh
curl -fsSL https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.sh | sh
```

### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.ps1 | iex
```

> If PowerShell warns about the execution policy, run:
> ```powershell
> powershell -ExecutionPolicy Bypass -c "irm https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.ps1 | iex"
> ```

## What the installer does

1. Detects your OS and CPU architecture.
2. Queries GitHub for the latest release tag.
3. Downloads the matching archive (a `.tar.gz` on macOS/Linux, `.zip` on
   Windows) plus its SHA256 checksum file.
4. Verifies the checksum.
5. **macOS/Linux only:** downloads the release's per-file manifest and checks
   every extracted file (`temper`, the bundled `libonnxruntime`, `LICENSE`)
   against it — see [Per-file manifest verification](#per-file-manifest-verification)
   below. A mismatch aborts before your existing install is touched.
6. Extracts the archive into:
   - macOS/Linux: `~/.local/share/temper/` (respects `$XDG_DATA_HOME`)
   - Windows: `%LOCALAPPDATA%\Programs\temper\`
7. Creates a `temper` entry on your PATH:
   - macOS/Linux: symlinks `~/.local/bin/temper` → the extracted binary
   - Windows: appends the install directory to your user PATH

The archive contains `temper[.exe]`, a bundled `libonnxruntime` for the local
embedding pipeline (used server-side; CLI ingestion routes through the cloud API), and a copy of the
project LICENSE.

## Per-file manifest verification

Each macOS/Linux release publishes a `temper-v<ver>-<triple>.manifest.json`
alongside the archive — sha256 and size for every file the archive ships. This
answers a narrower, stronger question than "did the archive download intact?":
it lets you check that the exact `temper` binary (and the ONNX Runtime library
beside it) sitting on your disk is the one the release actually shipped.

`install.sh` verifies every extracted file against this manifest before
swapping it into your live install directory. If anything disagrees, the
install aborts and **your existing install is left untouched** — the same
atomic-swap-with-rollback machinery that already guards a binary that fails to
run at all now also guards a file that fails to match.

The manifest is written into your install directory
(`~/.local/share/temper/.temper-manifest.json`) only after a successful,
verified install, so `temper version --verify` (below) has something to check
against later.

### Three verdicts, not two

Every verification surface in `temper` — `install.sh`, `temper version
--verify`, `temper update` — reports one of three verdicts, never a bare
pass/fail:

| Verdict | Meaning |
|---|---|
| `verified` | Every file matched the manifest. |
| `mismatch` | At least one file disagreed — names the file(s). |
| `unverifiable` | There is nothing to check against, or the check itself could not run. |

**`unverifiable` is not `mismatch`.** A `cargo install` build has no manifest
beside it; a network hiccup during `--verify --online` means the check never
ran; a Windows install ships no manifest at all today (see
[Windows](#windows-hash-verified-only) below). None of these say anything
about whether your binary is wrong — they say the question couldn't be
answered. Rendering "we cannot tell" as "it is wrong" would be its own kind of
dishonesty, so `temper` never collapses the two.

### `temper version --verify` — offline

```sh
temper version --verify
```

Checks the running binary, the ORT library, and the model against the
manifest **installed beside them** in the same directory. This is real, and it
catches real problems: corruption, a partial extraction, a hand-edited file,
local drift.

**It is not adversarially meaningful, and says so in its own output.** An
actor who could replace your binary could replace the manifest sitting next to
it too — offline verification compares two things it does not independently
trust. Treat a `verified` result here as "this install is internally
consistent," not as proof of provenance.

### `temper version --verify --online` — the one that carries provenance weight

```sh
temper version --verify --online
```

Re-fetches the **published** manifest for your exact version and host triple
from GitHub — rather than trusting the copy sitting beside your binary — and,
once that comparison agrees, verifies GitHub's build-provenance attestation
over the sha256 of **that fetched manifest** against a pinned Sigstore trust
root. (Note the object: the attestation check here covers the manifest's
digest, the exact bytes just compared — not the archive's. `temper update`
checks the archive's digest instead, because on that path the archive is the
object being installed. They are deliberately different checks over different
objects, not two views of the same one.)

This is the audit that answers *"is the temper on my machine byte-identical to
what was published, and can I prove it without taking your word for it?"* — a
compromised manifest sitting beside a compromised binary can no longer hide
behind a same-directory comparison, because the comparison object is now
fetched fresh and independently checked against a signature GitHub's release
workflow produced, not anything on your disk.

A failure anywhere in this chain — network, an unusable pinned trust root, or
a bundle that simply doesn't vouch for this artifact — renders
`unverifiable`, never a false `verified`.

### Out-of-band audit — verify without trusting us at all

Every release archive's build-provenance attestation is independently
checkable with GitHub's own `gh` CLI, with no dependency on `temper` or its
pinned trust root:

```sh
gh attestation verify temper-v0.3.0-aarch64-apple-darwin.tar.gz --repo tasker-systems/temper
```

Download the archive for your platform from
[the releases page](https://github.com/tasker-systems/temper/releases), then
run the command above against it. This is the check to reach for if you don't
want to trust `temper`'s own verification code at all — it goes straight to
GitHub's attestation service.

### Windows: hash-verified only

Windows installs (`install.ps1`) verify the archive checksum but write **no
per-file manifest** and have **no attestation-verified update path** today.
`temper version --verify` on Windows therefore always reports
`unverifiable` — it can never report `verified`, because there is nothing
installed to check against. This is a stated, deliberate gap (revisit when a
community Windows tester opts in to build out real coverage), not a silent
hole: nothing here is meant to imply Windows gets the same guarantee macOS and
Linux do.

## Pinning to a specific version

```sh
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.sh | sh -s -- --version v0.1.0
```

```powershell
# Windows
$script = irm https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.ps1
& ([scriptblock]::Create($script)) -Version v0.1.0
```

## Don't want to pipe to `sh`?

Download the script, read it, then run it:

```sh
curl -fsSL -o /tmp/install-temper.sh https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.sh
less /tmp/install-temper.sh         # inspect
sh /tmp/install-temper.sh           # run
```

Or grab the release tarball directly from
[github.com/tasker-systems/temper/releases](https://github.com/tasker-systems/temper/releases)
and unpack it wherever you like.

## Upgrading

Run the installer again — it overwrites the previous install in place.

On macOS/Linux, a curl-script install can also self-update in place:

```sh
temper update
```

Unlike a fresh install, `temper update` makes attestation verification
**mandatory, with no bypass**: it downloads the archive and manifest itself,
verifies GitHub's build-provenance attestation over the downloaded archive's
own digest against the pinned trust root, checks every manifest file against
that same archive's extracted contents, and only then hands the verified
archive to `install.sh --archive` for the atomic swap — so the object
verified and the object installed are always the exact same download, never a
second fetch of "the same" release. `temper update` refuses on a `cargo
install` build (nothing safe to swap) and on Windows (see
[Windows: hash-verified only](#windows-hash-verified-only)) — both refusals
name the recovery command.

## Uninstalling

### macOS / Linux

```sh
rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/temper"
rm -f "${XDG_BIN_HOME:-$HOME/.local/bin}/temper"
```

### Windows

```powershell
Remove-Item -Recurse -Force "$env:LOCALAPPDATA\Programs\temper"
# Then manually remove the install dir from your user PATH via:
#   rundll32.exe sysdm.cpl,EditEnvironmentVariables
```

## Building from source

If you're on a platform we don't ship binaries for (Linux arm64, Intel Mac,
Windows arm64) or you want a custom build, clone the repo and `cargo install`:

```sh
git clone https://github.com/tasker-systems/temper
cd temper
cargo install --path crates/temper-cli --locked --features embed,extract
```

You'll need:
- A Rust toolchain (install via [rustup](https://rustup.rs))
- A C++ compiler (for transitive deps)
- ONNX Runtime installed on your system if you want local embedding support.
  On macOS, `brew install onnxruntime` suffices.

## Troubleshooting

### "temper: command not found" after install (macOS/Linux)

Your shell's PATH doesn't include `~/.local/bin`. Add it:

```sh
# bash
echo 'export PATH="$PATH:$HOME/.local/bin"' >> ~/.bashrc

# zsh
echo 'export PATH="$PATH:$HOME/.local/bin"' >> ~/.zshrc

# fish
fish_add_path ~/.local/bin
```

Then open a new terminal.

### Windows: "temper : The term 'temper' is not recognized"

Restart your terminal. If the problem persists, log out of Windows and back
in (or reboot) so the updated user PATH propagates.

### Windows: SmartScreen warning

The `temper.exe` binary is currently unsigned. On first run, you may see a
SmartScreen "Windows protected your PC" dialog. Click **More info** →
**Run anyway**. (Code-signing is tracked as a future enhancement.)

### ONNX Runtime not found

The installer bundles `libonnxruntime` next to the `temper` binary for the
embedding pipeline. If you see a library-load error, file an issue at
https://github.com/tasker-systems/temper/issues with the output of:

```sh
temper --version
ls -la ~/.local/share/temper/     # macOS / Linux
dir %LOCALAPPDATA%\Programs\temper # Windows
```

## Running your own instance

The steps above install the `temper` CLI and (by default) leave it
unconfigured. To point it at the hosted service, run `temper init` and choose
the hosted option. To stand up your **own** Temper instance on Vercel + Neon +
Auth0 (API + MCP + CLI — plus an optional [web UI](./self-hosting.md#deploy-the-ui-optional)
configurable against any OIDC provider), see [Self-Hosting](./self-hosting.md).
