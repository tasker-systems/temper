import { defaultSlackAuth, slackChannel } from "eve/channels/slack";
import type { SlackContext, SlackMessage } from "eve/channels/slack";

import { deliverEphemeral } from "../lib/ephemeral.js";
import { decideIdentity, notVaultedPrompt, revokedPrompt, unlinkedPrompt } from "../lib/identity.js";
import { requestLinkState } from "../lib/link.js";
import { requestMintedToken } from "../lib/mint.js";
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

      if (link.status === "unlinked") {
        // Deliver privately, at the CHANNEL ROOT. `deliverEphemeral` owns the why — the
        // `thread_ts`-inheritance trap and the `{ ok, error }` failure surface — and is
        // shared with the event overrides in `events.ts`.
        await deliverEphemeral(ctx, userId, unlinkedPrompt(link.authorize_url));
        // DROP: a turn here would run the model under no identity — no tools, nothing to
        // ground an answer in. The prompt IS the reply.
        return null;
      }

      // PRE-FLIGHT THE MINT, before dispatching. The connection's `getToken` would mint too,
      // but a failure there is a FAILED TURN, and a failed turn is routed to the
      // `turn.failed` handler, which says one deliberately detail-free sentence
      // (`events.ts` — it must not quote model or tool output). That is exactly the generic
      // error each of these three states must NOT collapse into: "you were never vaulted"
      // and "your access was revoked" need different sentences and a remedy, and neither is
      // fixed by trying again. Minting here is what lets us say the true thing.
      //
      // This does mean a successful turn mints TWICE — once here, once inside `getToken`.
      // That is deliberately cheap, not overlooked: the server returns the CACHED access
      // token without touching the refresh token whenever it outlives a 5-minute skew
      // (`slack_grant_vault_service.rs`, `mint_access_token`: "Cached access token still
      // comfortably valid? Hand it back — no refresh, no RT rotation."). So the second mint
      // is a row read, never a second spend of the grant.
      const mint = await requestMintedToken(decision.principalId);

      switch (mint.status) {
        case "token":
          // DISPATCH. Returning `auth` is what gives the turn its identity — eve projects
          // this same `principalId` into the connection principal, so the tools run under
          // this human and no one else.
          return { auth: decision.auth };

        case "not_vaulted":
          await deliverEphemeral(ctx, userId, notVaultedPrompt(link.handle));
          // DROP: dispatching would fail in `getToken` and overwrite this specific,
          // actionable message with the generic turn-failure line.
          return null;

        case "revoked":
          await deliverEphemeral(ctx, userId, revokedPrompt(link.handle));
          // DROP, for the same reason.
          return null;
      }
    } catch (err) {
      // requestLinkState or requestMintedToken failed (API/network/secret drift). eve
      // swallows a thrown handler, so say something. This is the one genuinely generic
      // reply, and it is generic honestly: we do not know what is wrong, and unlike the
      // three states above, trying again really may work.
      console.error("temper account lookup failed", err);
      await deliverEphemeral(
        ctx,
        userId,
        "I couldn't check your temper account just now. Please try again in a moment.",
      );
      return null;
    }
  },
});
