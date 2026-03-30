/**
 * CLI auth login — initiates Neon Auth social sign-in.
 *
 * Redirects the browser to the Google OAuth flow via Neon Auth.
 * After sign-in, Neon Auth redirects to /api/auth-callback.
 *
 * Query params:
 *   - cli_port: (optional) passed through to callback for localhost redirect
 *   - provider: (optional) OAuth provider, defaults to "google"
 */

export const config = { runtime: "nodejs" };

function neonAuthBase(): string {
	const url = process.env.NEON_AUTH_URL;
	if (!url) {
		throw new Error("NEON_AUTH_URL environment variable is required");
	}
	return url;
}

export default async function handler(req: Request): Promise<Response> {
	const url = new URL(req.url, "https://temperkb.io");
	const cliPort = url.searchParams.get("cli_port");
	const provider = url.searchParams.get("provider") || "google";
	const neonAuth = process.env.NEON_AUTH_URL || "NOT_SET";

	console.log("[auth-login] Starting", { provider, cliPort, neonAuth: neonAuth.substring(0, 30) });

	if (req.method !== "GET") {
		return new Response(JSON.stringify({ error: "Method not allowed" }), {
			status: 405,
			headers: { "Content-Type": "application/json" },
		});
	}

	// Debug: if ?debug=1, return env info without calling Neon Auth
	if (url.searchParams.get("debug") === "1") {
		return new Response(
			JSON.stringify({
				status: "auth-login function reached",
				neon_auth_url: neonAuth.substring(0, 40) + "...",
				provider,
				cli_port: cliPort,
			}),
			{ status: 200, headers: { "Content-Type": "application/json" } },
		);
	}

	const callbackBase = "https://temperkb.io/api/auth-callback";
	const callbackURL = cliPort
		? `${callbackBase}?cli_port=${cliPort}`
		: callbackBase;

	try {
		console.log("[auth-login] Fetching Neon Auth sign-in/social...");
		const res = await fetch(`${neonAuth}/sign-in/social`, {
			method: "POST",
			headers: {
				"Content-Type": "application/json",
				Origin: "https://temperkb.io",
			},
			body: JSON.stringify({ provider, callbackURL }),
			signal: AbortSignal.timeout(10000),
		});
		console.log("[auth-login] Neon Auth responded:", res.status);

		if (!res.ok) {
			const body = await res.text();
			return new Response(
				JSON.stringify({
					error: { code: "AUTH_INIT_FAILED", message: body },
				}),
				{ status: 502, headers: { "Content-Type": "application/json" } },
			);
		}

		const data = await res.json();
		const redirectUrl = data.url;

		if (!redirectUrl) {
			return new Response(
				JSON.stringify({
					error: {
						code: "NO_REDIRECT",
						message: "Neon Auth did not return a redirect URL",
						response: data,
					},
				}),
				{ status: 502, headers: { "Content-Type": "application/json" } },
			);
		}

		console.log("[auth-login] Redirecting to:", redirectUrl.substring(0, 60));
		return Response.redirect(redirectUrl, 302);
	} catch (err) {
		const message = err instanceof Error ? err.message : String(err);
		console.error("[auth-login] Error:", message);
		return new Response(
			JSON.stringify({
				error: { code: "AUTH_ERROR", message },
			}),
			{ status: 500, headers: { "Content-Type": "application/json" } },
		);
	}
}
