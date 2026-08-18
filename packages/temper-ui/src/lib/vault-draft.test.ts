import { describe, expect, it } from 'vitest';
import {
	draftNavigationSettled,
	draftNavigationStarted,
	draftTyped,
	draftUrlChanged,
	newDraft,
} from './vault-draft';

describe('draftUrlChanged', () => {
	it('follows a URL that changed from outside', () => {
		// Back button, or a shared link — nothing is outstanding here, so the URL wins.
		const state = draftUrlChanged(newDraft('atlas'), 'steward');
		expect(state.draft).toBe('steward');
		expect(state.requested).toBe('steward');
	});

	it('leaves the draft alone when the URL echoes what was asked for', () => {
		let state = newDraft('');
		state = draftTyped(state, 'atlas');
		state = draftNavigationStarted(state);
		state = draftNavigationSettled(state);
		expect(draftUrlChanged(state, 'atlas').draft).toBe('atlas');
	});

	// THE REPRO. Type `hello`, pause past the 300ms debounce so the navigation goes, keep typing
	// ` world` while the round-trip is in the air. The old bare `$effect` resync landed `hello`
	// back on top of `hello world`; those characters were discarded and never sent.
	it('does not clobber characters typed since the navigation started', () => {
		let state = newDraft('');
		state = draftTyped(state, 'hello');
		state = draftNavigationStarted(state);
		state = draftTyped(state, 'hello world');
		state = draftNavigationSettled(state);

		state = draftUrlChanged(state, 'hello');
		expect(state.draft).toBe('hello world');
	});

	it('sends the characters typed since on the next navigation', () => {
		let state = newDraft('');
		state = draftTyped(state, 'hello');
		state = draftNavigationStarted(state);
		state = draftTyped(state, 'hello world');
		state = draftUrlChanged(state, 'hello');
		state = draftNavigationStarted(state);
		expect(state.requested).toBe('hello world');
	});

	it('ignores a URL value while a navigation of its own is still in flight', () => {
		let state = newDraft('');
		state = draftTyped(state, 'hello');
		state = draftNavigationStarted(state);
		// Arrives before the goto promise settles — the effect fires on render, not on resolve.
		expect(draftUrlChanged(state, '').draft).toBe('hello');
	});

	it('follows the URL again once its own navigation has settled', () => {
		let state = newDraft('');
		state = draftTyped(state, 'hello');
		state = draftNavigationStarted(state);
		state = draftNavigationSettled(state);
		expect(draftUrlChanged(state, 'elsewhere').draft).toBe('elsewhere');
	});

	// The URL carries the trimmed value, so an equivalent echo must not eat a space the user
	// is still typing behind.
	it('does not strip trailing whitespace the user is mid-word on', () => {
		let state = newDraft('');
		state = draftTyped(state, 'hello ');
		state = draftNavigationStarted(state);
		state = draftNavigationSettled(state);
		expect(draftUrlChanged(state, 'hello').draft).toBe('hello ');
	});

	it('follows a URL that cleared the filter', () => {
		expect(draftUrlChanged(newDraft('atlas'), '').draft).toBe('');
	});

	it('never lets the in-flight count go negative', () => {
		expect(draftNavigationSettled(newDraft('')).inflight).toBe(0);
	});
});
