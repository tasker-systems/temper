<script lang="ts">
	import { onMount } from 'svelte';
	import type { GraphEdge, GraphNode } from '$lib/types/generated/graph';
	import { deriveDisplay } from '$lib/graph/derive';
	import { resourceHref } from '$lib/graph/navigation';
	import { buildNeighborEntries } from '$lib/graph/peek';
	import { nodeColor, SESSION_GLYPH_COLOR } from '$lib/graph/styling';
	import { buildCrumbEntries } from '$lib/graph/trail';

	interface Props {
		node: GraphNode;
		nodes: GraphNode[];
		edges: GraphEdge[];
		owner: string;
		context: string;
		/**
		 * The drill path up to and including the current focused node. Single-
		 * item trails render no breadcrumb; length ≥ 2 shows the crumb bar.
		 */
		trail: string[];
		onClose: () => void;
		onFocus: (id: string) => void;
		/**
		 * Crumb click handler. Receives the *original* trail depth to slice to
		 * (`trail.slice(0, depth + 1)`) — callers shouldn't worry about whether
		 * the breadcrumb was collapsed for rendering.
		 */
		onCrumbClick: (depth: number) => void;
		width?: number;
		topOffset?: number;
	}

	let {
		node,
		nodes,
		edges,
		owner,
		context,
		trail,
		onClose,
		onFocus,
		onCrumbClick,
		width = 420,
		topOffset = 0
	}: Props = $props();

	// Derived view-model for the focused node. Recomputes when `node` changes
	// (which happens on row-click rebind or crumb-click slice).
	const display = $derived(deriveDisplay(node));
	const color = $derived(nodeColor(node.doc_type));
	const neighbors = $derived(buildNeighborEntries(node.id, nodes, edges));
	const href = $derived(resourceHref(owner, context, node));
	const sessionCount = $derived(node.session_count);
	const crumbs = $derived(buildCrumbEntries(trail, nodes));
	const showCrumbs = $derived(trail.length >= 2);

	onMount(() => {
		const handler = (e: KeyboardEvent) => {
			if (e.key === 'Escape') onClose();
		};
		window.addEventListener('keydown', handler);
		return () => window.removeEventListener('keydown', handler);
	});

	function prettifyEdgeType(t: string): string {
		return t.replace(/_/g, ' ').toUpperCase();
	}
</script>

<!--
  Hairline left border in the doctype hue (`${color}55`) and the parchment /
  mono-cap rhythm from the prototype. Scrim is transparent but intercepts
  taps outside the peek so a click-to-close feels responsive.
-->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
	class="pointer-events-auto absolute top-0 bottom-0 left-0 z-[14] bg-transparent"
	style="right: {width}px;"
	onclick={onClose}
></div>

<aside
	class="absolute right-0 bottom-0 z-[15] flex flex-col overflow-hidden backdrop-blur-md"
	style="
		top: {topOffset}px;
		width: {width}px;
		background: rgba(10,10,15,0.92);
		border-left: 1px solid {color}55;
		box-shadow: -12px 0 28px -16px rgba(0,0,0,0.6);
		animation: peekSlide 240ms cubic-bezier(0.2, 0.7, 0.2, 1);
	"
	data-testid="resource-peek"
>
	<!-- Header: doctype marker + close -->
	<div
		class="flex items-center justify-between border-b border-white/5 px-7 pt-5 pb-3"
	>
		<div
			class="font-mono text-[9px] tracking-[0.24em] uppercase"
			style="color: {color}cc;"
		>
			{node.aggregator ? 'AGGREGATOR · ' : 'PARTICIPANT · '}{node.doc_type}
		</div>
		<button
			class="cursor-pointer border-0 bg-transparent p-0 font-mono text-[9px] tracking-[0.22em] text-white/40 transition-colors hover:text-[#e8e4df]"
			onclick={onClose}>CLOSE ✕</button
		>
	</div>

	<!-- Breadcrumb — only when drilled (trail depth ≥ 2). Current crumb (last)
	     is an inert span; earlier crumbs are buttons that slice the trail and
	     recenter the camera via `onCrumbClick(depth)`. -->
	{#if showCrumbs}
		<div
			class="flex flex-wrap items-baseline gap-1.5 px-7 pt-2.5 font-mono text-[8.5px] tracking-[0.16em] text-white/40"
			data-testid="peek-breadcrumb"
		>
			{#each crumbs as entry, idx (idx)}
				{#if entry.kind === 'ellipsis'}
					<span class="opacity-50">…</span>
					<span class="opacity-30">›</span>
				{:else}
					{@const cColor = nodeColor(entry.node.doc_type)}
					{@const cLabel = deriveDisplay(entry.node).label}
					{#if idx > 0}
						<span class="opacity-30">›</span>
					{/if}
					{#if entry.isCurrent}
						<span
							class:italic={entry.node.aggregator}
							style="
								color: {cColor}cc;
								font-family: {entry.node.aggregator
								? '"Source Serif 4", Georgia, serif'
								: '"JetBrains Mono", monospace'};
								font-size: {entry.node.aggregator ? '11px' : '8.5px'};
								letter-spacing: {entry.node.aggregator ? '0' : '0.16em'};
							">{cLabel}</span
						>
					{:else}
						<button
							onclick={() => onCrumbClick(entry.depth)}
							title={`Back to ${entry.node.title}`}
							class="cursor-pointer border-0 bg-transparent p-0 transition-colors"
							class:italic={entry.node.aggregator}
							style="
								color: {cColor}88;
								font-family: {entry.node.aggregator
								? '"Source Serif 4", Georgia, serif'
								: '"JetBrains Mono", monospace'};
								font-size: {entry.node.aggregator ? '11px' : '8.5px'};
								letter-spacing: {entry.node.aggregator ? '0' : '0.16em'};
							"
							onmouseenter={(e) => (e.currentTarget.style.color = cColor)}
							onmouseleave={(e) => (e.currentTarget.style.color = `${cColor}88`)}
							>{cLabel}</button
						>
					{/if}
				{/if}
			{/each}
		</div>
	{/if}

	<!-- Title -->
	<div class="px-7 pt-4 pb-3">
		<h2
			class="m-0 font-serif text-[28px] leading-[1.15] font-normal tracking-[-0.005em]"
			class:italic={node.aggregator}
			style="color: {color};"
		>
			{display.fullTitle}
		</h2>
		{#if sessionCount > 0}
			<div
				class="mt-2.5 font-mono text-[9px] tracking-[0.2em]"
				style="color: {SESSION_GLYPH_COLOR};"
			>
				⌊{sessionCount}⌋ SESSION{sessionCount === 1 ? '' : 'S'} · ANNOTATION
			</div>
		{/if}
	</div>

	<!-- Scrollable body -->
	<div class="flex-1 overflow-y-auto px-7 pt-2 pb-7">
		{#if neighbors.length > 0}
			<div class="mb-3 flex items-baseline justify-between">
				<div class="font-mono text-[8.5px] tracking-[0.22em] text-white/35">
					{node.aggregator ? 'MEMBERS' : 'NEIGHBORS'} · {neighbors.length}
				</div>
			</div>

			<div class="mb-6">
				{#each neighbors as entry (entry.other.id + entry.type + entry.dir)}
					{@const nColor = nodeColor(entry.other.doc_type)}
					{@const nDisplay = deriveDisplay(entry.other)}
					<button
						class="grid w-full cursor-pointer grid-cols-[18px_72px_1fr] items-baseline gap-2.5 border-0 border-b border-white/5 bg-transparent px-0 py-2 text-left transition-colors hover:bg-white/[0.02]"
						onclick={() => onFocus(entry.other.id)}
					>
						<span class="font-mono text-xs text-white/30">{entry.dir}</span>
						<span
							class="font-mono text-[8px] tracking-[0.2em] text-white/40"
						>
							{prettifyEdgeType(entry.type)}
						</span>
						<span
							class="font-serif text-[13px]"
							class:italic={entry.other.aggregator}
							style="color: {nColor};"
						>
							{nDisplay.fullTitle}
						</span>
					</button>
				{/each}
			</div>
		{/if}

		<!-- Metadata rows (doctype / slug / edge counts) -->
		<div
			class="mb-5 grid grid-cols-[90px_1fr] gap-x-3.5 gap-y-2 border-t border-white/5 pt-4 font-mono text-[10px] leading-[1.5]"
		>
			<div class="tracking-[0.2em] text-white/35">DOCTYPE</div>
			<div class="text-[#e8e4df]">{node.doc_type}</div>
			<div class="tracking-[0.2em] text-white/35">SLUG</div>
			<div class="font-mono text-[#e8e4df]">{node.slug}</div>
			<div class="tracking-[0.2em] text-white/35">EDGES</div>
			<div class="text-[#e8e4df]">{node.edge_count} touching</div>
			{#if display.dateStrip}
				<div class="tracking-[0.2em] text-white/35">DATE</div>
				<div class="text-[#e8e4df]">{display.dateStrip}</div>
			{/if}
		</div>

		<!-- First-paragraph body preview. Hidden for resources with no chunk
		     content yet (e.g., frontmatter-only records). -->
		{#if node.excerpt}
			<div class="mb-2.5 font-mono text-[8.5px] tracking-[0.22em] text-white/35">
				EXCERPT
			</div>
			<p
				class="mb-[18px] font-serif text-[14px] leading-[1.6]"
				style="color: rgba(232,228,223,0.88); text-wrap: pretty;"
				data-testid="peek-excerpt"
			>
				{node.excerpt}
			</p>
		{/if}
	</div>

	<!-- Footer: ESC hint + open-resource link -->
	<div
		class="flex items-center justify-between border-t border-white/5 px-7 py-3.5 font-mono text-[9px] tracking-[0.2em]"
	>
		<span class="text-white/30">ESC · CLOSE</span>
		<a
			class="border-b pb-px no-underline"
			style="color: {color}; border-bottom-color: {color}55;"
			href={href}>OPEN RESOURCE →</a
		>
	</div>
</aside>

<style>
	@keyframes peekSlide {
		from {
			transform: translateX(32px);
			opacity: 0;
		}
		to {
			transform: translateX(0);
			opacity: 1;
		}
	}
</style>
