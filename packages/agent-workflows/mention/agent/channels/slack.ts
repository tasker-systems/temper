import { defaultSlackAuth, slackChannel } from "eve/channels/slack";
import type { SlackContext, SlackMessage } from "eve/channels/slack";

import { decideIdentity, linkedPrompt, unlinkedPrompt } from "../lib/identity.js";
import { requestLinkState } from "../lib/link.js";

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

      // Deliver privately, at the CHANNEL ROOT — not via `ctx.thread.postEphemeral`.
      //
      // eve's thread helper inherits the mention's `thread_ts`, so the ephemeral posts INTO a
      // thread the user isn't viewing, where an ephemeral is invisible and leaves no badge —
      // the symptom was total silence in the channel. A channel-root `chat.postEphemeral` (no
      // `thread_ts`) shows inline where the user actually mentioned. Still ephemeral, still
      // private-to-them: the unlinked arm carries a credential and must never go public.
      //
      // `ctx.slack.request` returns the raw Slack response instead of throwing on `ok:false`
      // (eve's typed `postEphemeral` throws, and eve's dispatcher then swallows the throw). So
      // on failure we can surface WHY — publicly, but with only the Slack error code, never the
      // reply. Silence is the one outcome we refuse to ship again.
      const res = await ctx.slack.request("chat.postEphemeral", {
        channel: ctx.slack.channelId,
        user: userId,
        text: reply,
      });
      if (!res.ok) {
        console.error("postEphemeral failed", { error: res.error });
        await ctx.thread.post({
          text: `I couldn't send you a private message (Slack: ${res.error ?? "unknown_error"}). Once that's sorted, mention me again.`,
        });
      }
    } catch (err) {
      // requestLinkState failed (API/network). eve swallows a thrown handler, so say something.
      console.error("link state lookup failed", err);
      await ctx.slack.request("chat.postEphemeral", {
        channel: ctx.slack.channelId,
        user: userId,
        text: "I couldn't check your temper account just now. Please try again in a moment.",
      });
    }

    // Deliberately DROP rather than dispatch, on BOTH arms. Unlinked, a turn would run the
    // model under no identity — no tools, nothing to ground an answer in. Linked, there is
    // still nothing to dispatch TO: reads under proven identity are a later task. Until then
    // the prompt IS the reply, and the default `message.completed` handler would post an
    // ungrounded turn if we dispatched one.
    return null;
  },
});
