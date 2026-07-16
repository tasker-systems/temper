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
    const decision = decideIdentity(defaultSlackAuth(message, ctx));

    // Bots and authorless events never dispatch. Dropping is silent by design:
    // an error reply to a bot is how mention loops start.
    if (decision.kind === "rejected") return null;

    // Posting here is a pre-dispatch side effect on the inbound webhook side —
    // the same seam eve's own default uses for its "Thinking..." indicator.
    await ctx.thread.post(unlinkedPrompt(decision.principalId));

    // Deliberately DROP rather than dispatch, even though this is a human we
    // resolved and `auth` is right here.
    //
    // T1 has no temper reach, so a dispatched turn would run the model with no
    // tools and nothing to ground an answer in — and since only `onAppMention`
    // is overridden, the DEFAULT `message.completed` handler would post that
    // answer to the thread. The user would get two replies: the prompt above,
    // then an LLM improvising about a knowledge base it cannot read. Worse than
    // useless — it is the "plausible answer under no identity" the agent's
    // instructions forbid.
    //
    // T2 lands the account link; the linked branch dispatches `{ auth }` then.
    // Until there is something a turn can honestly do, the prompt IS the reply.
    return null;
  },
});
