/**
 * CLI auth callback relay logic. Auth0 redirects here with ?code=…&state={port};
 * we redirect the code to the CLI's localhost server on that port. The redirect
 * target depends only on `state`, so this is host-neutral — the `host` argument
 * is used solely as a parse base when `rawUrl` is relative.
 */

function plain(body: string, status: number): Response {
  return new Response(body, {
    status,
    headers: { "Content-Type": "text/plain" },
  });
}

export function buildCliCallbackResponse(rawUrl: string, host: string | null): Response {
  const base = `https://${host ?? "localhost"}`;
  const url = new URL(rawUrl, base);

  const error = url.searchParams.get("error");
  if (error) {
    const description = url.searchParams.get("error_description") ?? "unknown error";
    return plain(`Authentication failed: ${error} — ${description}`, 400);
  }

  const code = url.searchParams.get("code");
  const state = url.searchParams.get("state");
  if (!code || !state) {
    return plain("Missing code or state parameter", 400);
  }

  const port = Number.parseInt(state, 10);
  if (Number.isNaN(port) || port < 1024 || port > 65535) {
    return plain("Invalid port in state parameter", 400);
  }

  const location = `http://localhost:${port}?code=${encodeURIComponent(code)}`;
  // Set the Location header manually rather than via `Response.redirect()`:
  // the latter runs the target through WHATWG URL normalization, which appends
  // a trailing slash (`localhost:PORT/?code=…`) the CLI's bare-port listener
  // does not expect. Keep the target byte-exact.
  return new Response(null, {
    status: 302,
    headers: { Location: location },
  });
}
