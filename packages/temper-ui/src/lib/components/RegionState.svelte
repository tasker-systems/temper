<script lang="ts">
	/**
	 * The region vocabulary on the web — arriving, empty, gave-up and failed. What each state
	 * says (its words, its marker glyph, its ARIA role) is `@tasker-systems/temper-design/region`,
	 * the one source the desktop reads too; this component only renders it. The reasons behind the
	 * words and the roles are stated there, once.
	 *
	 * `no-two-region-states-present-alike` is a negative-face clause: it is about states never
	 * presenting alike *anywhere*. If each page spells its own pending and failed markup, they
	 * drift page by page and the clause is enforced by nobody. One component means the states are
	 * defined once and every consumer inherits the distinction whether or not it was thinking
	 * about it.
	 *
	 * **The states differ by more than one channel**, deliberately: a sentence, a marker glyph, a
	 * colour and a border treatment each, so removing any one still leaves them resolvable. The
	 * colour and border are painted below until the web reads the contract's roles; then they come
	 * from the contract's `region.css` recipe, as they do on the desktop.
	 *
	 * **The fifth state, `present`, is deliberately not here.** A region that has its content does
	 * not need this component to describe it — the consumer renders its own markup in the
	 * `{:then}` branch.
	 *
	 * A `{:catch}` does not choose between `gave-up` and `failed` by hand — six call sites spelling
	 * one predicate is the drift this component exists to prevent. `regionStateFor` in `$lib/region`
	 * reads the discriminator; see `App.Error` for why it is a value rather than a class.
	 */
	import { type RegionStateName, regionWords } from '@tasker-systems/temper-design/region';

	let {
		state,
		label,
	}: {
		state: RegionStateName;
		label: string;
	} = $props();

	const said = $derived(regionWords(state, label));
</script>

<p class="region {state}" data-testid="region-{state}" role={said.role ?? undefined}>
	<span class="marker" aria-hidden="true">{said.marker}</span>
	<span class="words">
		{said.words}{#if said.qualifier}<span class="detail">{said.qualifier}</span>{/if}
	</span>
</p>

<style>
	/*
	 * Scoped `<style>` rather than Tailwind utilities, and the reason is the clause: these
	 * appearances have to be un-overridable from a consumer. A utility string in the class
	 * attribute of each call site is precisely the drift this component exists to prevent, and the
	 * graph components — the first consumers — are on a hard-coded palette rather than the app's
	 * Tailwind tokens, so a Tailwind spelling would read wrong in half the places it lands.
	 */
	.region {
		display: flex;
		align-items: baseline;
		gap: 8px;
		margin: 0;
		padding: 7px 9px;
		border-left: 2px solid;
		border-radius: 0 5px 5px 0;
		font-size: 12.5px;
		line-height: 1.5;
	}
	.marker {
		flex: none;
		font: 11px/1.4 ui-monospace, Menlo, monospace;
	}

	/* Arriving — dashed edge, cool blue, a pulsing ring. Unfinished, and says so three ways. */
	.arriving {
		border-left-style: dashed;
		border-left-color: rgba(143, 182, 232, 0.55);
		background: rgba(143, 182, 232, 0.05);
		color: #8fb6e8;
	}
	.arriving .marker {
		animation: pulse 1.1s ease-in-out infinite;
	}

	/* Empty — the quietest of the three. A settled fact about the reader's material, not a fault. */
	.empty {
		border-left-color: rgba(255, 255, 255, 0.12);
		background: rgba(255, 255, 255, 0.02);
		color: #79828f;
		font-style: italic;
	}

	/*
	 * Gave up — violet, dotted, and neither of the two it sits between. Orange beside orange at
	 * 12.5px is the one-channel trap spec §3.3 names, so the refusal takes a hue nothing else uses
	 * rather than a lighter shade of the failure. Dotted, because the wait was broken off rather
	 * than never started (dashed, arriving) or finished (solid).
	 */
	.gave-up {
		border-left-style: dotted;
		border-left-color: #8f7cc0;
		background: rgba(143, 124, 192, 0.07);
		color: #b9a6de;
	}
	.gave-up .detail {
		color: #9081b3;
	}

	/* Failed — warm, solid, heavier. Nothing about it reads as still in flight. */
	.failed {
		border-left-color: #c07a4a;
		background: rgba(192, 122, 74, 0.08);
		color: #e0a377;
		font-weight: 600;
	}
	.failed .detail {
		font-weight: 400;
		color: #b58b6a;
	}

	@keyframes pulse {
		0%,
		100% {
			opacity: 0.35;
		}
		50% {
			opacity: 1;
		}
	}

	/* The pulse is the one channel that is motion; a reader who has turned motion off still has the
	   others. */
	@media (prefers-reduced-motion: reduce) {
		.arriving .marker {
			animation: none;
		}
	}
</style>
