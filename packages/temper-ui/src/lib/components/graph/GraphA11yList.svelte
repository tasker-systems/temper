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
	 * needs to tell apart is what they asked for from what a walk reached.
	 */
	import type { GraphModel, NodeArm } from '$lib/graph/model';
	import { describeArm, whereOf } from '$lib/graph/presentation';
	import { withGraphSelection } from '$lib/vault-url';

	let { model, url }: { model: GraphModel; url: URL } = $props();

	const ARMS: NodeArm[] = ['seed', 'survey', 'walk'];
	const groups = $derived(
		ARMS.map((arm) => ({ arm, nodes: model.nodes.filter((n) => n.arm === arm) })).filter(
			(g) => g.nodes.length > 0,
		),
	);
</script>

<nav class="graph-a11y" aria-label="Every resource on this graph">
	{#each groups as g (g.arm)}
		<h2>{describeArm(g.arm)} · {g.nodes.length}</h2>
		<ul>
			{#each g.nodes as n (n.id)}
				<li>
					<a href={withGraphSelection(url, n.id)}>
						{n.title} — {n.doc_type} in {n.homeRef ?? 'home not reported'}, {n.degree}
						{n.degree === 1 ? 'link' : 'links'}
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
