/**
 * Turning a resource's `ref` into a clickable in-app link, as the mention agent's
 * replies require.
 *
 * The model already receives a `ref` on every resource a tool returns (a bare
 * UUID or the decorated `slug-<uuid>` form). What it lacks on its own is the
 * DEPLOYMENT'S site URL — temperkb.io for the community app, the customer's own
 * host when self-hosted — so the link's host cannot live in the committed
 * `instructions.md`. This module builds that one deployment-specific line from
 * `TEMPER_API_URL`, and `instructions/citations.ts` feeds it in as a dynamic
 * instruction resolved at runtime.
 *
 * Kept pure (no eve imports, no direct `process.env` read) so the URL-shaping
 * forks are unit-testable without a session — the same discipline `identity.ts`
 * and `link.ts` follow.
 */

/**
 * The web app's per-resource path. `${base}${VAULT_RESOURCE_PATH}${ref}` is the
 * whole URL.
 *
 * The SvelteKit route `(app)/vault/r/[ident]` resolves this for BOTH a bare UUID
 * and the decorated `slug-<uuid>` form — its `parseRef` keeps only the trailing
 * UUID and ignores the slug half. So a `ref` from any tool result drops in
 * verbatim: no parsing, no slug stripping, a stale slug half is harmless.
 */
export const VAULT_RESOURCE_PATH = "/vault/r/";

/**
 * Normalize the deployment's public base URL: trim surrounding whitespace and
 * any trailing slashes so `${base}${VAULT_RESOURCE_PATH}` can never double one.
 * Returns `null` for an absent or blank value.
 */
export function normalizeSiteBaseUrl(raw: string | null | undefined): string | null {
  const trimmed = (raw ?? "").trim().replace(/\/+$/, "");
  return trimmed.length > 0 ? trimmed : null;
}

/**
 * The always-on instruction that binds the `{BASE}` placeholder in
 * `instructions.md`'s citation rule to THIS deployment's site URL.
 *
 * Runtime, not build time. The site URL is a per-deployment value, and this
 * agent reads all of its deployment config at request time (see CLAUDE.md, "read
 * at request time") so an unset variable fails a mention rather than the deploy.
 * `instructions/citations.ts` calls this from a `session.started` resolver, so
 * the `TEMPER_API_URL` passed here is the running function's own environment.
 *
 * When the base URL is absent the agent still answers — it cites by title and
 * says a link isn't available, rather than emitting a broken or invented URL.
 * In practice `onAppMention` has already `requireEnv`'d `TEMPER_API_URL` before
 * any turn dispatches, so the null arm is defence, not the common path.
 */
export function citationLinkInstruction(rawBaseUrl: string | null | undefined): string {
  const base = normalizeSiteBaseUrl(rawBaseUrl);
  if (base === null) {
    return [
      "This deployment has no site base URL configured, so you cannot build clickable",
      "resource links right now. Still cite every source by its title, and say plainly",
      "that a link isn't available — never guess or invent a URL.",
    ].join(" ");
  }
  return [
    `The \`{BASE}\` referenced in your citation rule is this temper deployment's site URL: ${base}`,
    "",
    `So a resource citation is a Slack link of the form \`<${base}${VAULT_RESOURCE_PATH}{ref}|{title}>\` —`,
    "`{ref}` copied verbatim from the resource's own `ref` field, `{title}` its title. For example, a",
    `resource with ref \`spike-report-otel-019f943a-745e-7eb1-b473-4aebeac05151\` and title "Spike report"`,
    `becomes \`<${base}${VAULT_RESOURCE_PATH}spike-report-otel-019f943a-745e-7eb1-b473-4aebeac05151|Spike report>\`.`,
  ].join("\n");
}
