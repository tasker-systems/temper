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

	// Redirect to a simple HTML page that does the POST client-side
	const html = `<!DOCTYPE html>
<html><head><title>temper auth</title>
<style>body{font-family:system-ui;max-width:600px;margin:40px auto;padding:0 20px;color:#e0e0e0;background:#0f0f1a}</style>
</head><body>
<p>Redirecting to sign in...</p>
<script>
(async()=>{
  try {
    const res = await fetch("${neonAuth}/sign-in/social", {
      method: "POST",
      headers: {"Content-Type":"application/json"},
      body: JSON.stringify({provider:"${provider}",callbackURL:"${callbackURL}"})
    });
    const data = await res.json();
    if(data.url) { window.location.href = data.url; }
    else { document.body.innerHTML = "<pre>Error: " + JSON.stringify(data) + "</pre>"; }
  } catch(e) {
    document.body.innerHTML = "<pre>Error: " + e.message + "</pre>";
  }
})();
</script>
</body></html>`;

	return new Response(html, {
		status: 200,
		headers: { "Content-Type": "text/html" },
	});
}
