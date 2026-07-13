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
  close(): Promise<void>;
}

export interface MockApiOptions {
  /** Reject this many leading requests with 401 before serving 200. Default 0. */
  rejectFirst?: number;
}

export async function startMockApi(opts: MockApiOptions = {}): Promise<MockApi> {
  const bearers: string[] = [];
  const rejectFirst = opts.rejectFirst ?? 0;
  let seen = 0;

  const server: Server = createServer((req, res) => {
    const header = req.headers.authorization ?? "";
    bearers.push(header.replace(/^Bearer /, ""));
    seen += 1;

    if (seen <= rejectFirst) {
      res.writeHead(401, { "content-type": "application/json" });
      res.end(JSON.stringify({ error: "unauthorized" }));
      return;
    }

    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({ ok: true }));
  });

  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;

  return {
    url: `http://127.0.0.1:${port}/api/steward/dispatch`,
    bearers,
    close: () =>
      new Promise<void>((resolve, reject) => server.close((err) => (err ? reject(err) : resolve()))),
  };
}
