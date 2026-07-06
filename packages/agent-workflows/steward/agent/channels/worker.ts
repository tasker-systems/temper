import { defineChannel, GET } from "eve/channels";

/**
 * Internal fan-out channel: a `receive`-only channel so the code schedules can start ONE autonomous
 * agent session per claimed job via `receive(worker, …)`.
 *
 * Why not the `eve` channel: the default eve channel is the HTTP API you talk *to* (POST
 * /eve/v1/session); it implements no `receive` hook, so cross-channel session starts route into it
 * fail (`channel "eve" does not implement receive()`). A schedule's only session-starting primitive
 * is `receive(channel, …)`, and `receive` calls the *target* channel's authored `receive` hook — so
 * we author a minimal one here.
 *
 * Why the (never-called) route: `receive` resolves its target two ways (eve
 * `channel/cross-channel-receive.js`) — first by reference identity (`registered.definition ===
 * passed`), then by route-fingerprint (the sorted `METHOD path` set). Across eve's compile boundary
 * the schedule's imported `worker` is a *different* object than the registered channel, so reference
 * identity misses; and `createRouteFingerprint` returns **null when `routes.length === 0`**, so a
 * routes-empty channel is unresolvable (`… is not registered in this agent's channels/` — the exact
 * error the earlier `routes: []` version threw live). The fingerprint is computed from route shape,
 * which is identical on both sides of the compile boundary, so ONE route makes the channel resolvable.
 *
 * The adapter is already present: `defineChannel` always builds one, and a behaviorless channel (no
 * state/context/events/fetchFile — adding a route does not change that) gets the framework's
 * `{ kind: "http" }` adapter, the supported fast-path shape the runtime rehydrates at every workflow
 * step. That is all `send()` needs (eve's own markdown schedules start task-mode sessions through an
 * even barer `{ kind: "schedule" }` adapter).
 *
 * The hook just `send`s the message as a **task-mode** run (run-to-completion, cannot park — matching
 * the old markdown steward): the session is a fresh run of the steward agent, which does the
 * authored-4 over its temper connection (M2M-authed) and completes. There is no human channel to
 * deliver output to — the work IS the tool calls into temper. A fresh continuation token per call
 * guarantees an independent session per claimed job.
 */
export default defineChannel({
  // One inert route so `receive(worker, …)` resolves this channel by route-fingerprint. A
  // `receive`-only channel is never actually invoked over HTTP, so the handler just 404s — the route
  // exists solely to give the channel a non-null fingerprint (empty routes → unresolvable).
  routes: [GET("/internal/steward-worker", async () => new Response(null, { status: 404 }))],
  async receive(input, { send }) {
    return send(input.message, {
      auth: input.auth,
      continuationToken: crypto.randomUUID(),
      mode: "task",
    });
  },
});
