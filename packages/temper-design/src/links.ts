// links.ts — body links: which hrefs in a body name a temper resource, and the words a link says
// when temper did not vouch for it.
//
// A body cites another resource as `[<title>](./<uuidv7>)`. The typed title is a hint, written
// once and never updated, so a surface resolves the ids against temper and the sanitize pass
// (`sanitize.ts`) rewrites each link from that answer. This module owns the half that needs no
// DOM: the matcher both halves agree on, and extraction, which a surface runs before it renders —
// on the web, in the page's server load.
//
// **Exact form only.** `./` and a 36-character UUID, with an optional trailing `/` because the
// desktop's router follows that form too; nothing looser. A decorated `slug-<uuid>` ref, an
// absolute URL or a bare UUID is left alone — reading any of them as a citation would be a guess.
//
// **Extraction is bounded and total.** It carries `prepareMarkdown`'s source bound, since it now
// runs on the server where any reader can make a body be parsed, and a body marked cannot lex
// yields no ids rather than a throw: every link in it then renders unchecked, which is what an
// unanswered link is.
import { Marked, type Tokens } from 'marked';
import { MAX_SOURCE_LENGTH } from './markdown.js';

/**
 * The most links one body resolves: the id-set bound the server's `ids` filter enforces (the
 * `IdSet` schema's `maxItems` in the published contract), so one body is one request. Links past
 * it render unchecked.
 */
export const BODY_LINK_CAP = 256;

/** What temper answered for one id. Nothing answered is not an entry: it renders unchecked. */
export type LinkResolution = { state: 'resolved'; title: string } | { state: 'absent' };

/** Answers by lowercase id. */
export type LinkTable = ReadonlyMap<string, LinkResolution>;

/**
 * Hover words for a link temper answered for with nothing the reader can see. One sentence for
 * every cause: a reader must not be able to tell a deleted resource from one they may not read.
 */
export const ABSENT_WORDS = 'Not found, or not visible to you';

/** Hover words for a link temper did not answer for: nothing was verified either way. */
export const UNCHECKED_WORDS = 'Not checked against temper';

const VAULT_LINK = /^\.\/([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\/?$/i;

/** The lowercase id an href cites, or `null` when it is not exactly a vault link. */
export function vaultLinkId(href: string): string | null {
	const m = VAULT_LINK.exec(href);
	return m ? m[1].toLowerCase() : null;
}

// Lexing only. The renderer's instance (`markdown.ts`) adds highlighting, which changes how code
// renders, not what the lexer finds — so a plain instance sees the same links.
const lexer = new Marked();

/**
 * The distinct vault-link ids in a body, in first-occurrence order, at most
 * {@link BODY_LINK_CAP}; `dropped` counts the distinct ids past the cap.
 */
export function extractVaultLinks(markdown: string): { ids: string[]; dropped: number } {
	const none = { ids: [], dropped: 0 };
	if (typeof markdown !== 'string' || markdown.length > MAX_SOURCE_LENGTH) return none;
	const seen = new Set<string>();
	try {
		lexer.walkTokens(lexer.lexer(markdown), (token) => {
			if (token.type !== 'link') return;
			const id = vaultLinkId((token as Tokens.Link).href);
			if (id) seen.add(id);
		});
	} catch {
		return none;
	}
	const all = [...seen];
	return { ids: all.slice(0, BODY_LINK_CAP), dropped: Math.max(0, all.length - BODY_LINK_CAP) };
}
