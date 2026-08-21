import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import RegionState from './RegionState.svelte';

const html = (state: 'arriving' | 'empty' | 'failed') =>
	render(RegionState, { props: { state, label: 'history' } }).container.innerHTML;

describe('the four-state vocabulary', () => {
	it('C2: an arriving region says so, in words', () => {
		const el = render(RegionState, { props: { state: 'arriving', label: 'history' } }).container;
		expect(el.textContent?.toLowerCase()).toContain('history');
		expect(el.textContent?.trim()).not.toBe('');
	});

	it('C2: a failed region names WHAT failed, not "something went wrong"', () => {
		const el = render(RegionState, { props: { state: 'failed', label: 'history' } }).container;
		expect(el.textContent?.toLowerCase()).toContain('history');
		expect(el.textContent?.toLowerCase()).not.toContain('something went wrong');
	});

	// C4 — the differential test. It asserts the three states are pairwise unlike, without
	// asserting what any of them looks like, so it survives a redesign of all three.
	it('C4: no two states present alike', () => {
		const [arriving, empty, failed] = [html('arriving'), html('empty'), html('failed')];
		expect(arriving).not.toBe(empty);
		expect(empty).not.toBe(failed);
		expect(arriving).not.toBe(failed);
	});

	// The clause is about what a READER can resolve; markup differing by one attribute is not that.
	it('C4: the states differ in their WORDS, not only their styling', () => {
		const text = (s: 'arriving' | 'empty' | 'failed') =>
			render(RegionState, { props: { state: s, label: 'history' } }).container.textContent?.trim();
		const [a, e, f] = [text('arriving'), text('empty'), text('failed')];
		expect(new Set([a, e, f]).size).toBe(3);
	});
});
