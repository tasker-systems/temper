import { defaultSlackAuth, slackChannel } from "eve/channels/slack";
import type { SlackContext, SlackMessage } from "eve/channels/slack";

import { decideIdentity, unlinkedPrompt } from "../lib/identity.js";
import { requestAuthorizeUrl } from "../lib/link.js";

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
      const authorizeUrl = await requestAuthorizeUrl(decision.principalId);
      await ctx.thread.postEphemeral(userId, unlinkedPrompt(authorizeUrl));
    } catch (err) {
      // eve catches and logs a thrown error and drops the mention, so a failed intent
      // would be silent. Tell the user something honest instead of nothing.
      console.error("link intent failed", err);
      await ctx.thread.postEphemeral(
        userId,
        "I couldn't start the account-connect flow just now. Please try again in a moment.",
      );
    }

    // Deliberately DROP rather than dispatch (unchanged from T1). A turn under no identity
    // would run the model with no tools and nothing to ground an answer in, and the default
    // `message.completed` handler would post it. Until the link exists, the prompt IS the reply.
    return null;
  },
});
