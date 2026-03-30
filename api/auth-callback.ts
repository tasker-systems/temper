/**
 * CLI auth callback — exchanges Neon Auth session cookie for a JWT.
 *
 * After Google sign-in, Neon Auth redirects here with session cookies.
 * We server-side fetch /auth/token using those cookies (forwarded from
 * the browser request) and redirect to the CLI's localhost callback
 * with the JWT as a query parameter.
 *
 * Query params:
 *   - cli_port: localhost port for CLI callback redirect
 */

function neonAuthBase(): string {
	return process.env.NEON_AUTH_URL || "";
}

export async function GET(req: Request): Promise<Response> {
	const url = new URL(req.url, "https://temperkb.io");
	const cliPort = url.searchParams.get("cli_port") || "";
	const neonAuth = neonAuthBase();

	if (!neonAuth) {
		return new Response(
			JSON.stringify({ error: "NEON_AUTH_URL not configured" }),
			{ status: 500, headers: { "Content-Type": "application/json" } },
		);
	}

	// Forward ALL cookies from the browser to Neon Auth /token endpoint.
	// The browser sends Neon Auth session cookies because temperkb.io is a
	// trusted domain and the cookies have SameSite=None.
	const cookies = req.headers.get("cookie") || "";

	console.log("[auth-callback] Fetching token with cookies:", cookies.substring(0, 80));

	try {
		const tokenRes = await fetch(`${neonAuth}/token`, {
			headers: {
				Cookie: cookies,
				Accept: "application/json",
			},
		});

		console.log("[auth-callback] Token response status:", tokenRes.status);

		if (!tokenRes.ok) {
			const body = await tokenRes.text();
			console.log("[auth-callback] Token error body:", body);

			// Return an HTML error page with debug info
			return new Response(errorPage(tokenRes.status, body, cliPort), {
				status: 200,
				headers: { "Content-Type": "text/html" },
			});
		}

		const data = await tokenRes.json();
		const jwt = data.token || data.access_token || data.jwt;

		if (!jwt) {
			return new Response(errorPage(200, `No JWT in response: ${JSON.stringify(data)}`, cliPort), {
				status: 200,
				headers: { "Content-Type": "text/html" },
			});
		}

		// If cli_port is set, redirect to localhost with the JWT
		if (cliPort) {
			return Response.redirect(
				`http://localhost:${cliPort}/token?jwt=${encodeURIComponent(jwt)}`,
				302,
			);
		}

		// Otherwise show for manual copy
		return new Response(successPage(jwt), {
			status: 200,
			headers: { "Content-Type": "text/html" },
		});
	} catch (err) {
		const message = err instanceof Error ? err.message : String(err);
		return new Response(errorPage(0, `Fetch error: ${message}`, cliPort), {
			status: 200,
			headers: { "Content-Type": "text/html" },
		});
	}
}

function errorPage(status: number, detail: string, cliPort: string): string {
	const retry = cliPort
		? `/api/auth-login?cli_port=${cliPort}`
		: "/api/auth-login";
	return `<!DOCTYPE html><html><head><title>temper auth</title>
<style>body{font-family:system-ui;max-width:600px;margin:40px auto;padding:0 20px;color:#e0e0e0;background:#0f0f1a}
pre{background:#1a1a2e;padding:12px;border-radius:6px;white-space:pre-wrap}a{color:#6366f1}</style></head>
<body><h2 style="color:#ef4444">Authentication Error</h2>
<pre>Status: ${status}\n${esc(detail)}</pre>
<p><a href="${retry}">Try again</a></p></body></html>`;
}

function successPage(jwt: string): string {
	return `<!DOCTYPE html><html><head><title>temper auth</title>
<style>body{font-family:system-ui;max-width:600px;margin:40px auto;padding:0 20px;color:#e0e0e0;background:#0f0f1a}
pre{background:#1a1a2e;padding:12px;border-radius:6px;white-space:pre-wrap;word-break:break-all}
button{background:#6366f1;color:white;border:none;padding:8px 16px;border-radius:6px;cursor:pointer}
</style></head>
<body><h2 style="color:#22c55e">Authenticated!</h2>
<p>Run this in your terminal:</p>
<pre id="cmd">temper auth token ${esc(jwt)}</pre>
<button onclick="navigator.clipboard.writeText(document.getElementById('cmd').textContent)">Copy command</button>
<p style="margin-top:24px;color:#888">You can close this tab.</p></body></html>`;
}

function esc(s: string): string {
	return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}
