/**
 * CLI auth callback — fetches JWT client-side and shows it for copy.
 *
 * After Google sign-in, Neon Auth redirects here. The page uses
 * the Better Auth session (cookies on Neon Auth domain) to fetch
 * a JWT via /auth/token with credentials:include.
 *
 * If cli_port is set, redirects to localhost with the JWT.
 * Otherwise shows it for manual copy via `temper auth token`.
 */

export function GET(req: Request): Response {
	const url = new URL(req.url, "https://temperkb.io");
	const cliPort = url.searchParams.get("cli_port") || "";
	const neonAuth = process.env.NEON_AUTH_URL || "";

	const html = `<!DOCTYPE html>
<html><head><title>temper auth</title>
<style>
body{font-family:system-ui;max-width:600px;margin:40px auto;padding:0 20px;color:#e0e0e0;background:#0f0f1a}
pre{background:#1a1a2e;padding:12px;border-radius:6px;white-space:pre-wrap;word-break:break-all}
.ok{color:#22c55e} .err{color:#ef4444} .loading{color:#a0a0b0}
button{background:#6366f1;color:white;border:none;padding:8px 16px;border-radius:6px;cursor:pointer;font-size:14px}
button:hover{background:#4f46e5} a{color:#6366f1}
</style></head>
<body>
<h2 id="title" class="loading">Completing authentication...</h2>
<pre id="log"></pre>
<div id="success" style="display:none">
  <p>Run this in your terminal:</p>
  <pre id="cmd"></pre>
  <button onclick="navigator.clipboard.writeText(document.getElementById('cmd').textContent)">Copy command</button>
  <p style="margin-top:24px;color:#888">You can close this tab after copying.</p>
</div>
<div id="retry" style="display:none">
  <p><a href="/api/auth-login${cliPort ? `?cli_port=${cliPort}` : ""}">Try signing in again</a></p>
</div>
<script>
const title = document.getElementById("title");
const log = document.getElementById("log");
function show(msg) { log.textContent += msg + "\\n"; }

(async () => {
  show("Fetching token from Neon Auth...");
  try {
    const res = await fetch("${neonAuth}/token", {
      credentials: "include",
      headers: { "Accept": "application/json" }
    });
    show("Status: " + res.status);

    if (!res.ok) {
      title.className = "err";
      title.textContent = "Authentication Error";
      const body = await res.text();
      show(body || "No session found.");
      show("\\nThe session cookies may not have been set.");
      show("Try: 1) Clear cookies and try again");
      show("     2) Use an incognito window");
      show("     3) Check that third-party cookies are enabled");
      document.getElementById("retry").style.display = "block";
      return;
    }

    const text = await res.text();
    show("Response received (" + text.length + " chars)");
    let jwt = null;
    try {
      const data = JSON.parse(text);
      jwt = data.token || data.access_token || data.jwt;
    } catch {
      if (text.startsWith("eyJ")) jwt = text.trim();
    }

    if (!jwt) {
      title.className = "err";
      title.textContent = "No token found";
      show("Response: " + text.substring(0, 300));
      document.getElementById("retry").style.display = "block";
      return;
    }

    show("JWT acquired!");

    // If CLI port is set, redirect the JWT to localhost
    const cliPort = "${cliPort}";
    if (cliPort) {
      show("Sending token to CLI...");
      try {
        window.location.href = "http://localhost:" + cliPort + "/callback?verifier=" + encodeURIComponent(jwt);
      } catch(e) {
        show("Redirect failed: " + e.message);
      }
    }

    // Always show for manual copy as fallback
    title.className = "ok";
    title.textContent = "Authenticated!";
    document.getElementById("success").style.display = "block";
    document.getElementById("cmd").textContent = "temper auth token " + jwt;

  } catch (err) {
    title.className = "err";
    title.textContent = "Error";
    show(err.message);
    show("\\nThis may be a CORS issue. Check Neon Auth CORS settings.");
    document.getElementById("retry").style.display = "block";
  }
})();
</script>
</body></html>`;

	return new Response(html, {
		status: 200,
		headers: { "Content-Type": "text/html" },
	});
}
