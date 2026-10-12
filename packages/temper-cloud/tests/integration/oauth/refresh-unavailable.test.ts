import { exportPKCS8, generateKeyPair } from "jose";
import type postgres from "postgres";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { NeonClient } from "../../../src/db.js";
import { revokeRefreshToken } from "../../../src/oauth/flow.js";
import { hashToken } from "../../../src/oauth/mint.js";
import { makeTestDb, truncateOauthTables } from "../helpers/oauth-db.js";
import {
  login,
  refresh,
  type TokenErrorBody,
  type TokenSuccessBody,
} from "../helpers/oauth-flows.js";

/**
 * A refresh that the token store could not answer is not a refusal.
 *
 * RFC 6749 §5.2 `invalid_grant` says the grant is dead, and a client that hears it discards the
 * token — temper-api's Slack grant vault tells its user to reconnect. So it may only be said when
 * the rotation guard actually looked and found no live row. A Neon connection error, timeout or
 * cold start during that look says nothing about the grant, and is answered
 * `temporarily_unavailable` instead, the way the issue leg already answers its own faults.
 */

describe("refresh rotation against an unavailable token store", () => {
  let sql: postgres.Sql;
  let db: NeonClient;

  /**
   * The rotation guard is the only statement that sets `rotated_at` to `now()`; `unmarkRotation`
   * clears it and the chain-ending never touches it. Matching on that text fails exactly the
   * statement under test and lets every other read and write through — including the replay
   * judge's, so a test can tell whether the judge ran.
   */
  function isRotationGuard(strings: TemplateStringsArray): boolean {
    return strings.join("$").includes("rotated_at = now()");
  }

  /**
   * A store whose rotation guard fails the way a dropped Neon connection does — before the
   * statement reaches the database, or (`afterCommit`) once it has committed and only the reply is
   * lost.
   */
  function storeFailingRotation(opts: { afterCommit?: boolean } = {}): {
    db: NeonClient;
    failures: () => number;
  } {
    let failures = 0;
    const inner = sql as unknown as (s: TemplateStringsArray, ...v: unknown[]) => Promise<unknown>;
    const dropped = () =>
      Object.assign(new Error("Error connecting to database: fetch failed"), {
        code: "ECONNRESET",
      });
    const failing = async (strings: TemplateStringsArray, ...values: unknown[]) => {
      if (!isRotationGuard(strings)) {
        return inner(strings, ...values);
      }
      failures += 1;
      if (opts.afterCommit) {
        await inner(strings, ...values);
      }
      throw dropped();
    };
    return { db: failing as unknown as NeonClient, failures: () => failures };
  }

  /** The operator's view of replays; BIGINT counters come back as strings, so coerce at the read. */
  async function replayView() {
    const rows = (await sql`
      SELECT graced_count, hostile_count, tokens_revoked, chain_ended
        FROM vw_oauth_refresh_replays`) as unknown as Array<Record<string, unknown>>;
    return rows.map((r) => ({
      graced_count: Number(r.graced_count),
      hostile_count: Number(r.hostile_count),
      tokens_revoked: Number(r.tokens_revoked),
      chain_ended: r.chain_ended,
    }));
  }

  async function tokenRow(refreshToken: string | undefined) {
    const rows = await sql`
      SELECT revoked_at, rotated_at
        FROM kb_oauth_refresh_tokens WHERE token_hash = ${hashToken(refreshToken ?? "")}`;
    return rows[0] as { revoked_at: Date | null; rotated_at: Date | null } | undefined;
  }

  async function replayRows(): Promise<number> {
    const rows = await sql`SELECT count(*)::int AS n FROM kb_oauth_refresh_replays`;
    return (rows[0] as { n: number }).n;
  }

  beforeAll(async () => {
    const { privateKey } = await generateKeyPair("Ed25519", { extractable: true });
    process.env.AS_SIGNING_KEY_PKCS8 = await exportPKCS8(privateKey);
    process.env.AS_SIGNING_KID = "test-kid-1";
    process.env.AS_ISSUER = "https://issuer.test";
    process.env.AS_AUDIENCE = "https://audience.test";
    process.env.AS_ACCESS_TTL_SECONDS = "900";
    process.env.AS_REFRESH_TTL_SECONDS = "2592000";
    process.env.AS_CLIENTS = JSON.stringify({ cli: ["http://localhost/cb"] });
    ({ sql, db } = makeTestDb());
  });

  afterAll(async () => {
    await sql.end();
  });

  beforeEach(async () => {
    await truncateOauthTables(sql);
    process.env.AS_REFRESH_CHAIN_MAX_SECONDS = "7776000";
    delete process.env.AS_REFRESH_REPLAY_GRACE_SECONDS;
  });

  it("answers 503 temporarily_unavailable, and the grant survives the blip", async () => {
    const first = await login(db, { relay: "rs-blip", code: "c-blip", profileId: null });
    const blip = storeFailingRotation();

    const res = await refresh(blip.db, first.refresh_token);
    expect(blip.failures(), "the rotation guard must actually have been failed").toBe(1);
    expect(res.status).toBe(503);
    expect(res.headers.get("retry-after")).toBe("2");
    expect((await res.json()) as TokenErrorBody).toEqual({ error: "temporarily_unavailable" });

    // Nothing was spent and nothing was recorded: the guard never ran, so there is no refusal and
    // no replay for the judge to see.
    const row = await tokenRow(first.refresh_token);
    expect(row?.revoked_at).toBeNull();
    expect(row?.rotated_at).toBeNull();
    expect(await replayRows()).toBe(0);

    // Which is what the client's retry depends on: once the store answers, the same token rotates.
    const retry = await refresh(db, first.refresh_token);
    expect(retry.status).toBe(200);
    expect(((await retry.json()) as TokenSuccessBody).refresh_token).toBeTruthy();
  });

  it("does not judge a replay it could not look up — the replay is judged when the store answers", async () => {
    // Outside the grace window, so a replay that IS judged ends the chain. The blip must neither
    // end it nor record it; the reuse detector's verdict waits for a store that can answer.
    process.env.AS_REFRESH_REPLAY_GRACE_SECONDS = "0";
    const first = await login(db, { relay: "rs-replay", code: "c-replay", profileId: null });
    const second = (await (await refresh(db, first.refresh_token)).json()) as TokenSuccessBody;
    const blip = storeFailingRotation();

    const duringBlip = await refresh(blip.db, first.refresh_token);
    expect(blip.failures()).toBe(1);
    expect(duringBlip.status).toBe(503);
    expect((await duringBlip.json()) as TokenErrorBody).toEqual({
      error: "temporarily_unavailable",
    });
    expect(await replayRows()).toBe(0);
    expect((await tokenRow(second.refresh_token))?.revoked_at, "the live tip survives").toBeNull();

    // Unchanged semantics once the store answers: the replay is refused, recorded, and the chain
    // ended.
    const afterBlip = await refresh(db, first.refresh_token);
    expect(afterBlip.status).toBe(400);
    expect((await afterBlip.json()) as TokenErrorBody).toEqual({ error: "invalid_grant" });
    expect(await replayRows()).toBe(1);
    expect((await tokenRow(second.refresh_token))?.revoked_at).not.toBeNull();
  });

  describe("when the rotation committed and only its reply was lost", () => {
    // The AS cannot tell this from a failure before the statement ran, so it answers the same 503
    // and tidies nothing. The token is spent with no successor; the retry decides the record.
    async function spentByLostReply(relay: string): Promise<string | undefined> {
      const first = await login(db, { relay, code: `c-${relay}`, profileId: null });
      const blip = storeFailingRotation({ afterCommit: true });

      const res = await refresh(blip.db, first.refresh_token);
      expect(blip.failures()).toBe(1);
      expect(res.status).toBe(503);
      expect((await res.json()) as TokenErrorBody).toEqual({ error: "temporarily_unavailable" });

      const row = await tokenRow(first.refresh_token);
      expect(row?.revoked_at, "the guard committed").not.toBeNull();
      expect(row?.rotated_at).not.toBeNull();
      expect(await replayRows(), "nothing is judged until the client retries").toBe(0);
      return first.refresh_token;
    }

    it("files a retry inside the grace window as a client retry", async () => {
      const token = await spentByLostReply("rs-lost-fast");

      const retry = await refresh(db, token);
      expect(retry.status).toBe(400);
      expect((await retry.json()) as TokenErrorBody).toEqual({ error: "invalid_grant" });
      expect(await replayView()).toEqual([
        { graced_count: 1, hostile_count: 0, tokens_revoked: 0, chain_ended: false },
      ]);
    });

    it("files a late retry as a hostile replay that ends a chain holding nothing live", async () => {
      const token = await spentByLostReply("rs-lost-late");
      await sql`
        UPDATE kb_oauth_refresh_tokens
           SET rotated_at = rotated_at - interval '1 hour'
         WHERE token_hash = ${hashToken(token ?? "")}`;

      const retry = await refresh(db, token);
      expect(retry.status).toBe(400);
      expect((await retry.json()) as TokenErrorBody).toEqual({ error: "invalid_grant" });
      expect(await replayView()).toEqual([
        { graced_count: 0, hostile_count: 1, tokens_revoked: 0, chain_ended: true },
      ]);
    });
  });

  it("still answers 400 invalid_grant for a revoked token", async () => {
    const first = await login(db, { relay: "rs-rev", code: "c-rev", profileId: null });
    await revokeRefreshToken(db, first.refresh_token as string);

    const res = await refresh(db, first.refresh_token);
    expect(res.status).toBe(400);
    expect((await res.json()) as TokenErrorBody).toEqual({ error: "invalid_grant" });
  });

  it("still answers 400 invalid_grant for an unknown token", async () => {
    const res = await refresh(db, "never-issued-refresh-token");
    expect(res.status).toBe(400);
    expect((await res.json()) as TokenErrorBody).toEqual({ error: "invalid_grant" });
  });
});
