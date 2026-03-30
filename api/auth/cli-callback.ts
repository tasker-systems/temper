/**
 * CLI auth callback relay — redirects Auth0's authorization code to the CLI's
 * localhost server. The CLI port is passed via the OAuth2 `state` parameter.
 *
 * Flow: Auth0 redirects here with ?code=...&state={port}
 *       We redirect to http://localhost:{port}?code={code}
 */

export function GET(req: Request): Response {
	const url = new URL(req.url, "https://temperkb.io");
	const code = url.searchParams.get("code");
	const state = url.searchParams.get("state");
	const error = url.searchParams.get("error");
	const errorDescription = url.searchParams.get("error_description");

	if (error) {
		return new Response(
			`Authentication failed: ${error} — ${errorDescription || "unknown error"}`,
			{ status: 400, headers: { "Content-Type": "text/plain" } },
		);
	}

	if (!code || !state) {
		return new Response("Missing code or state parameter", {
			status: 400,
			headers: { "Content-Type": "text/plain" },
		});
	}

	const port = Number.parseInt(state, 10);
	if (Number.isNaN(port) || port < 1024 || port > 65535) {
		return new Response("Invalid port in state parameter", {
			status: 400,
			headers: { "Content-Type": "text/plain" },
		});
	}

	const target = `http://localhost:${port}?code=${encodeURIComponent(code)}`;

	return Response.redirect(target, 302);
}
