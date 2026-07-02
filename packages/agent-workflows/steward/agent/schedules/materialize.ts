import { getToken } from "@vercel/connect";
import { defineSchedule } from "eve/schedules";

/**
 * Region materialization on its OWN threshold cadence (T6 acceptance #3).
 *
 * This is DELIBERATELY a code `run` handler doing a direct POST to the T4b
 * materialize route — NOT a model-driven `markdown` prompt. Region formation is
 * deterministic substrate work, not one of the steward's authored acts, so the
 * `cogmap_materialize*` tools stay OUT of the steward's MCP allow-list
 * (connections/temper.ts) and this trigger never routes through the model.
 *
 * The server self-gates: `POST /materialize` runs the clustering only when the
 * formation-event delta clears the materialize threshold, and is an idempotent
 * no-op (`MaterializeAck { materialized: false }`) below it. So a fixed cadence
 * is safe — the threshold, not the cron, decides when work actually happens.
 *
 * Auth mirrors connections/temper.ts EXACTLY: Vercel Connect (app subject) when
 * `TEMPER_CONNECT_CONNECTOR` is set (T6's registered connector — the same one
 * the MCP connection uses), else the already-OAuth-obtained `TEMPER_TOKEN` that
 * drives `eve dev`. No provider secret ever lives in code.
 *
 * Targets are env-driven, never hardcoded:
 * - `TEMPER_API_URL` — the temper REST base (e.g. https://temperkb.io). Distinct
 *   from `TEMPER_MCP_URL`, which is the MCP endpoint the connection speaks to.
 * - `TEMPER_SELF_COGMAP_ID` — the team self-cognition map minted at genesis
 *   (`temper cogmap create`). Set once the map exists on the target instance.
 */
export default defineSchedule({
  cron: "0 * * * *", // hourly, UTC; the server gates on the materialize threshold
  async run({ waitUntil }) {
    // waitUntil keeps the cron task alive until the POST settles (per eve docs).
    waitUntil(materializeTick());
  },
});

async function materializeTick(): Promise<void> {
  const apiUrl = requireEnv("TEMPER_API_URL").replace(/\/+$/, "");
  const cogmapId = requireEnv("TEMPER_SELF_COGMAP_ID");
  const token = await temperToken();

  const res = await fetch(`${apiUrl}/api/cognitive-maps/${cogmapId}/materialize`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/json",
    },
    // Empty body → the server applies its DEFAULT_MATERIALIZE_THRESHOLD.
    body: "{}",
  });

  if (!res.ok) {
    throw new Error(`materialize POST failed: ${res.status} ${await res.text()}`);
  }
}

async function temperToken(): Promise<string> {
  const connector = process.env.TEMPER_CONNECT_CONNECTOR;
  if (connector) {
    return getToken(connector, { subject: { type: "app" } });
  }
  return requireEnv("TEMPER_TOKEN");
}

function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is required — the temper-mcp target/credential is never hardcoded`);
  }
  return value;
}
