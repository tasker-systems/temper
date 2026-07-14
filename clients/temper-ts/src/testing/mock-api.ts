import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";

/**
 * A stand-in resource server that rejects its first N requests with 401.
 *
 * This is what proves the on-401 re-mint: the failure a client must recover from is a 401 from the
 * RESOURCE server (a token that expired mid-flight), not from the token endpoint. `bearers` records
 * every Authorization header it saw, so a test can assert the retry carried a DIFFERENT, freshly
 * minted token rather than blindly replaying the dead one.
 */
export interface MockApi {
  url: string;
  /** Every bearer token presented, in order. */
  bearers: string[];
  /** Every `X-Temper-Surface` presented, in order — the attribution header. */
  surfaces: string[];
  /**
   * Every request's body, in order — the empty string for a request that carried none, so this
   * array is INDEX-ALIGNED with `bearers` and `surfaces`. Proves a retry REPLAYED the body it was
   * given. (Pushing only non-empty bodies would slide the indices apart the moment a test mixed a
   * GET in with a POST, and a body assertion would then be reading someone else's request.)
   */
  bodies: string[];
  close(): Promise<void>;
}

export interface MockApiOptions {
  /** Reject this many leading requests with 401 before serving 200. Default 0. */
  rejectFirst?: number;
}

export async function startMockApi(opts: MockApiOptions = {}): Promise<MockApi> {
  const bearers: string[] = [];
  const surfaces: string[] = [];
  const bodies: string[] = [];
  const rejectFirst = opts.rejectFirst ?? 0;
  let seen = 0;

  const server: Server = createServer((req, res) => {
    const header = req.headers.authorization ?? "";
    bearers.push(header.replace(/^Bearer /, ""));
    surfaces.push((req.headers["x-temper-surface"] as string | undefined) ?? "");
    // This request's ordinal, fixed at HEADER time. Reading the shared `seen` from inside the
    // deferred `end` closure would make the 401 decision depend on how long a body took to
    // arrive, not on arrival ORDER: a slow-bodied first POST would see `seen` already advanced
    // past `rejectFirst` by a fast second request and be served 200 — `rejectFirst: 1` yielding
    // zero 401s, and the re-mint tests silently asserting nothing.
    const mine = ++seen;

    const chunks: Buffer[] = [];
    req.on("data", (chunk: Buffer) => chunks.push(chunk));
    req.on("end", () => {
      bodies.push(Buffer.concat(chunks).toString("utf8"));

      if (mine <= rejectFirst) {
        res.writeHead(401, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: "unauthorized" }));
        return;
      }

      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ ok: true }));
    });
  });

  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;

  return {
    url: `http://127.0.0.1:${port}/api/steward/dispatch`,
    bearers,
    surfaces,
    bodies,
    close: () =>
      new Promise<void>((resolve, reject) => server.close((err) => (err ? reject(err) : resolve()))),
  };
}
