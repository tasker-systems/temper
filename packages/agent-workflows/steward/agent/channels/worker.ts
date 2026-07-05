import { defineChannel } from "eve/channels";

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
 * The hook just `send`s the message as a **task-mode** run (run-to-completion, cannot park — matching
 * the old markdown steward): the session is a fresh run of the steward agent, which does the
 * authored-4 over its temper connection (M2M-authed) and completes. There is no human channel to
 * deliver output to — the work IS the tool calls into temper. A fresh continuation token per call
 * guarantees an independent session per claimed job.
 */
export default defineChannel({
  // No HTTP surface — this channel exists only as a `receive` target for the schedules.
  routes: [],
  async receive(input, { send }) {
    return send(input.message, {
      auth: input.auth,
      continuationToken: crypto.randomUUID(),
      mode: "task",
    });
  },
});
