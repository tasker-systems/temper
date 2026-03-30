/**
 * CLI auth login — initiates Neon Auth social sign-in.
 *
 * Redirects the browser to the Google OAuth flow via Neon Auth.
 * After sign-in, Neon Auth redirects to /api/auth-callback.
 */

export function GET(req: Request): Response {
	const url = new URL(req.url, "https://temperkb.io");

	// Debug: if ?debug=1, return immediately to confirm function is reachable
	if (url.searchParams.get("debug") === "1") {
		return new Response(
			JSON.stringify({
				ok: true,
				neon_auth: (process.env.NEON_AUTH_URL || "NOT_SET").substring(0, 40),
			}),
			{ status: 200, headers: { "Content-Type": "application/json" } },
		);
	}

	const cliPort = url.searchParams.get("cli_port");
	const provider = url.searchParams.get("provider") || "google";
	const neonAuth = process.env.NEON_AUTH_URL;

	if (!neonAuth) {
		return new Response(
			JSON.stringify({ error: { code: "CONFIG_ERROR", message: "NEON_AUTH_URL not set" } }),
			{ status: 500, headers: { "Content-Type": "application/json" } },
		);
	}

	const callbackBase = "https://temperkb.io/api/auth-callback";
	const callbackURL = cliPort ? `${callbackBase}?cli_port=${cliPort}` : callbackBase;

	// Build the Neon Auth sign-in URL directly instead of POSTing server-side.
	// This avoids async fetch issues in the serverless runtime.
	// We construct the redirect URL that Neon Auth's sign-in/social endpoint would give us.
	const initUrl = new URL(`${neonAuth}/sign-in/social`, "https://temperkb.io");

	const html = `<!DOCTYPE html>
<html><head><title>temper auth</title>
<style>body{font-family:system-ui;max-width:600px;margin:40px auto;padding:0 20px;color:#e0e0e0;background:#0f0f1a}
pre{background:#1a1a2e;padding:12px;border-radius:6px;white-space:pre-wrap;word-break:break-all}</style>
</head><body>
<p id="status">Redirecting to sign in...</p>
<pre id="log" style="display:none"></pre>
<script>
const log = document.getElementById("log");
const status = document.getElementById("status");
function show(msg) { log.style.display="block"; log.textContent += msg + "\\n"; }

(async()=>{
  const target = "${neonAuth}/sign-in/social";
  show("POST " + target);
  show("provider: ${provider}");
  show("callbackURL: ${callbackURL}");
  try {
    const res = await fetch(target, {
      method: "POST",
      headers: {"Content-Type":"application/json"},
      body: JSON.stringify({provider:"${provider}",callbackURL:"${callbackURL}"})
    });
    show("Status: " + res.status);
    const text = await res.text();
    show("Body: " + text.substring(0, 500));
    try {
      const data = JSON.parse(text);
      if(data.url) {
        status.textContent = "Redirecting to Google...";
        show("Redirect: " + data.url.substring(0, 100));
        window.location.href = data.url;
      } else {
        status.textContent = "Unexpected response";
      }
    } catch(e) {
      status.textContent = "Parse error";
      show("Parse error: " + e.message);
    }
  } catch(e) {
    status.textContent = "Request failed";
    show("Fetch error: " + e.message);
    show("This is likely a CORS issue. The Neon Auth service needs to allow https://temperkb.io as an origin.");
  }
})();
</script>
</body></html>`;

	return new Response(html, {
		status: 200,
		headers: { "Content-Type": "text/html" },
	});
}
