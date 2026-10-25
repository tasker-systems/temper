/**
 * The latest handoff: the reader's most recently updated session, and the next steps it recorded.
 * Both surfaces show it on arrival under the same name. `nextSection` is copied here unchanged
 * from the desktop (`temper-contrib`, `apps/desktop/src/lib/shell/next-section.ts`); once the
 * desktop imports it from here instead of keeping its own copy, the two cannot quote the same
 * session differently. Until then they can drift.
 *
 * The next steps a session recorded: the section under the first heading that starts with "Next"
 * ("Next", "Next steps", "Next: the home view"), at any level, up to the next heading of the same
 * or a higher level. Quoted as written, trimmed and bounded — never summarised, and `null` when the
 * session has no such heading, so nothing is ever invented in its place.
 */

/** The name both surfaces give the latest handoff. */
export const HANDOFF_LABEL = 'latest handoff';

/** How much of the section is quoted before it says it continues. */
export const NEXT_BOUND = 600;

export type NextSection = { heading: string; text: string; truncated: boolean };

const HEADING = /^(#{1,6})\s+(.*?)\s*#*\s*$/;

export function nextSection(markdown: string, bound: number = NEXT_BOUND): NextSection | null {
	const lines = markdown.split(/\r?\n/);
	let fence = false;
	let start = -1;
	let level = 0;
	let heading = '';
	const body: string[] = [];
	for (let i = 0; i < lines.length; i++) {
		const line = lines[i];
		if (/^\s*(```|~~~)/.test(line)) fence = !fence;
		const match = fence ? null : HEADING.exec(line);
		if (start < 0) {
			if (match && /^next\b/i.test(match[2])) {
				start = i;
				level = match[1].length;
				heading = match[2];
			}
			continue;
		}
		if (match && match[1].length <= level) break;
		body.push(line);
	}
	if (start < 0) return null;
	const text = body.join('\n').trim();
	if (!text) return null;
	if (text.length <= bound) return { heading, text, truncated: false };
	const cut = text.slice(0, bound);
	const atWord = cut.slice(0, Math.max(cut.lastIndexOf(' '), bound - 40));
	return { heading, text: `${atWord.trimEnd()}…`, truncated: true };
}
