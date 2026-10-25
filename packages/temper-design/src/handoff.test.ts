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
});
