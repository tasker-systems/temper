/**
 * MCP OAuth callback relay (Vercel entry point). Receives Auth0's redirect after
 * login, extracts the original loopback redirect_uri + state from the encrypted
 * state token, and redirects the browser to the MCP client's local callback.
 *
 * Part of the Auth0 loopback proxy, so it serves Auth0-fronted instances only.
 * Unlike `api/oauth/authorize.ts` and `api/oauth/token.ts` there is no Temper AS
 * counterpart to dispatch to when `AS_ISSUER` is set, so the mode check lives in
 * `handleMcpCallback` — where a test can witness it — and reports the endpoint
 * absent. This wrapper stays thin.
 */

export async function GET(req: Request): Promise<Response> {
  const { handleMcpCallback } = await import(
    "../../packages/temper-cloud/src/oauth/auth0-proxy.js"
  );
  return handleMcpCallback(req);
}
