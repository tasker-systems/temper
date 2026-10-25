import { describe, expect, it } from 'vitest';
import { nextSection } from './handoff.js';

describe('the next steps a session recorded', () => {
	it('quotes the section under the first heading that starts with "Next", up to its sibling', () => {
		const md = [
			'## What this session did',
			'Built the shell.',
			'## Next: the home view — handoff preamble',
			'Take up the home view.',
			'',
			'### Detail',
			'Plan first.',
			'## Environment notes',
			'Install things.',
		].join('\n');
		expect(nextSection(md)).toEqual({
			heading: 'Next: the home view — handoff preamble',
			text: 'Take up the home view.\n\n### Detail\nPlan first.',
			truncated: false,
		});
	});

	it('finds it at any level, and stops at a higher heading', () => {
		const md = '# Session\n### Next steps\n- one\n- two\n## Carried forward\n- three';
		expect(nextSection(md)?.text).toBe('- one\n- two');
	});

	it('is null when the session has no such heading — nothing is invented', () => {
		expect(nextSection('## What this session did\nA lot.\n## Nextcloud notes\nNo.')).toBeNull();
		expect(nextSection('')).toBeNull();
	});

	it('is null when the heading has nothing under it', () => {
		expect(nextSection('## Next\n\n## Later\nSomething.')).toBeNull();
	});

	it('ignores a heading-looking line inside a code fence', () => {
		const md = '## Notes\n```\n## Next\nnot this\n```\n## Next\nThis.';
		expect(nextSection(md)?.text).toBe('This.');
	});

	it('is bounded, and says it was cut', () => {
		const long = `## Next\n${'word '.repeat(400)}`;
		const next = nextSection(long, 100);
		expect(next?.truncated).toBe(true);
		expect(next?.text.length).toBeLessThanOrEqual(101);
		expect(next?.text.endsWith('…')).toBe(true);
	});

	// The heading parse was rewritten from `/^(#{1,6})\s+(.*?)\s*#*\s*$/`; these are the shapes in
	// which that pattern's answer is easiest to change by accident, with the answers it gave.
	it.each([
		['## Next', 'Next'],
		['## Next ##', 'Next'],
		['## Next steps   ###   ', 'Next steps'],
		['## Next#', 'Next'],
		['## Next # #', 'Next #'],
		['##\tNext\t', 'Next'],
	])('reads the heading %j as %j', (line, title) => {
		expect(nextSection(`${line}\nGo.`)?.heading).toBe(title);
	});

	it('does not read a line without space after the hashes, or seven hashes, as a heading', () => {
		expect(nextSection('##Next\nGo.')).toBeNull();
		expect(nextSection('####### Next\nGo.')).toBeNull();
	});

	// A heading line, a long non-space run, a long space run, then a non-space: the shape on which
	// the old pattern backtracked super-linearly (~240ms at 4,000 characters, about eight-fold per
	// doubling, so ~2s at 8,000). The parse runs synchronously in a server load, where no promise
	// timer can interrupt it, so the time is asserted directly: a test timeout could not stop it either.
	it('reads an adversarial heading line in linear time', () => {
		const line = `# ${'a'.repeat(4_000)}${' '.repeat(4_000)}b`;
		const started = performance.now();
		expect(nextSection(`${line}\n## Next\nGo.`)?.text).toBe('Go.');
		expect(performance.now() - started).toBeLessThan(200);
	});
});
