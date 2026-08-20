# Install Temper

**For individual users** — someone who wants the `temper` CLI on their own machine, against a
hosted or self-hosted deployment.

By the end of this playbook you will have the `temper` binary installed and on your PATH,
ready to run `temper init` and connect to an instance. No Rust toolchain, no system package
manager, no homebrew tap is required.

## Prerequisites

- **A supported platform:** macOS (Apple Silicon), Linux (x86_64), or Windows (x86_64). For
  other platforms (Linux arm64, Intel Mac, Windows arm64), see
  [Building from source](#building-from-source) below.
- **A deployment to point at.** If someone else runs the instance, ask them for the URL. If
  you are standing it up yourself, see [Self-host Temper](./self-host-temper.md).

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
3. Downloads the matching archive (a `.tar.gz` on macOS/Linux, `.zip` on Windows) plus its
   SHA256 checksum file.
4. Verifies the checksum.
5. **macOS/Linux only:** downloads the release's per-file manifest and checks every extracted
   file (`temper`, the bundled `libonnxruntime`, `LICENSE`) against it. A mismatch aborts
   before your existing install is touched. See
   [Release Verification](../concepts/release-verification.md) for what this check does and
   does not prove.
6. Extracts the archive into:
   - macOS/Linux: `~/.local/share/temper/` (respects `$XDG_DATA_HOME`)
   - Windows: `%LOCALAPPDATA%\Programs\temper\`
7. Creates a `temper` entry on your PATH:
   - macOS/Linux: symlinks `~/.local/bin/temper` → the extracted binary
   - Windows: appends the install directory to your user PATH

The archive contains `temper[.exe]`, a bundled `libonnxruntime` for the local embedding
pipeline (used server-side; CLI ingestion routes through the cloud API), and a copy of the
project `LICENSE`.

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

On macOS/Linux, a curl-script install can also self-update:

```sh
temper update
```

`temper update` verifies the release before installing and works for curl-script installs on
macOS/Linux. For the verification it performs, see
[Release Verification](../concepts/release-verification.md); for the full flag reference, see
[`temper update`](../reference/cli/update.md).

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

If you are on a platform we don't ship binaries for (Linux arm64, Intel Mac, Windows arm64)
or you want a custom build, clone the repo and `cargo install`:

```sh
git clone https://github.com/tasker-systems/temper
cd temper
cargo install --path crates/temper-cli --locked --features embed,extract
```

You'll need:

- A Rust toolchain (install via [rustup](https://rustup.rs))
- A C++ compiler (for transitive deps)
- ONNX Runtime installed on your system if you want local embedding support. On macOS,
  `brew install onnxruntime` suffices.

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

Restart your terminal. If the problem persists, log out of Windows and back in (or reboot)
so the updated user PATH propagates.

### Windows: SmartScreen warning

The `temper.exe` binary is currently unsigned. On first run, you may see a SmartScreen
"Windows protected your PC" dialog. Click **More info** → **Run anyway**.

### ONNX Runtime not found

The installer bundles `libonnxruntime` next to the `temper` binary for the embedding
pipeline. If you see a library-load error, file an issue at
https://github.com/tasker-systems/temper/issues with the output of:

```sh
temper --version
ls -la ~/.local/share/temper/     # macOS / Linux
dir %LOCALAPPDATA%\Programs\temper # Windows
```

## Next: initialize the CLI

The installer's exit message points at `temper --help`, but the command you actually want
next is `temper init` — it writes your config and points the CLI at an instance:

```sh
temper init
```

Answer "hosted" if you are connecting to temperkb.io, or provide your instance URL for a
self-hosted deployment. Then continue with [Authenticate](./authenticate.md) to sign in and
request access.

## Further reading

- **Authenticate and request access:** [Authenticate](./authenticate.md).
- **Drive Temper from Claude Code:** [Connect Claude Code](./connect-claude-code.md).
- **Drive Temper from Claude Desktop:** [Connect Claude Desktop](./connect-claude-desktop.md).
- **What release verification does and does not prove:**
  [Release Verification](../concepts/release-verification.md).
- **The trust boundary your CLI session crosses:**
  [The Trust Boundary](../concepts/trust-boundary.md).
- **How Temper addresses contexts and resources:**
  [Contexts and Refs](../concepts/contexts-and-refs.md).
- **Using Temper from the CLI:** [temperkb.io/using-temper](https://temperkb.io/using-temper).
