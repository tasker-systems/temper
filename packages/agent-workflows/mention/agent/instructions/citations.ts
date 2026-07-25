import { defineDynamic, defineInstructions } from "eve/instructions";

import { citationLinkInstruction } from "../lib/citation-link.js";

/**
 * The one deployment-specific line of the system prompt: the site URL that turns
 * a cited resource's `ref` into a clickable link.
 *
 * `agent/instructions.md` (the always-on static prompt) owns the Slack mrkdwn
 * rules and the citation DISCIPLINE, and writes its link rule against a `{BASE}`
 * placeholder. This resolver binds that placeholder to the running deployment's
 * `TEMPER_API_URL` — temperkb.io for the community app, the customer's host when
 * self-hosted — so the committed instructions stay portable and no host is baked
 * into the repo.
 *
 * ## Why dynamic (runtime), not `instructions.ts` (build time)
 *
 * A module-backed `instructions.ts` runs ONCE at build and eve captures its text
 * into the manifest, so it would freeze whatever `TEMPER_API_URL` happened to be
 * set at `eve build` — which in CI is nothing (the build runs with only a
 * placeholder `TEMPER_MCP_URL`). `defineDynamic` resolves on `session.started`
 * at runtime, matching this agent's "read every deployment variable at request
 * time" rule (see CLAUDE.md): the value is the deployed function's own env, and
 * an unset one degrades gracefully (`citationLinkInstruction`'s null arm) rather
 * than shipping a stale or empty host.
 *
 * The env read lives INSIDE the handler, never at module load, so `eve build`
 * discovery does not depend on `TEMPER_API_URL` being present.
 *
 * ## Coexistence with the static file
 *
 * A root `agent/instructions.md` and an `agent/instructions/` directory combine:
 * the root file's text comes first, then the sorted directory entries. So this
 * line is appended AFTER the static rules that reference `{BASE}` — the forward
 * reference resolves in the assembled prompt.
 */
export default defineDynamic({
  events: {
    "session.started": () =>
      defineInstructions({
        markdown: citationLinkInstruction(process.env.TEMPER_API_URL),
      }),
  },
});
