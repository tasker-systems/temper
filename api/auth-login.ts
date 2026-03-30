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
	if (req.method !== "GET") {
		return new Response(JSON.stringify({ error: "Method not allowed" }), {
			status: 405,
			headers: { "Content-Type": "application/json" },
		});
	}

	const url = new URL(req.url, "https://temperkb.io");
	const cliPort = url.searchParams.get("cli_port");
	const provider = url.searchParams.get("provider") || "google";

	// Build callback URL that preserves cli_port
	const callbackBase = "https://temperkb.io/api/auth-callback";
	const callbackURL = cliPort
		? `${callbackBase}?cli_port=${cliPort}`
		: callbackBase;

	try {
		const res = await fetch(`${neonAuthBase()}/sign-in/social`, {
			method: "POST",
			headers: {
				"Content-Type": "application/json",
				Origin: "https://temperkb.io",
			},
			body: JSON.stringify({ provider, callbackURL }),
		});

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
					},
				}),
				{ status: 502, headers: { "Content-Type": "application/json" } },
			);
		}

		// Redirect browser to Google sign-in
		return new Response(null, {
			status: 302,
			headers: { Location: redirectUrl },
		});
	} catch (err) {
		const message = err instanceof Error ? err.message : String(err);
		return new Response(
			JSON.stringify({
				error: { code: "AUTH_ERROR", message },
			}),
			{ status: 500, headers: { "Content-Type": "application/json" } },
		);
	}
}
