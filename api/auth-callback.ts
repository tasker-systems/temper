/**
 * CLI auth callback — exchanges Neon Auth session verifier for a JWT.
 *
 * Flow:
 * 1. Neon Auth redirects here with ?neon_auth_session_verifier=...
 * 2. We call Neon Auth's own callback endpoint server-side with the verifier
 *    to get session cookies
 * 3. We use those session cookies to call /auth/token for the JWT
 * 4. We redirect to CLI's localhost with the JWT
 */

function neonAuthBase(): string {
	return process.env.NEON_AUTH_URL || "";
}

export async function GET(req: Request): Promise<Response> {
	const url = new URL(req.url, "https://temperkb.io");
	const cliPort = url.searchParams.get("cli_port") || "";
	const verifier = url.searchParams.get("neon_auth_session_verifier") || "";
	const neonAuth = neonAuthBase();

	if (!neonAuth) {
		return errorResponse("NEON_AUTH_URL not configured", cliPort);
	}

	if (!verifier) {
		return errorResponse("No session verifier in callback", cliPort);
	}

	console.log("[auth-callback] Got verifier, exchanging for session...");

	try {
		// Step 1: Call Neon Auth's callback endpoint with the verifier
		// to get session cookies. We need to follow the redirect chain
		// and capture set-cookie headers.
		//
		// The verifier endpoint is the social callback with the verifier param.
		// When called, it sets session cookies and redirects to the callbackURL.
		// We use redirect: "manual" to capture the cookies without following.
		const callbackRes = await fetch(
			`${neonAuth}/callback/google?neon_auth_session_verifier=${encodeURIComponent(verifier)}`,
			{ redirect: "manual" },
		);

		console.log("[auth-callback] Callback status:", callbackRes.status);

		// Capture session cookies from the response
		const setCookieHeaders = callbackRes.headers.getSetCookie?.() || [];
		console.log("[auth-callback] Set-Cookie count:", setCookieHeaders.length);

		if (setCookieHeaders.length === 0) {
			const body = await callbackRes.text();
			return errorResponse(
				`No session cookies returned (status ${callbackRes.status}). Body: ${body.substring(0, 200)}`,
				cliPort,
			);
		}

		// Build a cookie string from the set-cookie headers
		const cookies = setCookieHeaders
			.map((h: string) => h.split(";")[0]) // Extract just name=value
			.join("; ");

		console.log("[auth-callback] Forwarding cookies to /token:", cookies.substring(0, 80));

		// Step 2: Use the session cookies to get a JWT
		const tokenRes = await fetch(`${neonAuth}/token`, {
			headers: {
				Cookie: cookies,
				Accept: "application/json",
			},
		});

		console.log("[auth-callback] Token status:", tokenRes.status);

		if (!tokenRes.ok) {
			const body = await tokenRes.text();
			return errorResponse(
				`Token request failed (${tokenRes.status}): ${body}`,
				cliPort,
			);
		}

		const data = await tokenRes.json();
		const jwt = data.token || data.access_token || data.jwt;

		if (!jwt) {
			return errorResponse(
				`No JWT in response: ${JSON.stringify(data).substring(0, 200)}`,
				cliPort,
			);
		}

		console.log("[auth-callback] Got JWT, redirecting to CLI");

		// Step 3: Redirect to CLI's localhost with the JWT
		if (cliPort) {
			return Response.redirect(
				`http://localhost:${cliPort}/token?jwt=${encodeURIComponent(jwt)}`,
				302,
			);
		}

		// No CLI port — show for manual copy
		return new Response(successPage(jwt), {
			status: 200,
			headers: { "Content-Type": "text/html" },
		});
	} catch (err) {
		const message = err instanceof Error ? err.message : String(err);
		console.error("[auth-callback] Error:", message);
		return errorResponse(`Error: ${message}`, cliPort);
	}
}

function errorResponse(detail: string, cliPort: string): Response {
	const retry = cliPort ? `/api/auth-login?cli_port=${cliPort}` : "/api/auth-login";
	const html = `<!DOCTYPE html><html><head><title>temper auth</title>
<style>body{font-family:system-ui;max-width:600px;margin:40px auto;padding:0 20px;color:#e0e0e0;background:#0f0f1a}
pre{background:#1a1a2e;padding:12px;border-radius:6px;white-space:pre-wrap;word-break:break-all}a{color:#6366f1}</style></head>
<body><h2 style="color:#ef4444">Authentication Error</h2>
<pre>${esc(detail)}</pre>
<p><a href="${retry}">Try again</a></p></body></html>`;
	return new Response(html, { status: 200, headers: { "Content-Type": "text/html" } });
}

function successPage(jwt: string): string {
	return `<!DOCTYPE html><html><head><title>temper auth</title>
<style>body{font-family:system-ui;max-width:600px;margin:40px auto;padding:0 20px;color:#e0e0e0;background:#0f0f1a}
pre{background:#1a1a2e;padding:12px;border-radius:6px;white-space:pre-wrap;word-break:break-all}
button{background:#6366f1;color:white;border:none;padding:8px 16px;border-radius:6px;cursor:pointer}</style></head>
<body><h2 style="color:#22c55e">Authenticated!</h2>
<p>Run this in your terminal:</p>
<pre id="cmd">temper auth token ${esc(jwt)}</pre>
<button onclick="navigator.clipboard.writeText(document.getElementById('cmd').textContent)">Copy command</button>
<p style="margin-top:24px;color:#888">You can close this tab.</p></body></html>`;
}

function esc(s: string): string {
	return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}
