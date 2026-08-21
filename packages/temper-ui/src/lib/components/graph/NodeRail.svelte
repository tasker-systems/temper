<script lang="ts">
	/**
	 * The detail rail for one selected node.
	 *
	 * **A node, never an edge** — and that is a property of the vocabulary rather than a scope cut.
	 * An edge here is a `ViaEntry`, which carries no id; it has no durable address for `?sel=` and
	 * none for `/api/graph/elements/edge/{id}/trail` either. So there is nothing to select.
	 *
	 * Neighbours come from the graph already on screen, not a second read — the canvas is drawing
	 * those very edges. The excerpt and the history are the only things this panel costs, and both
	 * are read for exactly the one resource the reader asked about.
	 */
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import { summarizeEvent } from '$lib/graph/eventSummary';
	import { atlasNeighbors } from '$lib/graph/neighbors';
	import { docTypeHue } from '$lib/graph/palette';
	import { relativeTime } from '$lib/graph/relativeTime';
	import { trailModel } from '$lib/graph/trail';
	import type { GraphModel, GraphNode } from '$lib/graph/model';
	import { describeArm, whereOf } from '$lib/graph/presentation';
	import type { EventTrail } from '$lib/types/generated/element_trail';
	import { resourceHref, withGraphSeed, withGraphSelection } from '$lib/vault-url';

	interface Props {
		node: GraphNode;
		model: GraphModel;
		/** The body of THIS resource, read on selection. `null` when it has none or the read failed. */
		excerpt: string | null;
		trail: EventTrail | null;
	}
	let { node, model, excerpt, trail }: Props = $props();

	const hue = $derived(docTypeHue(node.doc_type));
	const neighbors = $derived(atlasNeighbors(node.id, model.nodes, model.edges));
	const history = $derived(trail ? trailModel(trail) : []);
	const titles = $derived(new Map(model.nodes.map((n) => [n.id, { title: n.title }])));
	// Which seeds reached this node — `ViaEntry.seed_id` is the reason `via` exists at all: the
	// score is the best path from ANY seed, so without it a multi-seed walk cannot say which.
	const reachedFrom = $derived(
		[
			...new Set(
				model.edges
					.filter((e) => e.source === node.id || e.target === node.id)
					.flatMap((e) => e.seedIds),
			),
		]
			.map((id) => titles.get(id)?.title)
			.filter((t): t is string => !!t && t !== node.title),
	);

	// Closing the rail is ephemeral — REPLACE, so it leaves no history step the Back button has
	// to walk back through. Walking from this node instead is a new question about the graph and
	// PUSHES, so Back returns to where the reader was.
	const close = () => goto(withGraphSelection($page.url, null), { replaceState: true });
	const walkFromHere = () => goto(withGraphSeed($page.url, node.id));
</script>

<aside class="node-rail" style="--hue: {hue};" data-testid="node-rail">
	<header>
		<span class="marker">{node.doc_type}</span>
		<button class="close" onclick={close}>CLOSE ✕</button>
	</header>

	<h2 class="title">{node.title}</h2>

	<section class="actions">
		<button class="walk" onclick={walkFromHere}>Walk from here →</button>
		<a class="view" href={resourceHref(node)} data-testid="view-full-resource">
			View full resource →
		</a>
	</section>

	{#if excerpt}
		<section>
			<div class="label">EXCERPT</div>
			<p class="excerpt">{excerpt}</p>
		</section>
	{/if}

	<section class="meta">
		<div><span class="k">IN</span><span>{node.homeRef ?? 'home not reported'}</span></div>
		<div><span class="k">HOW</span><span>{describeArm(node.arm)}</span></div>
		{#if node.stage}
			<div><span class="k">STAGE</span><span>{node.stage}</span></div>
		{/if}
		{#if node.updated}
			<div><span class="k">UPDATED</span><span>{relativeTime(node.updated)}</span></div>
		{/if}
	</section>

	{#if reachedFrom.length}
		<section>
			<div class="label">REACHED FROM</div>
			<ul class="reached">
				{#each reachedFrom as t (t)}<li>{t}</li>{/each}
			</ul>
		</section>
	{/if}

	{#if neighbors.length}
		<section class="neighbors">
			<div class="label">NEIGHBORS · {neighbors.length}</div>
			{#each neighbors as n (n.other.id + n.label + n.dir)}
				<a class="nb" href={withGraphSelection($page.url, n.other.id)}>
					<span class="dir">{n.dir}</span>
					<span class="rel">{n.label}</span>
					<span class="name" style="color: {docTypeHue(n.other.doc_type)}">{n.other.title}</span>
				</a>
			{/each}
		</section>
	{/if}

	{#if history.length}
		<section class="history">
			<div class="label">HISTORY · {history.length}</div>
			{#each history as row (row.id)}
				<div class="ev">
					<span class="when">{relativeTime(row.occurredAt)}</span>
					<span class="what">{row.kind}</span>
					<span class="who">{row.actorName}</span>
					{#if summarizeEvent(row.rawKind, row.payload, titles)}
						<span class="sum">{summarizeEvent(row.rawKind, row.payload, titles)}</span>
					{/if}
				</div>
			{/each}
		</section>
	{/if}
</aside>

<style>
	.node-rail {
		width: 22rem;
		max-width: 90vw;
		display: flex;
		flex-direction: column;
		gap: 12px;
		padding: 12px 14px 20px;
		overflow: auto;
		background: rgba(20, 23, 29, 0.97);
		border-left: 1px solid rgba(255, 255, 255, 0.1);
		color: #c9d1d9;
		font-size: 13px;
	}
	header {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: 8px;
	}
	.marker,
	.close,
	.label,
	.k {
		font:
			9px/1.6 ui-monospace,
			Menlo,
			monospace;
		letter-spacing: 0.16em;
		text-transform: uppercase;
	}
	.marker {
		color: var(--hue);
	}
	.close {
		background: none;
		border: 0;
		color: #6f7886;
		cursor: pointer;
		padding: 0;
	}
	.title {
		margin: 0;
		font-family: Georgia, serif;
		font-size: 17px;
		line-height: 1.25;
		color: var(--hue);
	}
	section {
		display: grid;
		gap: 6px;
	}
	.label,
	.k {
		color: #6f7886;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
	}
	.walk,
	.view {
		padding: 5px 9px;
		border: 1px solid rgba(255, 255, 255, 0.14);
		border-radius: 6px;
		background: none;
		color: #c9d1d9;
		font-size: 12px;
		text-decoration: none;
		cursor: pointer;
	}
	.excerpt {
		margin: 0;
		color: #9aa3b0;
		font-size: 12.5px;
		line-height: 1.55;
	}
	.meta div {
		display: flex;
		gap: 10px;
		justify-content: space-between;
		align-items: baseline;
	}
	.meta span:last-child {
		color: #9aa3b0;
		font-size: 12px;
		text-align: right;
	}
	.reached {
		margin: 0;
		padding-left: 16px;
		color: #9aa3b0;
		font-size: 12px;
	}
	.nb {
		display: grid;
		grid-template-columns: auto auto 1fr;
		gap: 8px;
		align-items: baseline;
		padding: 4px 0;
		text-decoration: none;
	}
	.dir {
		color: #6f7886;
	}
	.rel {
		font:
			9px/1.6 ui-monospace,
			Menlo,
			monospace;
		letter-spacing: 0.1em;
		color: #79828f;
	}
	.name {
		font-size: 12.5px;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.ev {
		display: grid;
		grid-template-columns: auto 1fr;
		gap: 2px 8px;
		padding: 5px 0;
		border-top: 1px solid rgba(255, 255, 255, 0.05);
		font-size: 11.5px;
		color: #9aa3b0;
	}
	.when {
		color: #6f7886;
		font-variant-numeric: tabular-nums;
	}
	.what {
		color: #c9d1d9;
	}
	.who,
	.sum {
		grid-column: 1 / -1;
		color: #79828f;
	}
</style>
