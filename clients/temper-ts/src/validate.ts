/**
 * What this client is willing to put a secret on, checked once, at the seam.
 *
 * Two URLs arrive from a caller and then carry credentials on every use: the
 * instance origin — every request puts the bearer token on it — and the token
 * URL, where every mint puts the client_secret on the wire. For both, the
 * scheme is not cosmetic: plaintext `http` off the loopback interface puts the
 * credential in the clear, readable by anything on the path.
 *
 * Checked at CONSTRUCTION, not at first use — the same discipline the sibling
 * clients state (`temper-py`'s `temper/_validate.py`, `temper-client`'s
 * `endpoint.rs`): a URL validated when a request is built would surface its
 * error several layers and possibly several minutes from the configuration
 * that caused it.
 *
 * `allowInsecureHttp` is the deliberate opt-out for the case this check cannot
 * see — a private network where TLS terminates elsewhere. It is a keyword a
 * caller has to write, which is the whole point: it must not be a typo away.
 */

/**
 * Whether `hostname` names this machine, by literal address or by reserved
 * name. `URL::hostname` is already lowercased and already has the brackets
 * stripped off an IPv6 literal. Dependency-free on purpose — this module runs
 * in the browser too.
 */
export function isLoopback(hostname: string): boolean {
  // One fully-qualified trailing dot is the same name.
  const host = hostname.replace(/\.$/, "").toLowerCase();
  if (host === "localhost" || host.endsWith(".localhost")) {
    return true;
  }
  // The whole 127.0.0.0/8 block, not just 127.0.0.1. Octet ranges are NOT
  // validated here — `new URL` rejects invalid IPv4 hosts, so this arm is only
  // reachable with well-formed octets via `requireEndpoint`; a direct caller
  // of `isLoopback` with nonsense input gets a harmless over-accept.
  if (/^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(host)) {
    return true;
  }
  return host === "::1" || host === "[::1]";
}

/**
 * An absolute http(s) origin this package is willing to put a secret on.
 *
 * * no whitespace or control characters — the WHATWG parser silently strips
 *   tab/CR/LF (CVE-2019-9740's fix), so an embedded newline would otherwise be
 *   accepted here and normalized into something the caller never wrote
 * * absolute `http`/`https`, with a host
 * * no userinfo — `https://id:secret@host/` would ride the secret in every
 *   error message that names the URL
 * * no query or fragment — the client joins the origin with request paths,
 *   which would bury them mid-URL
 * * `http` only to the loopback interface, unless `allowInsecureHttp`
 *
 * Throws `TypeError` at the seam, matching `requireNonEmpty`'s contract: the
 * client's own plumbing failing before a request is ever sent.
 */
export function requireEndpoint(
  value: string,
  name: string,
  opts: { allowInsecureHttp?: boolean } = {},
): URL {
  if (typeof value !== "string" || value === "") {
    throw new TypeError(`${name} must be a non-empty string`);
  }
  if (/\s/.test(value) || /[\u0000-\u001f\u007f]/.test(value)) {
    throw new TypeError(`${name} must not contain whitespace or control characters`);
  }

  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new TypeError(`${name} is not a parseable URL: ${JSON.stringify(value)}`);
  }

  if ((url.protocol !== "http:" && url.protocol !== "https:") || url.hostname === "") {
    throw new TypeError(`${name} must be an absolute http(s) URL, got ${JSON.stringify(value)}`);
  }

  // Refused rather than dropped: a caller who wrote credentials into the URL
  // meant them to authenticate something, and quietly discarding them would
  // produce a 401 whose cause is invisible.
  if (url.username !== "" || url.password !== "") {
    throw new TypeError(
      `${name} must not carry userinfo (user:password@); pass credentials to ClientCredentials or BearerToken instead`,
    );
  }

  if (url.search !== "" || url.hash !== "") {
    throw new TypeError(
      `${name} must be an origin (optionally with a path prefix), not a URL with a query or fragment`,
    );
  }

  if (url.protocol === "http:" && !(opts.allowInsecureHttp === true || isLoopback(url.hostname))) {
    throw new TypeError(
      `${name} is plaintext http to a non-loopback host, which would put the bearer token ` +
        `and client_secret on the wire in the clear; use https, or pass allowInsecureHttp: true ` +
        `to accept that deliberately`,
    );
  }

  return url;
}
