/**
 * CLI auth callback — renders a page that fetches a JWT from Neon Auth.
 *
 * Flow:
 *   1. CLI opens browser → Neon Auth Google sign-in
 *   2. Neon Auth redirects here — session cookies are set on the Neon Auth domain
 *   3. We render a client-side page that fetches /auth/token with credentials:include
 *      (the browser has the session cookies, the server does not)
 *   4. Page shows the JWT for `temper auth token` or redirects to CLI localhost
 *
 * Query params:
 *   - cli_port: (optional) localhost port — if set, auto-redirects with ?token=<jwt>
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
	const authBase = neonAuthBase();

	// Render a client-side page that fetches the JWT using browser cookies
	return new Response(tokenFetchPage(authBase, cliPort), {
		status: 200,
		headers: { "Content-Type": "text/html" },
	});
}

function tokenFetchPage(authBase: string, cliPort: string | null): string {
	const cliRedirect = cliPort
		? `"http://localhost:${escapeHtml(cliPort)}/callback?token=" + encodeURIComponent(jwt)`
		: "null";

	return `<!DOCTYPE html>
<html><head><title>temper auth</title>
<style>
body { font-family: system-ui; max-width: 600px; margin: 40px auto; padding: 0 20px; color: #e0e0e0; background: #0f0f1a; }
pre { background: #1a1a2e; color: #e0e0e0; padding: 16px; border-radius: 8px; overflow-x: auto; white-space: pre-wrap; word-break: break-all; }
.success { color: #22c55e; }
.error { color: #ef4444; }
.loading { color: #a0a0b0; }
button { background: #6366f1; color: white; border: none; padding: 8px 16px; border-radius: 6px; cursor: pointer; font-size: 14px; }
button:hover { background: #4f46e5; }
a { color: #6366f1; }
</style></head>
<body>
<div id="loading">
  <h2 class="loading">Completing authentication...</h2>
  <p>Fetching token from Neon Auth...</p>
</div>
<div id="success" style="display:none">
  <h2 class="success">Authenticated!</h2>
  <p>Run this in your terminal:</p>
  <pre id="cmd"></pre>
  <button onclick="navigator.clipboard.writeText(document.getElementById('cmd').textContent)">Copy command</button>
  <p style="margin-top:24px;color:#888">You can close this tab after copying.</p>
</div>
<div id="error" style="display:none">
  <h2 class="error">Authentication Error</h2>
  <pre id="error-detail"></pre>
  <p><a href="/api/auth-login${cliPort ? `?cli_port=${escapeHtml(cliPort)}` : ""}">Try signing in again</a></p>
</div>
<script>
(async () => {
  try {
    const res = await fetch("${escapeHtml(authBase)}/token", {
      credentials: "include",
      headers: { "Accept": "application/json" }
    });

    if (!res.ok) {
      const body = await res.text();
      showError("Token request failed (" + res.status + ")\\n\\n" + (body || "No session found.") +
        "\\n\\nThis usually means the session cookies were not set. " +
        "Make sure third-party cookies are enabled for the Neon Auth domain.");
      return;
    }

    const text = await res.text();
    let jwt = null;

    try {
      const data = JSON.parse(text);
      jwt = data.token || data.access_token || data.jwt;
    } catch {
      if (text.startsWith("eyJ")) jwt = text.trim();
    }

    if (!jwt) {
      showError("No JWT found in response.\\n\\nResponse: " + text);
      return;
    }

    // Check for CLI auto-redirect
    const cliRedirect = ${cliRedirect};
    if (cliRedirect) {
      window.location.href = cliRedirect;
      return;
    }

    // Show for manual copy
    document.getElementById("loading").style.display = "none";
    document.getElementById("success").style.display = "block";
    document.getElementById("cmd").textContent = "temper auth token " + jwt;

  } catch (err) {
    showError("Fetch error: " + err.message +
      "\\n\\nThis is likely a CORS issue. The Neon Auth service may need " +
      "to allow https://temperkb.io as an origin.");
  }
})();

function showError(msg) {
  document.getElementById("loading").style.display = "none";
  document.getElementById("error").style.display = "block";
  document.getElementById("error-detail").textContent = msg;
}
</script>
</body></html>`;
}

function escapeHtml(s: string): string {
	return s
		.replace(/&/g, "&amp;")
		.replace(/</g, "&lt;")
		.replace(/>/g, "&gt;")
		.replace(/"/g, "&quot;");
}
