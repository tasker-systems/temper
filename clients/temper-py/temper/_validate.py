"""What a caller is allowed to hand this package, checked once, at the seam.

Three kinds of value arrive from a caller and then get put somewhere that cannot
defend itself:

* An ENDPOINT — `configure(base_url=...)`, `ClientCredentials(token_url=...)`. Every
  API request puts a bearer token on the first; every mint puts the client_secret on
  the second. So the scheme is not cosmetic, and neither is userinfo: `urlsplit` will
  happily accept ``https://id:secret@host/`` and the secret then rides in every error
  message that names the URL.
* An OPAQUE SECRET or IDENTIFIER — a token, a client id, a device id. These end up in
  an HTTP header or a form field. The failure that actually happens in the field is
  not an attack: it is ``TEMPER_M2M_CLIENT_SECRET=$(cat secret.txt)`` keeping the
  trailing newline, which produces an `invalid_client` no amount of squinting at the
  secret explains. Rejecting it here names it.
* A plain non-empty string.

Checked at CONSTRUCTION, not at first use. `Connection` builds its `ApiClient` lazily,
so a value validated in `_build()` would surface its error on the first API call —
several layers and possibly several minutes from the `configure()` that caused it.
"""

from __future__ import annotations

import ipaddress
from urllib.parse import SplitResult, urlsplit

#: Hostnames that are the local machine by definition. `.localhost` is reserved for
#: exactly this by RFC 6761 §6.3, and Docker/CI setups do use `foo.localhost`.
_LOOPBACK_NAMES = frozenset({"localhost"})


def require_str(value: object, name: str) -> str:
    """A non-empty `str`, and nothing more."""
    if not isinstance(value, str) or not value:
        raise ValueError(f"{name} must be a non-empty str")
    return value


def require_opaque(value: object, name: str) -> str:
    """A non-empty `str` safe to put in a header value or a form field, verbatim.

    Rejects rather than strips. A stripped value is a guess about what the caller
    meant, and the guess is wrong precisely when it matters: a secret with a stray
    space in the MIDDLE is a different secret, not a formatting slip, and silently
    sending a trimmed one would turn a clear error into a 401 with no cause attached.

    The whitespace rule is also what keeps a `\\r\\n` out of an `Authorization` or
    `X-Temper-Device-Id` header. urllib3 v2 rejects such a header itself — but it
    does so on the request, wrapped in a transport error, with nothing pointing back
    at the `configure()` or `BearerToken(...)` that introduced it.
    """
    text = require_str(value, name)
    if text != text.strip():
        raise ValueError(
            f"{name} has leading or trailing whitespace; "
            f"strip it at the source (a secret read from a file keeps its newline)"
        )
    if any(ch.isspace() for ch in text):
        raise ValueError(f"{name} must not contain whitespace")
    # Cc/Cf and friends: never part of a real token, and a `\r\n` here is a header
    # split. `str.isprintable()` is false for every control character AND for the
    # separators the whitespace check above already caught.
    if not text.isprintable():
        raise ValueError(f"{name} must not contain control characters")
    return text


def require_endpoint(
    value: object,
    *,
    name: str,
    allow_insecure_http: bool = False,
) -> SplitResult:
    """An absolute http(s) origin this package is willing to put a secret on.

    Returns the parsed URL so a caller can reuse the parts rather than re-splitting.

    `http://` is refused off the loopback interface: a bearer token and a
    client_secret both travel in the clear over it, to anything on the path. Loopback
    is exempt because that is what a test server and a `temper serve` on your laptop
    are, and `allow_insecure_http=True` is the deliberate opt-out for the case this
    cannot see — a private network where TLS terminates elsewhere. It is a keyword a
    caller has to write, which is the whole point: `verify_ssl=False` used to be a
    typo away.
    """
    text = require_str(value, name)
    # `urlsplit` SILENTLY strips tab, CR and LF anywhere in the URL (CVE-2019-9740's
    # fix). Silently is the problem: a `base_url` with an embedded newline would be
    # accepted here and normalized into something the caller never wrote.
    if not text.isprintable():
        raise ValueError(f"{name} must not contain whitespace or control characters")

    try:
        parts = urlsplit(text)
    except ValueError as exc:  # a malformed IPv6 literal, principally
        raise ValueError(f"{name} is not a parseable URL: {text!r}") from exc

    if parts.scheme not in ("http", "https") or not parts.netloc:
        raise ValueError(f"{name} must be an absolute http(s) URL, got {text!r}")

    # `parts.username` is None for `host:port`, so this catches ONLY a real userinfo
    # section. Refused rather than dropped: a caller who wrote credentials into the
    # URL meant them to authenticate something, and quietly discarding them would
    # produce a 401 whose cause is invisible.
    if parts.username is not None or parts.password is not None:
        raise ValueError(
            f"{name} must not carry userinfo (user:password@); "
            f"pass credentials to ClientCredentials or BearerToken instead"
        )

    try:
        # Accessing it is the check: `port` raises for one out of range or not a number.
        _ = parts.port
    except ValueError as exc:
        raise ValueError(f"{name} has an invalid port: {text!r}") from exc

    if parts.query or parts.fragment:
        raise ValueError(
            f"{name} must be an origin (optionally with a path prefix), "
            f"not a URL with a query or fragment: {text!r}"
        )

    if parts.scheme == "http" and not (allow_insecure_http or is_loopback(parts.hostname)):
        raise ValueError(
            f"{name} is plaintext http to a non-loopback host, which would put the "
            f"bearer token and client_secret on the wire in the clear; use https, or "
            f"pass allow_insecure_http=True to accept that deliberately"
        )

    return parts


def is_loopback(hostname: str | None) -> bool:
    """Whether `hostname` names this machine, by literal address or by reserved name.

    `urlsplit.hostname` is already lowercased and already has the brackets stripped
    off an IPv6 literal, which is what makes `ip_address` the right test here — and
    it covers the whole 127.0.0.0/8 block, not just 127.0.0.1.
    """
    if not hostname:
        return False
    try:
        return ipaddress.ip_address(hostname).is_loopback
    except ValueError:
        pass
    return hostname in _LOOPBACK_NAMES or hostname.endswith(".localhost")
