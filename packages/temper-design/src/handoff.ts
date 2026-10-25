/**
 * The latest handoff: the reader's most recently updated session, and the next steps it recorded.
 * Both surfaces show it on arrival under the same name. `nextSection` is copied here from the
 * desktop, its heading parse rewritten without the backtracking pattern but answering identically, (`temper-contrib`, `apps/desktop/src/lib/shell/next-section.ts`); once the
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

/** An ATX heading's opener: one to six `#`, then whitespace. Anchored and unambiguous. */
const OPENER = /^(#{1,6})\s+/;

/**
 * A line read as an ATX heading — its level and its text, with any closing `#`s and the space
 * around them dropped — or `null`. The text is taken by slicing and trimming rather than by a
 * pattern with adjacent optional runs (`(.*?)\s*#*\s*$`), which can backtrack polynomially on a
 * long run of spaces in an uncontrolled body.
 */
function heading(line: string): { level: number; text: string } | null {
	const open = OPENER.exec(line);
	if (!open) return null;
	let text = line.slice(open[0].length).trimEnd();
	let end = text.length;
	while (end > 0 && text[end - 1] === '#') end--;
	text = text.slice(0, end).trimEnd();
	return { level: open[1].length, text };
}

export function nextSection(markdown: string, bound: number = NEXT_BOUND): NextSection | null {
	const lines = markdown.split(/\r?\n/);
	let fence = false;
	let start = -1;
	let level = 0;
	let title = '';
	const body: string[] = [];
	for (let i = 0; i < lines.length; i++) {
		const line = lines[i];
		if (/^\s*(```|~~~)/.test(line)) fence = !fence;
		const match = fence ? null : heading(line);
		if (start < 0) {
			if (match && /^next\b/i.test(match.text)) {
				start = i;
				level = match.level;
				title = match.text;
			}
			continue;
		}
		if (match && match.level <= level) break;
		body.push(line);
	}
	if (start < 0) return null;
	const text = body.join('\n').trim();
	if (!text) return null;
	if (text.length <= bound) return { heading: title, text, truncated: false };
	const cut = text.slice(0, bound);
	const atWord = cut.slice(0, Math.max(cut.lastIndexOf(' '), bound - 40));
	return { heading: title, text: `${atWord.trimEnd()}…`, truncated: true };
}
