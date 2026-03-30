/**
 * CLI auth callback — passes the session verifier to the CLI's localhost server.
 *
 * After Google sign-in, Neon Auth redirects here with the session verifier.
 * We redirect to the CLI's localhost callback with the verifier. The CLI
 * handles the token exchange server-side using the challenge cookie it
 * saved from the initial sign-in request.
 */

export function GET(req: Request): Response {
	const url = new URL(req.url, "https://temperkb.io");
	const cliPort = url.searchParams.get("cli_port");
	const verifier = url.searchParams.get("neon_auth_session_verifier") || "";

	if (!cliPort) {
		// No CLI port — show the verifier for manual use
		return new Response(
			JSON.stringify({ neon_auth_session_verifier: verifier }),
			{ status: 200, headers: { "Content-Type": "application/json" } },
		);
	}

	// Redirect to CLI's localhost callback with the verifier
	return Response.redirect(
		`http://localhost:${cliPort}/callback?verifier=${encodeURIComponent(verifier)}`,
		302,
	);
}
