export const config = { runtime: "nodejs" };

export default async function handler(req: Request): Promise<Response> {
  if (req.method !== "POST") {
    return new Response(JSON.stringify({ error: "Method not allowed" }), {
      status: 405,
      headers: { "Content-Type": "application/json" },
    });
  }

  const { authenticateRequest } = await import(
    "../packages/temper-cloud/src/middleware.js"
  );
  const { validatePayload } = await import(
    "../packages/temper-cloud/src/content-ingest.js"
  );
  const { processContentIngest } = await import(
    "./workflows/process-content-ingest.js"
  );

  // Authenticate
  const auth = await authenticateRequest(req);
  if (!auth.ok) return auth.response;

  // Parse and validate body
  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return new Response(
      JSON.stringify({ error: "Invalid JSON body" }),
      { status: 400, headers: { "Content-Type": "application/json" } },
    );
  }

  const validation = validatePayload(body);
  if (!validation.ok) {
    return new Response(
      JSON.stringify({ error: validation.error }),
      { status: 400, headers: { "Content-Type": "application/json" } },
    );
  }

  const { resource_id, content, replace } = validation.payload;

  // Verify the caller can access this resource
  const visibleResources = await auth.db`
    SELECT resource_id FROM resources_visible_to(${auth.profileId}::uuid)
    WHERE resource_id = ${resource_id}::uuid
  `;
  if (visibleResources.length === 0) {
    return new Response(
      JSON.stringify({ error: "Resource not found or not accessible" }),
      { status: 404, headers: { "Content-Type": "application/json" } },
    );
  }

  // Trigger the processing workflow. If the workflow fails to start,
  // we still return 202 since the request was accepted.
  try {
    await processContentIngest(resource_id, content, replace, auth.profileId);
  } catch (err) {
    console.error("Failed to trigger content processing workflow:", err);
  }

  return new Response(
    JSON.stringify({ resource_id, status: "processing" }),
    { status: 202, headers: { "Content-Type": "application/json" } },
  );
}
