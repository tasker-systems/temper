#!/usr/bin/env bash
# =============================================================================
# Temper — rtk PreToolUse hook (Bash)
# =============================================================================
#
# Wired into .claude/settings.json as a PreToolUse/Bash hook. Delegates to
# `rtk hook claude` (https://github.com/rtk-ai/rtk), which reads the tool-call
# JSON on stdin and rewrites verbose dev commands (git, cargo, test runners, …)
# to their token-compact rtk equivalents before execution — 60-90% fewer tokens
# on common commands, with the full output still one `rtk proxy <cmd>` away.
#
# PRESENCE-GATED, BY DESIGN. If rtk isn't on PATH this exits 0 with no output,
# which Claude Code reads as "no change — run the command as written." So the
# committed hook is a true no-op for anyone who hasn't installed rtk (local devs
# who haven't opted in, or a web session where the install failed). The web
# SessionStart setup (tools/bin/setup-claude-web.sh) installs the rtk binary,
# and that install is exactly what activates this hook in cloud sessions.
#
# PROTOCOL NOTE: stdout must carry ONLY rtk's JSON. Anything else corrupts the
# hook protocol and silently disables it, so the guard below writes nothing to
# stdout and `exec`s straight into rtk (which forwards stdin and owns stdout).
# =============================================================================

# The web setup persists ~/.local/bin onto PATH, but hooks may run with a leaner
# environment — re-add the standard install dirs so `rtk` is found either way.
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"

# rtk absent → pass the command through unchanged (silent, exit 0).
command -v rtk >/dev/null 2>&1 || exit 0

exec rtk hook claude
