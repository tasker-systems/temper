import { defineChannel, GET } from "eve/channels";

/**
 * Internal fan-out channel for the CITATION AUDITOR — the auditor twin of `worker.ts`.
 *
 * It is a separate channel for one mechanical reason, and it is worth stating because "just reuse
 * `worker`" is the obvious first instinct: eve resolves a `receive` target by reference identity
 * first and then by **route fingerprint** — the sorted `METHOD path` set (eve
 * `channel/cross-channel-receive.js`). Across eve's compile boundary the schedule's imported channel
 * object is a different object than the registered one, so identity misses and the fingerprint
 * decides. Two channels sharing a route path would therefore share a fingerprint, and `to(…)` would
 * refuse to resolve either (eve throws on a multi-match). The distinct route below is what keeps the
 * auditor's fan-out addressable as itself.
 *
 * (The rest of the reasoning in `worker.ts` applies verbatim and is not repeated: why a `receive`-only
 * channel is needed at all, why `routes: []` is unresolvable, and why the never-called route still
 * has to exist.)
 *
 * The hook `send`s the message from a fresh channel-local address: each claimed job becomes one
 * independent session, guaranteed independent by that fresh address. There is no human channel to
 * deliver output to — the work IS the tool calls into temper. (No `task` mode since eve 0.67 — see
 * `worker.ts` on why the session parks rather than completes.)
 */
export default defineChannel({
  // One inert route, distinct from `worker.ts`'s, so this channel's fingerprint is its own.
  routes: [GET("/internal/auditor-worker", async () => new Response(null, { status: 404 }))],
  async receive(input, { from }) {
    return from(crypto.randomUUID()).send(input.message, { auth: input.auth });
  },
});
