<script lang="ts">
	/**
	 * The non-spatial mirror of the canvas.
	 *
	 * The field is drawn inside `<svg role="img">`, which is opaque to screen readers, so this is
	 * the accessible equivalent — **every node, not a sample**, which matters more here than it did
	 * on the predecessor: G2 withholds a caption from a crowded node, and this is where that node
	 * is still named. Visually hidden, revealed on keyboard focus so it is not a dead trap.
	 *
	 * Grouped by ARM rather than by home. On this surface both homes are ordinary; what a reader
	 * needs to tell apart is where the read stood from what it reached.
	 *
	 * **The headings are the read's own words.** The groups come from `model.arms`, in the order
	 * that read declared them — this component knows no arm names and can therefore never put one
	 * read's sentence above another read's marks, which is how the entry heading came to assert a
	 * question nobody had asked. An arm the read declared but returned nothing for is dropped: a
	 * heading over an empty list is a claim about a group that is not on screen.
	 */
	import type { GraphModel } from '$lib/graph/model';
	import { describeNodeLinks } from '$lib/graph/presentation';
	import { withGraphSelection } from '$lib/vault-url';

	let { model, url }: { model: GraphModel; url: URL } = $props();

	const groups = $derived(
		model.arms
			.map((arm) => ({ arm, nodes: model.nodes.filter((n) => n.arm === arm.key) }))
			.filter((g) => g.nodes.length > 0),
	);
</script>

<nav class="graph-a11y" aria-label="Every resource on this graph">
	{#each groups as g (g.arm.key)}
		<h2>{g.arm.label} · {g.nodes.length}</h2>
		<ul>
			{#each g.nodes as n (n.id)}
				<li>
					<a href={withGraphSelection(url, n.id)}>
						{n.title} — {n.doc_type} in {n.homeRef ?? 'home not reported'}, {describeNodeLinks(n)}
					</a>
				</li>
			{/each}
		</ul>
	{:else}
		<p>Nothing is drawn on this graph.</p>
	{/each}
</nav>

<style>
	/* Visually hidden until focused (standard sr-only + reveal-on-focus). */
	.graph-a11y {
		position: absolute;
		width: 1px;
		height: 1px;
		margin: -1px;
		padding: 0;
		overflow: hidden;
		clip: rect(0 0 0 0);
		white-space: nowrap;
		border: 0;
	}
	.graph-a11y:focus-within {
		position: absolute;
		top: 8px;
		left: 8px;
		z-index: 5;
		width: min(40rem, calc(100% - 16px));
		max-height: 70%;
		height: auto;
		margin: 0;
		padding: 12px 16px;
		overflow: auto;
		clip: auto;
		white-space: normal;
		background: rgba(20, 23, 29, 0.97);
		border: 1px solid rgba(255, 255, 255, 0.12);
		border-radius: 10px;
		color: #c9ced9;
		font-size: 13px;
	}
	.graph-a11y h2 {
		font-size: 12px;
		letter-spacing: 0.04em;
		margin: 6px 0 4px;
	}
	.graph-a11y a {
		color: #9fc4d6;
	}
</style>
