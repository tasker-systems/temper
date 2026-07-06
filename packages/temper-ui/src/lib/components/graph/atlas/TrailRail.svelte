<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import { buildDrillNodeUrl, clearSelectionUrl, type SelectedElement } from '$lib/graph/atlas/nav';
	import type { AtlasSubgraph } from '$lib/types/generated/graph_atlas';
	import type { EventTrail } from '$lib/types/generated/element_trail';
	import type { ResourceRow } from '$lib/types/generated/resource';
	import { atlasNeighbors } from '$lib/graph/atlas/neighbors';
	import { trailModel } from '$lib/graph/atlas/trail';
	import { docTypeHue } from '$lib/graph/atlas/palette';
	import { relativeTime } from '$lib/graph/atlas/relativeTime';
	import { summarizeEvent } from '$lib/graph/atlas/eventSummary';
	import { flattenPayload } from '$lib/graph/atlas/payloadRows';

	interface Props {
		selection: SelectedElement;
		subgraph: AtlasSubgraph | null;
		trail: EventTrail | null;
		resourceRow: ResourceRow | null;
	}
	let { selection, subgraph, trail, resourceRow }: Props = $props();

	const node = $derived(
		selection.kind === 'node' && subgraph ? (subgraph.nodes.find((n) => n.id === selection.id) ?? null) : null
	);
	const edge = $derived(
		selection.kind === 'edge' && subgraph ? (subgraph.edges.find((e) => e.id === selection.id) ?? null) : null
	);
	// A resource leaf with no mapped neighbors has no subgraph node — fall back to the
	// resourceRow so title / doc-type / meta still render off the loaded row + trail.
	const isNode = $derived(selection.kind === 'node');
	const nodeTitle = $derived(node?.title ?? resourceRow?.title ?? null);
	const nodeDocType = $derived(node?.doc_type ?? resourceRow?.doc_type_name ?? null);
	const hue = $derived(isNode ? docTypeHue(nodeDocType) : edge ? '#c9b183' : '#8a929e');
	const neighbors = $derived(node && subgraph ? atlasNeighbors(node.id, subgraph.nodes, subgraph.edges) : []);
	const rows = $derived(trail ? trailModel(trail) : []);
	// Read-first excerpt: server-derived first-paragraph preview (R4's compute_excerpt).
	// null for edges and for nodes without a body (e.g. a leaf with no mapped content).
	const nodeExcerpt = $derived(node?.excerpt ?? null);
	// Title lookup for summarizeEvent's relationship-target resolution — built from
	// the same subgraph nodes already loaded for the neighbors section.
	const nodesById = $derived(new Map((subgraph?.nodes ?? []).map((n) => [n.id, { title: n.title }])));
	// Which history row (by event id, the same stable key as `rows`) has its
	// payload expanded. Exactly one row open at a time; null = all collapsed.
	let openEvent = $state<string | null>(null);

	// Closing the panel is ephemeral (clears ?sel) — REPLACE so it doesn't leave a
	// history step. Refocusing to a neighbor is a drill — PUSH so Back returns to
	// the prior node (see nav.ts).
	function close() {
		goto(clearSelectionUrl($page.url), { replaceState: true });
	}
	function refocus(id: string) {
		goto(buildDrillNodeUrl($page.url, id));
	}
</script>

{#if selection.kind !== 'none'}
	<aside class="trail-rail" style="--hue: {hue};" data-testid="trail-rail">
		<header>
			<span class="marker">{edge ? 'EDGE' : 'NODE'} · {nodeDocType ?? edge?.edge_kind ?? ''}</span>
			<button class="close" onclick={close}>CLOSE ✕</button>
		</header>
		<h2 class="title">{nodeTitle ?? (edge ? `${edge.edge_kind}` : '')}</h2>

		{#if isNode && nodeExcerpt}
			<section class="excerpt-section">
				<div class="label">EXCERPT</div>
				<p class="excerpt">{nodeExcerpt}</p>
			</section>
		{/if}

		{#if node && neighbors.length}
			<section class="neighbors">
				<div class="label">NEIGHBORS · {neighbors.length}</div>
				{#each neighbors as n (n.other.id + n.label + n.dir)}
					<button class="nb" onclick={() => refocus(n.other.id)}>
						<span class="dir">{n.dir}</span>
						<span class="rel">{n.label}</span>
						<span class="name" style="color: {docTypeHue(n.other.doc_type ?? null)}">{n.other.title}</span>
					</button>
				{/each}
			</section>
		{/if}

		{#if isNode && resourceRow}
			<section class="meta">
				<div><span class="k">CONTEXT</span><span>{resourceRow.context_slug ?? '—'}</span></div>
				{#if resourceRow.cogmap_name}
					<div><span class="k">COGMAP</span><span>{resourceRow.cogmap_name}</span></div>
				{/if}
				{#if resourceRow.stage}
					<div><span class="k">STAGE</span><span>{resourceRow.stage}</span></div>
				{/if}
			</section>
		{/if}

		{#if edge}
			<section class="meta">
				<div><span class="k">POLARITY</span><span>{edge.polarity}</span></div>
				<div><span class="k">WEIGHT</span><span>{edge.weight}</span></div>
			</section>
		{/if}

		<section class="history">
			<div class="label">HISTORY · {rows.length}</div>
			{#if rows.length === 0}
				<p class="empty">No recorded history.</p>
			{:else}
				{#each rows.slice(0, 50) as row (row.id)}
					{@const summary = summarizeEvent(row.rawKind, row.payload, nodesById)}
					<div class="event">
						<button
							class="event-head"
							onclick={() => (openEvent = openEvent === row.id ? null : row.id)}
							aria-expanded={openEvent === row.id}
						>
							<span class="ekind">{row.kind}</span>
							<span class="chev">{openEvent === row.id ? '⌄' : '›'}</span>
						</button>
						{#if summary}<div class="ev-summary">{summary}</div>{/if}
						<div class="ev-meta">
							by <b>{row.actorName}</b> · {relativeTime(row.occurredAt)}{#if row.confidence}
								· <span class="conf">{row.confidence}</span>{/if}
						</div>
						{#if openEvent === row.id}
							<dl class="ev-payload">
								{#each flattenPayload(row.payload) as pr (pr.key)}
									<div><dt>{pr.key}</dt><dd>{pr.value}</dd></div>
								{/each}
							</dl>
						{/if}
					</div>
				{/each}
			{/if}
		</section>
	</aside>
{/if}

<style>
	.trail-rail {
		width: 340px;
		height: 100%;
		overflow-y: auto;
		background: rgba(20, 23, 29, 0.96);
		border-left: 1px solid color-mix(in srgb, var(--hue) 33%, transparent);
		backdrop-filter: blur(8px);
		color: #c9d1d9;
	}
	header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		padding: 14px 18px 8px;
	}
	.marker {
		font-family: monospace;
		font-size: 9px;
		letter-spacing: 0.2em;
		color: color-mix(in srgb, var(--hue) 80%, white);
	}
	.close {
		background: none;
		border: 0;
		color: #6a727e;
		font: 9px monospace;
		letter-spacing: 0.2em;
		cursor: pointer;
	}
	.title {
		margin: 0;
		padding: 0 18px 10px;
		font-family: Georgia, serif;
		font-size: 22px;
		color: var(--hue);
	}
	section {
		padding: 8px 18px;
		border-top: 1px solid rgba(255, 255, 255, 0.06);
	}
	.label {
		font-family: monospace;
		font-size: 8.5px;
		letter-spacing: 0.2em;
		color: #6a727e;
		margin-bottom: 6px;
	}
	.nb {
		display: grid;
		grid-template-columns: 14px 70px 1fr;
		gap: 8px;
		width: 100%;
		text-align: left;
		background: none;
		border: 0;
		padding: 5px 0;
		cursor: pointer;
		font-size: 13px;
	}
	.nb .dir {
		color: #4a5261;
	}
	.nb .rel {
		font: 8px monospace;
		letter-spacing: 0.15em;
		color: #6a727e;
	}
	.meta > div {
		display: grid;
		grid-template-columns: 80px 1fr;
		gap: 10px;
		font: 10px monospace;
		padding: 3px 0;
	}
	.meta .k {
		color: #6a727e;
		letter-spacing: 0.15em;
	}
	.event {
		display: flex;
		flex-direction: column;
		gap: 3px;
		padding: 6px 0;
		font-size: 12px;
	}
	.event:not(:first-child) {
		border-top: 1px solid rgba(255, 255, 255, 0.06);
	}
	.ekind {
		color: var(--hue);
		font-weight: 600;
	}
	.conf {
		font-size: 9px;
		color: #8fd8a8;
	}
	.empty {
		color: #6a727e;
		font-size: 12px;
	}

	/* ─── Excerpt — read-first body preview, directly under the title ─── */
	.excerpt-section {
		padding-top: 0;
	}
	.excerpt {
		margin: 0;
		color: #aeb7c4;
		font-size: 12.5px;
		line-height: 1.5;
		border-left: 2px solid color-mix(in srgb, var(--hue) 55%, transparent);
		padding-left: 10px;
	}

	/* ─── History rows — collapsed summary + actor/time, expand for payload ─── */
	.event-head {
		display: flex;
		align-items: baseline;
		gap: 8px;
		width: 100%;
		background: none;
		border: 0;
		padding: 0;
		margin: 0;
		font: inherit;
		text-align: left;
		color: inherit;
		cursor: pointer;
	}
	.chev {
		margin-left: auto;
		color: #5a6270;
		font-size: 11px;
	}
	.ev-summary {
		color: #aeb7c4;
		font-size: 11.5px;
	}
	.ev-meta {
		color: #8a929e;
		font-size: 10.5px;
	}
	.ev-meta b {
		color: #b7c0cd;
		font-weight: 600;
	}
	.ev-payload {
		margin: 4px 0 0;
		padding-top: 6px;
		border-top: 1px dashed rgba(255, 255, 255, 0.1);
	}
	.ev-payload > div {
		display: grid;
		grid-template-columns: 90px 1fr;
		gap: 4px 10px;
		padding: 2px 0;
	}
	.ev-payload dt,
	.ev-payload dd {
		margin: 0;
		font-family: monospace;
		font-size: 10.5px;
	}
	.ev-payload dt {
		color: #6a727e;
		letter-spacing: 0.08em;
	}
	.ev-payload dd {
		color: #c3ccd8;
		word-break: break-word;
	}
</style>
