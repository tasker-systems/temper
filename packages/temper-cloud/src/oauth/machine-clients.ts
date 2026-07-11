import { timingSafeEqual } from "node:crypto";
import type { NeonClient } from "../db.js";
import { hashToken } from "./mint.js";

/** Constant-time compare of two lowercase-hex strings of equal expected length. */
function hexEqual(a: string, b: string): boolean {
  const ba = Buffer.from(a, "hex");
  const bb = Buffer.from(b, "hex");
  return ba.length === bb.length && timingSafeEqual(ba, bb);
}

interface MachineSecretRow {
  secret_hash: string | null;
  secret_hash_previous: string | null;
  secret_previous_expires_at: string | Date | null;
}

/**
 * Verify a temper-issued client secret. True iff it matches the current secret, or the previous
 * secret while still inside its grace window. Only `issuer='temper'`, non-revoked rows are
 * considered — an `auth0-m2m` row (secret_hash NULL) never matches here; it verifies via JWKS.
 */
export async function verifyMachineSecret(
  db: NeonClient,
  clientId: string,
  clientSecret: string,
): Promise<boolean> {
  const rows = await db`
    SELECT secret_hash, secret_hash_previous, secret_previous_expires_at
    FROM kb_machine_clients
    WHERE client_id = ${clientId} AND issuer = 'temper' AND revoked_at IS NULL
  `;
  const row = rows[0] as MachineSecretRow | undefined;
  if (!row?.secret_hash) {
    return false;
  }

  const provided = hashToken(clientSecret);
  if (hexEqual(provided, row.secret_hash)) {
    return true;
  }

  if (
    row.secret_hash_previous &&
    row.secret_previous_expires_at &&
    new Date(row.secret_previous_expires_at) > new Date()
  ) {
    return hexEqual(provided, row.secret_hash_previous);
  }
  return false;
}

/**
 * Coarse liveness touch (mirrors the Rust gate's five-minute rule): writes only when
 * `last_seen_at` is NULL or older than five minutes, so token minting stays read-mostly.
 */
export async function touchMachineLastSeen(db: NeonClient, clientId: string): Promise<void> {
  await db`
    UPDATE kb_machine_clients
    SET last_seen_at = now()
    WHERE client_id = ${clientId}
      AND (last_seen_at IS NULL OR last_seen_at < now() - interval '5 minutes')
  `;
}
