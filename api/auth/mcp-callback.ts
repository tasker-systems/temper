/**
 * MCP OAuth callback relay (Vercel entry point). Receives Auth0's redirect after
 * login, extracts the original loopback redirect_uri + state from the signed
 * state token, and redirects the browser to the MCP client's local callback.
 */

export async function GET(req: Request): Promise<Response> {
  const { handleMcpCallback } = await import(
    "../../packages/temper-cloud/src/oauth/auth0-proxy.js"
  );
  return handleMcpCallback(req);
}
