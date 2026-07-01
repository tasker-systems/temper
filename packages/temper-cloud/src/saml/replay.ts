import type { NeonClient } from "../db.js";

/** Thrown when a SAML assertion ID has already been consumed. */
export class ReplayError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ReplayError";
  }
}

/**
 * Enforces single-use of a SAML assertion ID via `kb_saml_replay`'s primary key. The insert is
 * the atomic guard: a first use inserts and returns one row; a replay conflicts on the primary
 * key, `DO NOTHING` short-circuits, and zero rows come back -- that's the replay signal.
 */
export async function guardReplay(
  db: NeonClient,
  assertionId: string,
  expiresAt: Date,
): Promise<void> {
  const rows = await db`INSERT INTO kb_saml_replay (assertion_id, expires_at)
    VALUES (${assertionId}, ${expiresAt.toISOString()})
    ON CONFLICT (assertion_id) DO NOTHING
    RETURNING assertion_id`;
  if (rows.length === 0) {
    throw new ReplayError(`SAML assertion replayed: ${assertionId}`);
  }
}
