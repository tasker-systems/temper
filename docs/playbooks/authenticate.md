# Authenticate

**For individual users** — someone who has installed Temper and needs to authenticate against a
hosted or self-hosted instance. This is a prerequisite for every command the users door
offers.

## Outcome

By the end of this page you will have an authenticated CLI session with approved access to a
Temper instance, ready to run `temper warmup` and work with your knowledge base.

## Prerequisites

- **Temper installed** — see [Install Temper](./install-temper.md).
- **A deployment to authenticate against.** If someone else runs the instance, ask them for
  the instance URL. If you are standing it up yourself, see
  [Self-hosting Temper](./self-host-temper.md).

## The sequence

### 1. Initialize the CLI

```bash
temper init
```

This writes `~/.config/temper/config.toml`. Answer "hosted" if you are connecting to
temperkb.io, or provide your instance URL for a self-hosted deployment.

> **Do not use `--no-interactive` with no flags.** It writes `provider = "none"`, and
> `temper auth login` then fails with an internal error. The non-interactive hosted form
> needs all four flags (`--instance-url`, `--auth-domain`, `--auth-client-id`,
> `--auth-audience`), whose hosted values are published constants.

### 2. Sign in

```bash
temper auth login
```

This opens a browser for an OAuth Authorization Code + PKCE flow with a localhost listener.
Your token is cached at `~/.config/temper/auth.json` (0600 — a different file from
`config.toml`).

Check your status at any time:

```bash
temper auth status
```

### 3. Request access

**This step is required for every new user.** A brand-new signup is born *denied* —
deliberately. The community edition has no paywall; an admin approving access *is* the
access-control mechanism. Team invitations do not confer access; requesting access moves you
from `denied` to `requested`, which is still not `approved`.

```bash
temper auth request-access --message "Your message to the admin"
```

### 4. Wait for an admin to approve

Nothing before this point fails — your token is valid, your profile exists, your team
memberships are recorded. But every data call will return `403 SYSTEM_ACCESS_REQUIRED` until
an admin approves you. Poll with:

```bash
temper auth status
```

Once your standing shows `approved`, you can proceed.

### 5. Warm up

```bash
temper warmup --context @me/default
```

Note: `warmup` requires a context **ref** (`@me/default`, `@handle/slug`, or a UUID) and
rejects a bare name. This is different from `temper pull`, which takes a bare positional name
(`temper pull default`).

## Further reading

- **What the trust boundary is and why access is gated:**
  [The Trust Boundary](../concepts/trust-boundary.md).
- **What a context is and how to address one:**
  [Contexts and Refs](../concepts/contexts-and-refs.md).
- **Using Temper from the CLI:** [temperkb.io/using-temper](https://temperkb.io/using-temper).
