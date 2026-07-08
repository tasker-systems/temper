<script lang="ts">
	// Non-spatial accessible mirror of the Beat D composition force-graph (the
	// region → resources drill). The field is drawn inside `<svg role="img">`, opaque
	// to screen readers, so this `<nav>` is the accessible equivalent: the two axes as
	// two link lists — Ideas (cogmap facets) and Sources (the context-homed work they
	// were derived_from) — each item a drill link + doc-type + degree. Visually hidden,
	// revealed on keyboard focus so it is not a dead trap (same pattern as HomeA11yList).
	import { page } from '$app/stores';
	import type { AtlasNode, AtlasSubgraph } from '$lib/types/generated/graph_atlas';
	import { buildDrillNodeUrl } from '$lib/graph/atlas/nav';
	import { groupByAxis } from '$lib/graph/atlas/marks';

	interface Props {
		subgraph: AtlasSubgraph;
	}
	let { subgraph }: Props = $props();

	const groups = $derived(groupByAxis(subgraph.nodes));
	const href = (id: string) => buildDrillNodeUrl($page.url, id);
	const meta = (n: AtlasNode) => `${n.doc_type ?? 'untyped'} · ${n.degree} links`;
</script>

<!-- Label is tier-neutral: this mirror serves both the tier-1 region composition
     and the tier-2 node neighborhood (both group into ideas + sources). -->
<nav class="composition-a11y" aria-label="Graph nodes — ideas and the work they came from">
	<h2>Ideas</h2>
	{#if groups.ideas.length}
		<ul>
			{#each groups.ideas as n (n.id)}
				<li><a href={href(n.id)}>{n.title} — {meta(n)}</a></li>
			{/each}
		</ul>
	{:else}
		<p>No ideas in this view yet.</p>
	{/if}

	<h2>Sources</h2>
	{#if groups.sources.length}
		<ul>
			{#each groups.sources as n (n.id)}
				<li><a href={href(n.id)}>{n.title} — {meta(n)}</a></li>
			{/each}
		</ul>
	{:else}
		<p>No linked work yet.</p>
	{/if}
</nav>

<style>
	/* Visually hidden until focused (standard sr-only + reveal-on-focus). */
	.composition-a11y {
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
	.composition-a11y:focus-within {
		position: absolute;
		top: 8px;
		left: 8px;
		z-index: 5;
		width: auto;
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
	.composition-a11y h2 {
		font-size: 12px;
		letter-spacing: 0.04em;
		margin: 6px 0 4px;
	}
	.composition-a11y a {
		color: #9fc4d6;
	}
</style>
