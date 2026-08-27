import { exportPKCS8, generateKeyPair } from "jose";
import type postgres from "postgres";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { NeonClient } from "../../../src/db.js";
import {
  endRefreshChain,
  revokeRefreshToken,
  rotateRefreshToken,
  storeRefreshToken,
} from "../../../src/oauth/flow.js";
import { hashToken } from "../../../src/oauth/mint.js";
import { makeTestDb, truncateOauthTables } from "../helpers/oauth-db.js";
import {
  login,
  refresh,
  type TokenErrorBody,
  type TokenSuccessBody,
} from "../helpers/oauth-flows.js";

/**
 * A rotated refresh token that comes back.
 *
 * RFC 6819 §5.2.2.3 reads that as the indication that a chain has been copied, and acting on it
 * rests on a distinction `revoked_at` cannot draw alone: rotation is one of FIVE writers of that
 * column. Three are administrative revokers, the fifth is this feature's own chain-ending, and all
 * four of the others leave `rotated_at` NULL. Each test here holds one part of the distinction —
 * that the system records which of the five it saw, that an ordinary de-provisioning is not filed
 * as a theft, that an operator can read the record without log retention, that the response reaches
 * the chain and only that chain and cannot be undone by a rotation already in flight, and that the
 * boundary between a thief and a client that lost a response is a stated number rather than an
 * implication.
 */

describe("replayed refresh token", () => {
  let sql: postgres.Sql;
  let db: NeonClient;
  const handles: string[] = [];

  async function principal(state: string): Promise<string> {
    const handle = `replay-test-${state}-${Date.now()}-${handles.length}`;
    handles.push(handle);
    const rows = await sql`
      INSERT INTO kb_profiles (handle, display_name, email, preferences)
      VALUES (${handle}, ${handle}, ${`${handle}@example.test`}, '{}')
      RETURNING id`;
    const id = (rows[0] as { id: string }).id;
    await sql`INSERT INTO kb_principal_standing (profile_id, state) VALUES (${id}, ${state})`;
    return id;
  }

  async function tokenRow(refreshToken: string) {
    const rows = await sql`
      SELECT id, chain_id, revoked_at, rotated_at
        FROM kb_oauth_refresh_tokens WHERE token_hash = ${hashToken(refreshToken)}`;
    return rows[0] as {
      id: string;
      chain_id: string | null;
      revoked_at: Date | null;
      rotated_at: Date | null;
    };
  }

  /** How many tokens of this chain are still live. Single-use rotation means a healthy chain has 1. */
  async function liveInChain(chainId: string | null): Promise<number> {
    const rows = await sql`
      SELECT count(*)::int AS n FROM kb_oauth_refresh_tokens
       WHERE chain_id = ${chainId} AND revoked_at IS NULL`;
    return (rows[0] as { n: number }).n;
  }

  /**
   * Reads the operator's view — the surface criterion 2 is about, not the table beneath it.
   *
   * The counters are BIGINT (an attacker holding one spent token drives `replay_count` at will, and
   * an INTEGER that wrapped would make the upsert throw and the record silently stop advancing), and
   * a 64-bit integer does not fit a JS number safely, so both drivers hand it back as a STRING.
   * Coerced here, at the read, rather than by narrowing the column to something that can overflow.
   */
  async function replayView() {
    const rows = (await sql`
      SELECT *, EXTRACT(EPOCH FROM first_replay_age)::float8 AS age_seconds
        FROM vw_oauth_refresh_replays
       ORDER BY first_seen`) as unknown as Array<Record<string, unknown>>;
    return rows.map((r) => ({
      ...r,
      replay_count: Number(r.replay_count),
      graced_count: Number(r.graced_count),
      hostile_count: Number(r.hostile_count),
      tokens_revoked: Number(r.tokens_revoked),
      age_seconds: Number(r.age_seconds),
    })) as unknown as Array<{
      token_id: string;
      chain_id: string | null;
      profile_handle: string | null;
      client_id: string;
      first_seen: string;
      last_seen: string;
      replay_count: number;
      graced_count: number;
      hostile_count: number;
      tokens_revoked: number;
      age_seconds: number;
    }>;
  }

  /**
   * Moves a rotation into the past. The grace window is judged on the DATABASE clock against
   * `rotated_at`, so this is the only fact a test needs to move to reach the far side of it —
   * and moving that one fact, rather than the window, is what makes the assertion be about the
   * boundary rather than about the parser.
   */
  async function backdateRotation(refreshToken: string, interval: string): Promise<void> {
    await sql`
      UPDATE kb_oauth_refresh_tokens
         SET rotated_at = rotated_at - ${interval}::interval
       WHERE token_hash = ${hashToken(refreshToken)}`;
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
    for (const handle of handles) {
      await sql`DELETE FROM kb_profiles WHERE handle = ${handle}`;
    }
    await sql.end();
  });

  beforeEach(async () => {
    await truncateOauthTables(sql);
    process.env.AS_REFRESH_CHAIN_MAX_SECONDS = "7776000";
    delete process.env.AS_REFRESH_REPLAY_GRACE_SECONDS;
  });

  it("tells a token that was ROTATED apart from one that was merely revoked or unknown", async () => {
    const owner = await principal("approved");
    const first = await login(db, { relay: "rs-tell", code: "c-tell", profileId: owner });
    const second = (await (await refresh(db, first.refresh_token)).json()) as TokenSuccessBody;

    // The rotated predecessor, presented again.
    const replayed = await refresh(db, first.refresh_token);
    expect(replayed.status).toBe(400);
    expect((await replayed.json()) as TokenErrorBody).toEqual({ error: "invalid_grant" });

    // A token an administrator revoked. `revoked_at` has FIVE writers and only one of them is
    // rotation, so each of the other four has to be witnessed as leaving no rotation mark — a test
    // that exercises one of them says nothing about the others, and a stray `rotated_at` in any of
    // them turns an ordinary revoke into a permanent false theft report. (The fifth, this feature's
    // own `endRefreshChain`, is witnessed by the chain-ending tests below, which go on to replay
    // the chain's tip and find no second record.)
    //
    // Through the TypeScript revoker, called for real:
    const other = await login(db, { relay: "rs-adm", code: "c-adm", profileId: owner });
    await revokeRefreshToken(db, other.refresh_token as string);
    expect((await refresh(db, other.refresh_token)).status).toBe(400);

    // …and written the way the two Rust revokers write it. They match on different keys — the
    // standing hook on `profile_id`, the Slack one on `token_hash` — but both write exactly this
    // SET clause, `revoked_at` and nothing else, so this stands in for both at row level; each is
    // held to it directly by a test in its own crate.
    const third = await login(db, { relay: "rs-adm2", code: "c-adm2", profileId: owner });
    await sql`
      UPDATE kb_oauth_refresh_tokens SET revoked_at = now()
       WHERE token_hash = ${hashToken(third.refresh_token)}`;
    expect((await refresh(db, third.refresh_token)).status).toBe(400);

    // And a token this instance never issued.
    expect((await refresh(db, "not-a-token-we-ever-minted")).status).toBe(400);

    // The distinction criterion 1 is actually about lives on the ROW, before any replay: a token
    // retired by rotation says so, and one an administrator revoked cannot be made to.
    const rotatedRow = await tokenRow(first.refresh_token);
    expect(rotatedRow.revoked_at).not.toBeNull();
    expect(rotatedRow.rotated_at, "rotation records itself").not.toBeNull();
    for (const revoked of [other, third]) {
      const row = await tokenRow(revoked.refresh_token as string);
      expect(row.revoked_at, "dead by the same column").not.toBeNull();
      expect(
        row.rotated_at,
        "an administrator's revoke cannot masquerade as a rotation",
      ).toBeNull();
    }

    // Four identical refusals to the client; one event in the records.
    const recorded = await replayView();
    expect(recorded).toHaveLength(1);
    expect(recorded[0].token_id).toBe((await tokenRow(first.refresh_token)).id);
    expect(recorded[0].profile_handle, "the record names who held the chain").toBe(
      handles[handles.length - 1],
    );
    expect(recorded[0].client_id).toBe("cli");

    // The successor is untouched by any of it.
    expect((await refresh(db, second.refresh_token)).status).toBe(200);
  });

  it("inside the grace window, refuses the retry and leaves the chain alive", async () => {
    const first = await login(db, { relay: "rs-grace", code: "c-grace", profileId: null });
    const second = (await (await refresh(db, first.refresh_token)).json()) as TokenSuccessBody;
    const chainId = (await tokenRow(second.refresh_token)).chain_id;

    // Presented immediately — the shape of a client that lost the response, or raced two refreshes.
    expect((await refresh(db, first.refresh_token)).status).toBe(400);

    expect(await liveInChain(chainId), "a retry must not cost the client its session").toBe(1);
    expect((await refresh(db, second.refresh_token)).status, "and the successor still works").toBe(
      200,
    );

    const [row] = await replayView();
    expect(row.replay_count).toBe(1);
    expect(row.graced_count, "recorded as a retry, not as a theft").toBe(1);
    expect(row.hostile_count).toBe(0);
    expect(row.tokens_revoked).toBe(0);
  });

  it("graces a retry seconds after the rotation, not merely one microseconds after", async () => {
    // Every other graced presentation in this file is immediate — a few milliseconds — which is
    // inside the window under seconds OR milliseconds. A comparison that had slipped units by 1000×
    // would pass all of them while ending the chain of every real client retry. Five seconds is
    // inside a ten-second window and outside a ten-millisecond one.
    const first = await login(db, { relay: "rs-units", code: "c-units", profileId: null });
    const second = (await (await refresh(db, first.refresh_token)).json()) as TokenSuccessBody;
    const chainId = (await tokenRow(second.refresh_token as string)).chain_id;
    await backdateRotation(first.refresh_token, "5 seconds");

    expect((await refresh(db, first.refresh_token)).status).toBe(400);

    const [row] = await replayView();
    expect(row.graced_count, "5s < 10s: still a client retry").toBe(1);
    expect(row.hostile_count).toBe(0);
    expect(row.age_seconds).toBeGreaterThan(1);
    expect(await liveInChain(chainId), "and the chain is untouched").toBe(1);
    expect((await refresh(db, second.refresh_token)).status).toBe(200);
  });

  it("outside the grace window, ends the chain", async () => {
    const first = await login(db, { relay: "rs-late", code: "c-late", profileId: null });
    const second = (await (await refresh(db, first.refresh_token)).json()) as TokenSuccessBody;
    const chainId = (await tokenRow(second.refresh_token)).chain_id;
    await backdateRotation(first.refresh_token, "10 minutes");

    expect((await refresh(db, first.refresh_token)).status).toBe(400);

    expect(await liveInChain(chainId), "the copied chain is ended, not merely refused").toBe(0);
    expect(
      (await refresh(db, second.refresh_token)).status,
      "including the successor the thief did not present",
    ).toBe(400);

    const [row] = await replayView();
    expect(row.graced_count).toBe(0);
    expect(row.hostile_count).toBe(1);
    expect(row.tokens_revoked, "says how many live tokens it actually took").toBe(1);
    expect(row.age_seconds).toBeGreaterThan(60);
  });

  it("reaches the CURRENT tip from a replay of a MIDDLE token, two rotations back", async () => {
    // The property chain identity exists for. A thief holding an early copy of the chain presents
    // a token whose own successor is long dead; the response has to reach whatever is live NOW.
    //
    // **The replayed token is deliberately NOT the chain root.** A root satisfies `chain_id = id`,
    // so an implementation that reached by the presented row's own id instead of by its chain would
    // be indistinguishable here — the assertion below pins `id !== chain_id` precisely so this test
    // can tell the two apart.
    const t1 = await login(db, { relay: "rs-deep", code: "c-deep", profileId: null });
    const t2 = (await (await refresh(db, t1.refresh_token)).json()) as TokenSuccessBody;
    const t3 = (await (await refresh(db, t2.refresh_token)).json()) as TokenSuccessBody;
    const t4 = (await (await refresh(db, t3.refresh_token)).json()) as TokenSuccessBody;

    const rows = await Promise.all(
      [t1, t2, t3, t4].map((t) => tokenRow(t.refresh_token as string)),
    );
    const chainId = rows[0].chain_id;
    expect(chainId, "a new chain is rooted at its own first token").toBe(rows[0].id);
    for (const row of rows) {
      expect(row.chain_id, "inherited UNCHANGED, which is what makes the reach possible").toBe(
        chainId,
      );
    }

    expect(rows[1].id, "the replayed token must not be its own chain's name").not.toBe(chainId);

    await backdateRotation(t2.refresh_token, "1 hour");
    expect((await refresh(db, t2.refresh_token)).status).toBe(400);

    expect(await liveInChain(chainId)).toBe(0);
    expect((await refresh(db, t4.refresh_token)).status, "the live tip is ended").toBe(400);
    const [row] = await replayView();
    expect(row.token_id).toBe(rows[1].id);
    expect(row.tokens_revoked, "exactly the one live token, not the whole chain's history").toBe(1);
  });

  it("ends only the replayed chain, not the principal's other sessions", async () => {
    // A replay is evidence about ONE copied chain. Ending every chain the principal owns is what
    // an administrator's revoke does, and doing it here would sign a user out of their other
    // devices for someone else's theft.
    const owner = await principal("approved");
    const laptop = await login(db, { relay: "rs-l", code: "c-l", profileId: owner });
    const phone = await login(db, { relay: "rs-p", code: "c-p", profileId: owner });

    const laptop2 = (await (await refresh(db, laptop.refresh_token)).json()) as TokenSuccessBody;
    await backdateRotation(laptop.refresh_token, "10 minutes");
    expect((await refresh(db, laptop.refresh_token)).status).toBe(400);

    expect((await refresh(db, laptop2.refresh_token)).status, "the copied chain is ended").toBe(
      400,
    );
    expect((await refresh(db, phone.refresh_token)).status, "the untouched chain is not").toBe(200);
  });

  it("upserts one row per token however many times it is presented", async () => {
    // The write is reachable by anyone holding a retired token, so an append-shaped record would
    // let a loop grow the table without bound.
    const first = await login(db, { relay: "rs-loop", code: "c-loop", profileId: null });
    await refresh(db, first.refresh_token);
    await backdateRotation(first.refresh_token, "10 minutes");

    for (let i = 0; i < 4; i++) {
      expect((await refresh(db, first.refresh_token)).status).toBe(400);
    }

    const rows = await replayView();
    expect(rows).toHaveLength(1);
    expect(rows[0].replay_count).toBe(4);
    // The chain was ended by the first of them; the other three had nothing left to take, and the
    // count says so rather than claiming four sessions were ended.
    expect(rows[0].tokens_revoked).toBe(1);
    expect(rows[0].hostile_count).toBe(4);
  });

  it("cannot be stepped over by a rotation already past its own guard", async () => {
    // Rotation is TWO statements with no transaction across them: the guard revokes the predecessor,
    // and the successor is inserted afterwards. A chain-ending that lands in that gap finds every
    // row of the chain momentarily dead, so revoking rows takes nothing — and without a record of
    // the ENDING itself, the successor would then arrive and the chain would be alive again behind
    // a responder that had just reported ending it.
    //
    // Played out deterministically through the same functions the endpoint calls, in the order a
    // real interleaving would produce them. This is also exactly what a client racing two refreshes
    // produces, which is the case AS_REFRESH_REPLAY_GRACE_SECONDS=0 is aimed at.
    const first = await login(db, { relay: "rs-race", code: "c-race", profileId: null });
    const second = (await (await refresh(db, first.refresh_token)).json()) as TokenSuccessBody;

    // A rotation gets past its guard: the tip is now spent, and nothing of the chain is live.
    const rotated = await rotateRefreshToken(db, second.refresh_token as string);
    expect(await liveInChain(rotated.chainId), "the gap the ending has to survive").toBe(0);

    // The ending lands inside the gap and honestly reports taking nothing…
    expect(await endRefreshChain(db, rotated.chainId)).toBe(0);

    // …and the successor is refused anyway, because the ending was recorded and not merely applied.
    const minted = await storeRefreshToken(db, {
      token: "successor-that-must-not-land",
      clientId: rotated.clientId,
      claims: rotated.claims,
      expiresAt: new Date(Date.now() + 3600_000),
      chainExpiresAt: rotated.chainExpiresAt,
      chainId: rotated.chainId,
      profileId: rotated.profileId,
    });
    expect(minted, "an ended chain does not come back to life behind the responder").toBeNull();
    expect(await liveInChain(rotated.chainId)).toBe(0);
  });

  it("honours a zero grace window as the BCP's strictest reading", async () => {
    process.env.AS_REFRESH_REPLAY_GRACE_SECONDS = "0";
    const first = await login(db, { relay: "rs-zero", code: "c-zero", profileId: null });
    const second = (await (await refresh(db, first.refresh_token)).json()) as TokenSuccessBody;

    // No backdating: the replay is immediate, and with no window there is no benign reading of it.
    expect((await refresh(db, first.refresh_token)).status).toBe(400);
    expect((await refresh(db, second.refresh_token)).status).toBe(400);

    const [row] = await replayView();
    expect(row.graced_count).toBe(0);
    expect(row.tokens_revoked).toBe(1);
  });

  it("keeps single-use intact — a graced replay is refused every time, never honoured", async () => {
    // The mechanism added here only ever REFUSES harder. If a graced replay were ever answered
    // with a token pair, the grace window would have become a hole in single-use rather than a
    // mercy in front of it.
    const first = await login(db, { relay: "rs-once", code: "c-once", profileId: null });
    const second = (await (await refresh(db, first.refresh_token)).json()) as TokenSuccessBody;
    const chainId = (await tokenRow(second.refresh_token)).chain_id;

    for (let i = 0; i < 3; i++) {
      const res = await refresh(db, first.refresh_token);
      expect(res.status).toBe(400);
      expect((await res.json()) as TokenErrorBody).toEqual({ error: "invalid_grant" });
    }

    expect(await liveInChain(chainId), "still exactly one live token, never two").toBe(1);
    expect((await tokenRow(first.refresh_token)).rotated_at).not.toBeNull();

    // The upsert's DO UPDATE arm, which the single-presentation tests above never reach. All three
    // presentations were inside the window, so `graced_count` has to have followed `replay_count`;
    // a record that only ever incremented the total would report two of a client's own retries as
    // hostile. And `first_seen` must NOT move, because the age an operator reads is the age of the
    // presentation the grace judgement was made against — a replay loop must not be able to keep
    // resetting it to look like a fresh retry.
    const [row] = await replayView();
    expect(row.replay_count).toBe(3);
    expect(row.graced_count, "a retrying client is not three-quarters a thief").toBe(3);
    expect(row.hostile_count).toBe(0);
    expect(new Date(row.first_seen).getTime(), "first_seen is the FIRST presentation").toBeLessThan(
      new Date(row.last_seen).getTime(),
    );
  });

  it("does not report a de-provisioned user's own retry as a theft", async () => {
    // The admission gate sits AFTER the rotation guard: a terminal principal's refresh really does
    // rotate, and only then is refused a successor. Left marked as rotated, that spent token turns
    // the user's next retry into a replay — putting a de-provisioned person in the operator's view
    // under a hostile count, which is the same false theft report the administrative revokers are
    // held away from, reached from the other side.
    const off = await principal("approved");
    const issued = await login(db, { relay: "rs-offb", code: "c-offb", profileId: off });
    await sql`UPDATE kb_principal_standing SET state = 'deactivated' WHERE profile_id = ${off}`;

    const refused = await refresh(db, issued.refresh_token);
    expect(refused.status, "no renewable session for a terminal principal").toBe(400);
    expect(
      (await tokenRow(issued.refresh_token)).rotated_at,
      "a rotation that minted no successor is not a rotation",
    ).toBeNull();

    // …so the client's retry is an ordinary stale token, not evidence of anything — and it stays
    // that way however long the client waits, because there is no mark to age.
    expect((await refresh(db, issued.refresh_token)).status).toBe(400);
    expect(await replayView(), "nobody is accused").toHaveLength(0);
  });

  it("keeps an unusable grace setting from silently disabling the detector", async () => {
    // The sibling parser (`refreshChainMaxSeconds`) refuses an unusable value and is witnessed
    // doing so; this one substitutes and warns, and the reason is that its caller swallows
    // everything — a throw here would surface as nothing at all and leave the detector inert. Both
    // an unparseable value and one past the one-hour ceiling must therefore still detect.
    for (const configured of ["10s", "36000"]) {
      await truncateOauthTables(sql);
      process.env.AS_REFRESH_REPLAY_GRACE_SECONDS = configured;
      const first = await login(db, {
        relay: `rs-cfg-${configured}`,
        code: `c-cfg-${configured}`,
        profileId: null,
      });
      await refresh(db, first.refresh_token);
      await backdateRotation(first.refresh_token, "2 hours");

      expect((await refresh(db, first.refresh_token)).status).toBe(400);
      const [row] = await replayView();
      expect(row, `AS_REFRESH_REPLAY_GRACE_SECONDS=${configured} must still detect`).toBeDefined();
      expect(
        row.hostile_count,
        "and fall back to the default window, not to an unbounded one",
      ).toBe(1);
    }
  });

  /**
   * A replay record is attributed to whoever the TOKEN was attributed to — never to nobody when
   * the token had an owner, and never to somebody when it did not.
   *
   * This looks like a detail of one INSERT and is load-bearing somewhere else entirely. Deleting a
   * profile cascades into `kb_oauth_refresh_tokens`, and since the AS retention sweep landed,
   * `kb_oauth_refresh_replays.token_id` is `ON DELETE RESTRICT` — so evidence cannot be swept away,
   * and equally a token cannot be deleted while evidence points at it. Those two facts only
   * coexist because this one holds: an OWNED token's evidence is owned too and cascades beside it,
   * and an UNOWNED token is not reachable from a profile delete at all. Break the agreement in
   * either direction and a profile delete meets a 23001 it has no way past.
   *
   * `recordRefreshReplay` gets `profile_id` from the token row it is recording against, so this
   * holds by construction rather than by care — which is exactly why it is asserted here and not
   * assumed. Nothing else in the schema enforces it (`profile_id` is nullable on both tables by
   * design, for the fail-open login that records no owner), so this test is the whole guard.
   */
  it("attributes a replay to exactly whoever the token was attributed to", async () => {
    const owner = await principal("approved");

    for (const [label, profileId] of [
      ["an owned chain", owner],
      ["a chain from a fail-open login, which records no owner", null],
    ] as const) {
      await truncateOauthTables(sql);
      const first = await login(db, {
        relay: `rs-attr-${profileId ?? "null"}`,
        code: `c-attr-${profileId ?? "null"}`,
        profileId,
      });
      await refresh(db, first.refresh_token);
      const rotated = await tokenRow(first.refresh_token);

      // The rotated predecessor, presented again — this is what writes the replay row.
      expect((await refresh(db, first.refresh_token)).status).toBe(400);

      const rows = await sql`
        SELECT t.profile_id AS token_owner, r.profile_id AS evidence_owner
          FROM kb_oauth_refresh_replays r
          JOIN kb_oauth_refresh_tokens t ON t.id = r.token_id
         WHERE r.token_id = ${rotated.id}`;
      const [row] = rows as unknown as Array<{
        token_owner: string | null;
        evidence_owner: string | null;
      }>;

      expect(row, `${label}: the replay must have been recorded at all`).toBeDefined();
      expect(row.token_owner, `${label}: the token carries the owner the login resolved`).toBe(
        profileId,
      );
      expect(
        row.evidence_owner,
        `${label}: the evidence must carry the SAME owner — a mismatch here is what would make a ` +
          `profile delete fail against the RESTRICT protecting this row`,
      ).toBe(row.token_owner);
    }
  });
});
