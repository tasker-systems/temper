# temper-ts

TypeScript client for the Temper knowledge base API. Sibling of `temper-rb` (and the coming
`temper-py`) — all three are pinned to the same OpenAPI contract. Workspace-isolated: this package
is **not** a bun workspace member and **not** a cargo member, so it never collides with
`temper-cloud`'s TS 5.8 or the root pre-commit. Run npm commands from inside this directory
(`cd clients/temper-ts && npm install`) — a root `npm install` inherits the root's bun `overrides`
and fails.

It has three parts:

- **`credentials.ts`** — `Credentials`, `BearerToken`, `ClientCredentials`: bearer tokens and
  `client_credentials` M2M minting, with refresh-on-401 and coalesced concurrent mints.
- **`generated/schema.ts`** — the wire contract: `paths`, `components`, `operations`, emitted by
  `openapi-typescript` from the repo-root `openapi.json`.
- **`auth-fetch.ts` / `client.ts`** — `createAuthedFetch` and `createTemperClient`: the minimal
  hand-written layer the contract cannot generate (auth, attribution, one re-mint on 401).

## `schema.ts` is generated — never hand-edit it

`clients/temper-ts/src/generated/schema.ts` is a committed product of `openapi.json`, itself a
product of the Axum router. A new or renamed field on a response DTO leaves it stale until it is
regenerated.

Regenerate from the repo root:

```bash
cargo make openapi       # regenerates openapi.json + the temper-rb gem + this schema, together
# or, narrower:
cargo make openapi-ts    # regenerates only this schema
```

or from inside this package:

```bash
npm run generate
```

`cargo make check` (and CI, via the `test-agents-ts` job) runs `openapi-ts-drift`, which
regenerates the schema and diffs it against what's committed — **it never skips** (unlike the
gem's Docker-dependent drift gate, `openapi-typescript` needs only Node). If it fails right after
you correctly regenerated, the file is probably just unstaged — `git add` it and re-run.

`openapi-typescript` is pinned **exactly** (no caret) in `devDependencies` and locked in
`package-lock.json`: a moving generator would make the drift gate fail on days nothing in this
repo changed.

## Usage

```typescript
import { ClientCredentials, createTemperClient } from "temper-ts";

const credentials = new ClientCredentials({
  tokenUrl: "https://your-instance.example.com/oauth/token",
  clientId: process.env.TEMPER_CLIENT_ID!,
  clientSecret: process.env.TEMPER_CLIENT_SECRET!,
});

const client = createTemperClient({
  baseUrl: "https://your-instance.example.com",
  credentials,
});

try {
  // Every path in openapi.json is callable, typed end to end — params and per-status
  // responses come straight off the generated schema.
  const { data, error, response } = await client.GET("/api/resources", {
    params: { query: { limit: 20 } },
  });

  if (error) {
    // An HTTP STATUS error is a value, not an exception: `error` is typed from the spec's
    // own error responses, per status. A 404 or a 422 lands here.
    console.error(response.status, error);
  } else {
    console.log(data);
  }
} catch (cause) {
  // AUTH and TRANSPORT failures THROW — they are the client's own plumbing failing, and the
  // contract never described them. A bad `clientSecret` throws `TokenMintError` before a
  // request is ever sent; an unreachable host throws a network `TypeError`. `createTemperClient`
  // injects the authed fetch, and openapi-fetch rethrows whatever its fetch throws.
  //
  // An `if (error)` with no `try` around it is therefore not enough: the commonest M2M
  // misconfiguration there is — a wrong secret — would surface as an unhandled rejection.
  console.error("temper call failed before it got an answer", cause);
}
```

`createTemperClient` returns `openapi-fetch`'s `Client<paths>` directly — there are no
hand-written per-endpoint methods (no `resources.create()`). TypeScript infers everything the
schema knows; a hand-written wrapper over that would just be a second, worse spelling of the same
thing, and a place for the two to drift.

**The error model has two halves, and only one of them is a value.** HTTP status errors come back
in `error`, typed per status from the contract — no exception, no hand-written error hierarchy
(that is what the gem's `errors.rb` exists for, because Faraday raises). Auth and transport
failures **throw**: `TokenMintError` (exported from this package; carries the issuer's `status`)
when a credential cannot mint, and a network `TypeError` when the host is unreachable. Wrap the
call in a `try`.

`ClientCredentials` also accepts an optional `audience` (only meaningful against an Auth0-fronted
instance — temper's own AS ignores a request-supplied audience) and an injectable `now` for
tests. `BearerToken` is the other `Credentials` implementation, for a request already carrying a
token from a signed-in human; it cannot refresh, so a 401 comes back untouched rather than being
silently swallowed.

To attach only the auth/attribution layer to your own `fetch`-based code (without the typed
client), use `createAuthedFetch({ credentials })` directly — it returns a `(Request) => Promise<Response>`
that sets the bearer token and `X-Temper-Surface` header and re-mints once on a 401.

## The `temper-ts/schema` export, and the temper-ui exit

```typescript
import type { components, operations, paths } from "temper-ts/schema";
```

This is a public export from day one, not an internal detail. `temper-ui` types its API surface
today from types generated by `ts-rs` (`packages/temper-ui/src/lib/types/generated/`) — a
*second*, independent generator run over the same Rust structs. 103 of `temper-ui`'s 133 ts-rs
types also exist as OpenAPI schemas; the two renderings do not divide the world, they overlap it.
The exit is `temper-ui` importing its HTTP-contract types from `temper-ts/schema` instead and
retiring those 103 overlapping ts-rs types, leaving ts-rs to the ~30 non-HTTP wire types it alone
can express. That migration hasn't happened yet — it needs the `file:`-vs-published question
settled first (`temper-ui` is a bun workspace member; `temper-ts` deliberately is not) — but the
export exists now so that migration has something to reach for.

## Testing

```bash
npm test           # vitest
npm run typecheck  # tsc --noEmit, then tsc -p tsconfig.test.json — src AND tests, both strict
npm run build      # tsc, emits dist/ (including dist/generated/schema.js + .d.ts)
```

`src/testing/` exports `startMockApi` and `startMockIssuer` (as `temper-ts/testing`) — an
in-process HTTP mock API and a mock OAuth issuer (both a `temper-as`-flavored and an
Auth0-flavored mint), used by this package's own tests and importable by consumers (the steward
uses them for its own auth tests).
