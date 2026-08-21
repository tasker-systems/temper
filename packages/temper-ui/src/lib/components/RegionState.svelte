<script lang="ts">
	/**
	 * The four-state region vocabulary — **the single place** arriving, empty and failed are given
	 * their appearance and their words.
	 *
	 * `no-two-region-states-present-alike` is a negative-face clause: it is about states never
	 * presenting alike *anywhere*. If each page spells its own pending and failed markup, the three
	 * drift page by page and the clause is enforced by nobody. One component means the states are
	 * defined once and every consumer inherits the distinction whether or not it was thinking about
	 * it.
	 *
	 * **The states differ by more than one channel**, deliberately. Spec §3.3: a test asserting two
	 * renders differ is satisfied by a single character or one pixel of colour, while the clause is
	 * about what a *reader* can resolve — a grey "no history" beside a grey "history unavailable" at
	 * 11px passes the test and fails the clause. So each state carries its own **sentence**, its own
	 * **marker glyph**, its own **colour**, its own **border treatment**, and its own **ARIA role**.
	 * Removing any one of those still leaves the three resolvable.
	 *
	 * Two rules the words obey, and both come from the register:
	 *
	 * - **A pending region carries text**, never a bare shimmer, so it reaches the accessibility
	 *   tree rather than being a silent animation.
	 * - **A failed region names what failed** — "history unavailable", never "something went wrong" —
	 *   and says that nothing was read, which is exactly what separates it from `empty`. `empty`
	 *   asserts *there is nothing here*; that is a claim about the reader's material, and a failed
	 *   read has verified no such thing.
	 *
	 * `label` names the region ("history", "excerpt", "connections") and is composed into all three
	 * sentences, so a consumer cannot word its arrival and its failure out of step with each other.
	 *
	 * **The fourth state, `present`, is deliberately not here.** A region that has its content does
	 * not need this component to describe it — the consumer renders its own markup in the `{:then}`
	 * branch. An earlier draft took `children` for that; it was removed because no consumer needs
	 * it and passing both would have silently dropped the state's words.
	 */
	let {
		state,
		label,
	}: {
		state: 'arriving' | 'empty' | 'failed';
		label: string;
	} = $props();

	// Only the failure opens its sentence with the label, so only it needs the capital.
	const named = $derived(label.charAt(0).toUpperCase() + label.slice(1));
</script>

{#if state === 'arriving'}
	<!-- `role="status"` announces politely: an arrival is not an interruption. -->
	<p class="region arriving" data-testid="region-arriving" role="status">
		<span class="marker" aria-hidden="true">◌</span>
		<span class="words">Loading {label}…</span>
	</p>
{:else if state === 'empty'}
	<p class="region empty" data-testid="region-empty">
		<span class="marker" aria-hidden="true">—</span>
		<span class="words">No {label}.</span>
	</p>
{:else}
	<!-- `role="alert"` because a failure is not merely a later arrival; it will not resolve. -->
	<p class="region failed" data-testid="region-failed" role="alert">
		<span class="marker" aria-hidden="true">⚠</span>
		<span class="words">
			{named} unavailable<span class="detail"> — nothing here was read.</span>
		</span>
	</p>
{/if}

<style>
	/*
	 * Scoped `<style>` rather than Tailwind utilities, and the reason is the clause: these three
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
	   other four. */
	@media (prefers-reduced-motion: reduce) {
		.arriving .marker {
			animation: none;
		}
	}
</style>
