import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";

/**
 * A faithful in-process stand-in for the two issuers a temper instance can be fronted by.
 *
 * Faithfulness is TRANSITIVE, and that is the whole design: this mock is built from
 * `tests/contracts/m2m-token-request.json`, and the REAL authorization server is asserted against
 * that same file by packages/temper-cloud's oauth integration suite. A mock that drifts from the AS
 * breaks the AS's own test first. That is what lets a client prove the temper-AS path — the one a
 * self-hosted/SAML instance depends on — without standing an AS up.
 *
 * The two flavors differ in ways that MATTER to a client, and the differences are the point:
 *
 *   auth0      — `audience` is required; a JSON body is tolerated (Auth0's extension); long TTL.
 *   temper-as  — `audience` is ignored entirely; a JSON body is `invalid_request`; 900s TTL; a
 *                rotated previous secret stays valid inside its grace window.
 *
 * The Auth0 flavor's JSON tolerance is deliberately reproduced. It is precisely why a JSON-minting
 * client stayed green for months: the only issuer it ever met forgave it.
 */

export type IssuerFlavor = "auth0" | "temper-as";

/** A recorded mint attempt, so a test can assert what the client actually put on the wire. */
export interface MintRequest {
  contentType: string;
  params: Record<string, string>;
  /** Present when the client used `client_secret_basic` instead of putting credentials in the body. */
  basic?: { clientId: string; clientSecret: string };
}

export interface MockIssuerOptions {
  flavor: IssuerFlavor;
  clientId: string;
  clientSecret: string;
  /** auth0 only: the audience the issuer demands. Ignored by the temper-as flavor, as the real AS ignores it. */
  audience?: string;
  /** Defaults: 86400 (auth0), 900 (temper-as — the AS's AS_ACCESS_TTL_SECONDS default). */
  expiresInSeconds?: number;
  /** temper-as only: a rotated-out secret, valid until `previousSecretExpiresAt`. */
  previousSecret?: string;
  /** Absolute ms since epoch. */
  previousSecretExpiresAt?: number;
}

export interface MockIssuer {
  /** The token endpoint — hand this to a client as its `tokenUrl`. */
  url: string;
  /** Every mint attempt, in order. */
  requests: MintRequest[];
  close(): Promise<void>;
}

function json(res: ServerResponse, status: number, body: unknown): void {
  const payload = JSON.stringify(body);
  res.writeHead(status, { "content-type": "application/json" });
  res.end(payload);
}

async function readBody(req: IncomingMessage): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of req) {
    chunks.push(chunk as Buffer);
  }
  return Buffer.concat(chunks).toString("utf8");
}

/** RFC 6749 §2.3.1 — `Basic base64(client_id:client_secret)`. The AS prefers this over the body when present. */
function parseBasic(header: string | undefined): { clientId: string; clientSecret: string } | undefined {
  if (header === undefined || !header.startsWith("Basic ")) {
    return undefined;
  }
  const decoded = Buffer.from(header.slice("Basic ".length), "base64").toString("utf8");
  const separator = decoded.indexOf(":");
  if (separator === -1) {
    return undefined;
  }
  return { clientId: decoded.slice(0, separator), clientSecret: decoded.slice(separator + 1) };
}

export async function startMockIssuer(opts: MockIssuerOptions): Promise<MockIssuer> {
  const requests: MintRequest[] = [];
  const isAs = opts.flavor === "temper-as";
  const expiresIn = opts.expiresInSeconds ?? (isAs ? 900 : 86_400);
  let minted = 0;

  const server: Server = createServer((req, res) => {
    void (async () => {
      const contentType = (req.headers["content-type"] ?? "").split(";")[0]?.trim() ?? "";
      const raw = await readBody(req);

      // RFC 6749 §4 mandates form encoding. Auth0 tolerates JSON; temper's AS answers
      // `invalid_request` — and must NOT throw, or a JSON-minting client cannot read its own error.
      let params: Record<string, string> = {};
      if (contentType === "application/json") {
        if (isAs) {
          json(res, 400, { error: "invalid_request", error_description: "body must be form-encoded" });
          return;
        }
        params = JSON.parse(raw) as Record<string, string>;
      } else {
        params = Object.fromEntries(new URLSearchParams(raw));
      }

      const basic = parseBasic(req.headers.authorization);
      requests.push({ contentType, params, ...(basic === undefined ? {} : { basic }) });

      const clientId = basic?.clientId ?? params.client_id;
      const clientSecret = basic?.clientSecret ?? params.client_secret;

      if (params.grant_type !== "client_credentials") {
        json(res, 400, { error: "unsupported_grant_type" });
        return;
      }

      // Auth0's audience is not part of the client_credentials protocol — it is Auth0's. The AS
      // ignores a request-supplied one entirely, which is why a temper-issued client omits it.
      if (!isAs && params.audience !== opts.audience) {
        json(res, 400, { error: "invalid_request", error_description: "audience is required" });
        return;
      }

      const secretIsCurrent = clientSecret === opts.clientSecret;
      const secretIsInGrace =
        isAs &&
        opts.previousSecret !== undefined &&
        clientSecret === opts.previousSecret &&
        (opts.previousSecretExpiresAt ?? 0) > Date.now();

      if (clientId !== opts.clientId || (!secretIsCurrent && !secretIsInGrace)) {
        json(res, 401, { error: "invalid_client" });
        return;
      }

      minted += 1;
      // No refresh token, ever (RFC 6749 §4.4.3): the credential IS the refresh mechanism.
      json(res, 200, {
        access_token: `${opts.flavor}-token-${minted}`,
        token_type: "Bearer",
        expires_in: expiresIn,
      });
    })();
  });

  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;

  return {
    url: `http://127.0.0.1:${port}/oauth/token`,
    requests,
    close: () =>
      new Promise<void>((resolve, reject) => server.close((err) => (err ? reject(err) : resolve()))),
  };
}
