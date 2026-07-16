import { defaultSlackAuth, slackChannel } from "eve/channels/slack";
import type { SlackContext, SlackMessage } from "eve/channels/slack";

import { decideIdentity, unlinkedPrompt } from "../lib/identity.js";

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
    const auth = defaultSlackAuth(message, ctx);
    const decision = decideIdentity(auth);

    // Bots and authorless events never dispatch. Dropping is silent by design:
    // an error reply to a bot is how mention loops start.
    if (decision.kind === "rejected") return null;

    // T1's acceptance: the resolved principal is echoed back, whole.
    try {
      await ctx.thread.post(unlinkedPrompt(decision.principalId));
    } catch (error) {
      console.error("mention: unlinked prompt delivery failed", {
        principalId: decision.principalId,
        error,
      });
    }

    // The decision above already proved `auth` is a non-null human; it is
    // passed on verbatim because dispatch needs the whole SessionAuthContext
    // (attributes, authenticator, issuer), not just the principal.
    return { auth };
  },
});
