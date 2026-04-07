import { z } from "zod";

export const ContentIngestSchema = z.object({
  resource_id: z.string().uuid(),
  content: z.string().min(1),
  replace: z.boolean(),
});

export type ContentIngestPayload = z.infer<typeof ContentIngestSchema>;

/**
 * Validate the content-ingest request body.
 * Returns the parsed payload or null with an error message.
 */
export function validatePayload(
  body: unknown,
): { ok: true; payload: ContentIngestPayload } | { ok: false; error: string } {
  const result = ContentIngestSchema.safeParse(body);
  if (!result.success) {
    return {
      ok: false,
      error: result.error.issues.map((i) => i.message).join(", "),
    };
  }
  return { ok: true, payload: result.data };
}
