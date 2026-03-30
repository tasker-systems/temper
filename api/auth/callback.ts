/**
 * CLI auth callback — exchanges Neon Auth session for a JWT.
 *
 * Flow:
 *   1. CLI opens browser → Neon Auth Google sign-in
 *   2. Neon Auth redirects here with session cookies set
 *   3. We call /auth/token on Neon Auth (same-origin cookies work via server-side fetch)
 *   4. Return page showing JWT + optional redirect to CLI localhost callback
 *
 * Query params:
 *   - neon_auth_session_verifier: session verifier from Neon Auth callback
 *   - cli_port: (optional) localhost port for CLI callback — if set, redirects with ?token=<jwt>
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

	const url = new URL(req.url);
	const cliPort = url.searchParams.get("cli_port");

	// Forward cookies from the incoming request to Neon Auth /token endpoint
	const cookieHeader = req.headers.get("cookie") || "";

	try {
		const tokenRes = await fetch(`${neonAuthBase()}/token`, {
			headers: {
				Cookie: cookieHeader,
				Accept: "application/json",
			},
		});

		if (!tokenRes.ok) {
			const body = await tokenRes.text();
			return respondWithError(
				`Authentication incomplete (${tokenRes.status}). ${body || "No session found."}`,
				`Try signing in again at: ${signInUrl(cliPort)}`,
				cliPort,
			);
		}

		const data = await tokenRes.json();
		const jwt = data.token || data.access_token || data.jwt;

		if (!jwt) {
			return respondWithError(
				"No JWT in token response",
				`Response: ${JSON.stringify(data)}`,
				cliPort,
			);
		}

		// If CLI port is set, redirect to localhost with the token
		if (cliPort) {
			return new Response(null, {
				status: 302,
				headers: {
					Location: `http://localhost:${cliPort}/callback?token=${encodeURIComponent(jwt)}`,
				},
			});
		}

		// Otherwise show the JWT for manual copy
		return new Response(successPage(jwt), {
			status: 200,
			headers: { "Content-Type": "text/html" },
		});
	} catch (err) {
		const message = err instanceof Error ? err.message : String(err);
		return respondWithError(
			`Token exchange failed: ${message}`,
			"Check NEON_AUTH_URL configuration",
			cliPort,
		);
	}
}

function signInUrl(cliPort: string | null): string {
	const callback = cliPort
		? `https://temperkb.io/api/auth/callback?cli_port=${cliPort}`
		: "https://temperkb.io/api/auth/callback";
	return `/api/auth/login?callbackURL=${encodeURIComponent(callback)}`;
}

function respondWithError(
	title: string,
	detail: string,
	cliPort: string | null,
): Response {
	const retryUrl = signInUrl(cliPort);
	const html = `<!DOCTYPE html>
<html><head><title>temper auth</title>
<style>body{font-family:system-ui;max-width:600px;margin:40px auto;padding:0 20px}
pre{background:#1a1a2e;color:#e0e0e0;padding:16px;border-radius:8px;overflow-x:auto;white-space:pre-wrap}
a{color:#6366f1}</style></head>
<body>
<h2>Authentication Error</h2>
<pre>${escapeHtml(title)}\n\n${escapeHtml(detail)}</pre>
<p><a href="${retryUrl}">Try signing in again</a></p>
</body></html>`;

	return new Response(html, {
		status: 400,
		headers: { "Content-Type": "text/html" },
	});
}

function successPage(jwt: string): string {
	return `<!DOCTYPE html>
<html><head><title>temper auth</title>
<style>body{font-family:system-ui;max-width:600px;margin:40px auto;padding:0 20px}
pre{background:#1a1a2e;color:#e0e0e0;padding:16px;border-radius:8px;overflow-x:auto;white-space:pre-wrap;word-break:break-all}
code{background:#2a2a3e;padding:2px 6px;border-radius:4px}
.success{color:#22c55e}
button{background:#6366f1;color:white;border:none;padding:8px 16px;border-radius:6px;cursor:pointer;font-size:14px}
button:hover{background:#4f46e5}</style></head>
<body>
<h2 class="success">Authenticated!</h2>
<p>Run this in your terminal:</p>
<pre id="cmd">temper auth token ${escapeHtml(jwt)}</pre>
<button onclick="navigator.clipboard.writeText(document.getElementById('cmd').textContent)">Copy command</button>
<p style="margin-top:24px;color:#888">You can close this tab after copying.</p>
</body></html>`;
}

function escapeHtml(s: string): string {
	return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}
