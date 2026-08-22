import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import { sentenceOf } from '../../test/sentence';
import RegionState from './RegionState.svelte';

/**
 * Every state the vocabulary spells, enumerated once.
 *
 * The differential tests below read this list rather than naming states inline, so a state added to
 * the component without being added here is a state that renders untested — and adding it here
 * without teaching it its own words fails on the next line.
 */
const STATES = ['arriving', 'empty', 'gave-up', 'failed'] as const;
type State = (typeof STATES)[number];

const html = (state: State) =>
	render(RegionState, { props: { state, label: 'history' } }).container.innerHTML;

/**
 * The sentence a state says, with its decorative marker stripped.
 *
 * Raw `textContent` is not enough and the probe proved it: wording the give-up exactly like the
 * failure left the differential below green, because `⊘` and `⚠` differ and that was all the
 * comparison needed. Spec §3.3's whole point is that one channel is not a distinction.
 */
const words = (state: State) =>
	sentenceOf(render(RegionState, { props: { state, label: 'history' } }).container);

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

	/**
	 * The refusal (spec §5.4) has to say what the system stopped waiting FOR.
	 *
	 * "Timeout" on its own is the failure this state exists to avoid: it describes the mechanism and
	 * names nothing the reader was waiting for.
	 */
	it('C2: a region the system gave up on names the read, and does not say a bare "timeout"', () => {
		const el = render(RegionState, { props: { state: 'gave-up', label: 'history' } }).container;
		const words = el.textContent?.toLowerCase() ?? '';
		expect(words).toContain('history');
		expect(words).toContain('stopped waiting');
		expect(words).not.toContain('something went wrong');
		expect(words.replace(/history/g, '').trim()).not.toBe('timeout');
	});

	// C4 — the differential test. It asserts the states are pairwise unlike, without asserting what
	// any of them looks like, so it survives a redesign of all of them.
	it('C4: no two states present alike', () => {
		expect(new Set(STATES.map(html)).size).toBe(STATES.length);
	});

	// The clause is about what a READER can resolve; markup differing by one attribute is not that.
	it('C4: the states differ in their WORDS, not only their styling', () => {
		expect(new Set(STATES.map(words)).size).toBe(STATES.length);
	});

	/**
	 * The pair this task exists for, asserted on its own rather than left inside the sweep above.
	 *
	 * A give-up and a failure are the two states a reader is most likely to conflate — both are
	 * terminal, both mean no content — so the sweep going green on three other pairs must not be
	 * able to carry this one.
	 */
	it('C4: a give-up does not present like a failure', () => {
		expect(words('gave-up')).not.toBe(words('failed'));
		expect(html('gave-up')).not.toBe(html('failed'));
	});
});
