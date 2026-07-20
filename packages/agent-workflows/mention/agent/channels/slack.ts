import { defaultSlackAuth, slackChannel } from "eve/channels/slack";
import type { SlackContext, SlackMessage } from "eve/channels/slack";

import { deliverEphemeral } from "../lib/ephemeral.js";
import { decideIdentity, linkedPrompt, unlinkedPrompt } from "../lib/identity.js";
import { requestLinkState } from "../lib/link.js";
import { ephemeralEvents } from "./events.js";

/**
 * Slack channel for the @temper mention agent.
 *
 * Route: `POST /eve/v1/slack` (eve's default). HTTP only — no Socket Mode.
 *
 * Credentials are omitted entirely, so eve falls back to `SLACK_BOT_TOKEN` and
 * `SLACK_SIGNING_SECRET` from the environment. That is deliberate for T1: the
 * app is reproducible from the committed `slack-app-manifest.yml` with no
 * feature-flagged Vercel CLI in the loop. Vercel Connect
 * (`connectSlackCredentials`) is the eventual path — see CLAUDE.md.
 */
export default slackChannel({
  /**
   * Every eve default that posts PUBLICLY is replaced with a channel-root
   * ephemeral. A dispatched turn will run under the mentioning human's full
   * temper reach, so the answer — and every failure message that can quote it
   * — belongs to that human alone. See `events.ts`, including the documented
   * residual gap on `authorization.required`.
   */
  events: ephemeralEvents,

  /**
   * Replaces eve's default mention pipeline (auth derivation + a "Thinking..."
   * typing indicator). We keep the same auth derivation and add the human-only
   * gate, so bots cannot drive a turn.
   *
   * Thrown errors here are caught and logged by eve and the mention is dropped,
   * which is why the best-effort post is wrapped: a Slack hiccup must not
   * silently swallow the mention.
   */
  async onAppMention(ctx: SlackContext, message: SlackMessage) {
    const decision = decideIdentity(defaultSlackAuth(message, ctx));

    // Bots and authorless events never dispatch. Dropping is silent by design:
    // an error reply to a bot is how mention loops start.
    if (decision.kind === "rejected") return null;

    // The link challenge is a CREDENTIAL: whoever opens it binds their temper identity to
    // this Slack principal. So it goes to the mentioning user ONLY — never `thread.post`,
    // which is public. The user id comes from `attributes.user_id`; NEVER from parsing
    // principalId, which has 2-4 segments.
    const userId = decision.auth.attributes.user_id;
    if (typeof userId !== "string") {
      // We cannot postEphemeral without a user id, so this drop is forced — but a silent
      // one leaves the user with nothing and us with no trace. Log the whole principal
      // (never a parse of it) so the drop is at least diagnosable. No `thread.post`
      // fallback: the challenge is a credential and must never go to a public channel.
      console.warn("dropping mention: no user_id on attributes", {
        principalId: decision.principalId,
      });
      return null;
    }

    try {
      // Ask what to SAY, not for a URL. A linked user gets no challenge and mints no intent;
      // asking for a URL unconditionally is what re-prompted linked users forever.
      const link = await requestLinkState(decision.principalId);
      const reply =
        link.status === "linked"
          ? linkedPrompt(link.handle)
          : unlinkedPrompt(link.authorize_url);

      // Deliver privately, at the CHANNEL ROOT. `deliverEphemeral` owns the why — the
      // `thread_ts`-inheritance trap and the `{ ok, error }` failure surface — and is now
      // shared with the event overrides in `events.ts`.
      await deliverEphemeral(ctx, userId, reply);
    } catch (err) {
      // requestLinkState failed (API/network). eve swallows a thrown handler, so say something.
      console.error("link state lookup failed", err);
      await deliverEphemeral(
        ctx,
        userId,
        "I couldn't check your temper account just now. Please try again in a moment.",
      );
    }

    // Deliberately DROP rather than dispatch, on BOTH arms. Unlinked, a turn would run the
    // model under no identity — no tools, nothing to ground an answer in. Linked, there is
    // still nothing to dispatch TO: reads under proven identity are a later task. Until then
    // the prompt IS the reply, and the default `message.completed` handler would post an
    // ungrounded turn if we dispatched one.
    return null;
  },
});
