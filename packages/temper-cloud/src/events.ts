import { uuidv7 } from "uuidv7";
import type { NeonClient } from "./db.js";

export const DEVICE_ID_CLOUD = "vercel-cloud";

/**
 * Insert a kb_events row and a kb_resource_audits row atomically
 * via the insert_event_and_audit() SQL function.
 */
export async function insertEventAndAudit(
  db: NeonClient,
  params: {
    profileId: string;
    deviceId: string;
    contextId: string;
    resourceId: string;
    eventType: string;
    action: string;
    bodyHash: string;
    managedHash: string;
    openHash: string;
  },
): Promise<{ eventId: string; auditId: string }> {
  const eventId = uuidv7();

  const rows = await db`
    SELECT event_id, audit_id
    FROM insert_event_and_audit(
      ${eventId}::uuid,
      ${params.profileId}::uuid,
      ${params.deviceId},
      ${params.contextId}::uuid,
      ${params.resourceId}::uuid,
      ${params.eventType},
      ${params.action},
      ${params.bodyHash},
      ${params.managedHash},
      ${params.openHash}
    )
  `;

  return {
    eventId: rows[0].event_id as string,
    auditId: rows[0].audit_id as string,
  };
}
