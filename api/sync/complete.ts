export const config = { runtime: "nodejs" };

export default async function handler(req: Request): Promise<Response> {
  const { requireMethod, authenticateRequest } = await import(
    "../../packages/temper-cloud/src/middleware.js"
  );
  const { SyncCompleteBodySchema, completeSyncRound } = await import(
    "../../packages/temper-cloud/src/sync.js"
  );

  const methodError = requireMethod(req, "POST");
  if (methodError) return methodError;

  const auth = await authenticateRequest(req);
  if (!auth.ok) return auth.response;

  const rawBody = await req.json();
  const parsed = SyncCompleteBodySchema.safeParse(rawBody);
  if (!parsed.success) {
    return new Response(
      JSON.stringify({ error: { code: "VALIDATION", issues: parsed.error.issues } }),
      { status: 400, headers: { "Content-Type": "application/json" } },
    );
  }

  const result = await completeSyncRound(auth.db, auth.profileId, parsed.data);

  return new Response(JSON.stringify(result), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });
}
