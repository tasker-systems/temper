// vault-draft.ts
/**
 * When a URL-derived value may overwrite what the user is typing.
 *
 * The filter bar's free-text controls debounce into a navigation, so the URL is always
 * catching up to the keyboard. `FilterBar` originally resynced with a bare
 * `$effect(() => { qDraft = filters.q ?? '' })`, which fired on EVERY navigation — including
 * the one the control itself had just started. Type `hello`, pause past the 300ms debounce,
 * keep typing ` world` while the round-trip is in the air, and the effect landed `hello` back
 * on top of `hello world`: the characters typed since were discarded and never sent.
 *
 * Deleting the sync is not the fix — the field must still follow a URL that changed from
 * outside (browser back/forward, a pasted link, another control's navigation). So the rule is
 * narrower than "always follow" and narrower than "never follow":
 *
 *   Adopt an incoming URL value only when this control has nothing outstanding —
 *   no navigation of its own still in flight, and no typed characters not yet sent.
 *
 * Both guards are load-bearing. `inflight` catches the value coming back while the round-trip
 * is still settling; the `draft.trim() !== requested` check catches the case where the user
 * typed again after the debounce fired but before the answer arrived — the actual repro.
 *
 * Kept as a pure reducer beside its `.test.ts` rather than as component state, so the rule
 * that decides whether a keystroke survives is pinned by tests instead of by a browser.
 */

export interface DraftState {
	/** What the input shows. */
	draft: string;
	/** The value this control last asked the URL to carry; `''` means "no param". */
	requested: string;
	/** Navigations this control started that have not settled yet. */
	inflight: number;
}

/** A draft seeded from the URL, with nothing outstanding. */
export function newDraft(initial: string): DraftState {
	return { draft: initial, requested: initial, inflight: 0 };
}

/** The user typed. The draft is now ahead of the URL until a navigation catches it up. */
export function draftTyped(state: DraftState, text: string): DraftState {
	return { ...state, draft: text };
}

/** A navigation carrying the current draft has been started. */
export function draftNavigationStarted(state: DraftState): DraftState {
	return { ...state, requested: state.draft.trim(), inflight: state.inflight + 1 };
}

/** A navigation this control started has settled. */
export function draftNavigationSettled(state: DraftState): DraftState {
	return { ...state, inflight: Math.max(0, state.inflight - 1) };
}

/**
 * The URL now says `incoming` for this filter. Adopt it only if nothing typed here is
 * outstanding — otherwise the URL is behind the keyboard and the keyboard wins.
 */
export function draftUrlChanged(state: DraftState, incoming: string): DraftState {
	if (state.inflight > 0) return state;
	// Unsent keystrokes: the URL is describing an older draft than the one on screen.
	if (state.draft.trim() !== state.requested) return state;
	// Our own value echoing back. Compare against the trimmed draft so a trailing space the
	// user is still typing behind is not swallowed by an equivalent URL value.
	if (incoming === state.draft.trim()) return { ...state, requested: incoming };
	return { draft: incoming, requested: incoming, inflight: 0 };
}
