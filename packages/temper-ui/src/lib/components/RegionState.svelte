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
	 * colour and border are the contract's `region.css` recipe on `--tp-region-*` roles (imported
	 * in app.css), the same recipe the desktop paints with; this component adds no style of its own,
	 * because a call site that restyles a state is the drift the recipe exists to prevent.
	 *
	 * **The fifth state, `present`, is deliberately not here.** A region that has its content does
	 * not need this component to describe it — the consumer renders its own markup in the
	 * `{:then}` branch.
	 *
	 * A `{:catch}` does not choose between `gave-up` and `failed` by hand — six call sites spelling
	 * one predicate is the drift this component exists to prevent. `regionStateFor` in `$lib/region`
	 * reads the discriminator; see `App.Error` for why it is a value rather than a class.
	 *
	 * `detail` is what one region adds to its state's sentence — a failure's reason, say. It sits
	 * inside the element that carries the role, so it is announced with the sentence; beside it, a
	 * screen reader would hear that the read failed and never why. The desktop's component takes
	 * the same prop.
	 */
	import { type RegionStateName, regionWords } from '@tasker-systems/temper-design/region';

	let {
		state,
		label,
		detail,
	}: {
		state: RegionStateName;
		label: string;
		detail?: string | null;
	} = $props();

	const said = $derived(regionWords(state, label));
</script>

<p class="region-state" data-region={state} data-testid="region-{state}" role={said.role ?? undefined}>
	<span class="region-marker" aria-hidden="true">{said.marker}</span>
	<span>
		{said.words}{#if said.qualifier}<span class="region-qualifier">{said.qualifier}</span>{/if}{#if detail}<span class="region-detail">{detail}</span>{/if}
	</span>
</p>
