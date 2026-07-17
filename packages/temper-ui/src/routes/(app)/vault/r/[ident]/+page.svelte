<script lang="ts">
	import type { PageData } from './$types';
	import MarkdownRenderer from '$lib/components/MarkdownRenderer.svelte';
	import HomeChip from '$lib/components/vault/HomeChip.svelte';
	import PropertySet from '$lib/components/vault/PropertySet.svelte';
	import EventHistory from '$lib/components/vault/EventHistory.svelte';
	import EdgeList from '$lib/components/vault/EdgeList.svelte';
	import { mergeProperties } from '$lib/properties';
	import { docTypeHue } from '$lib/graph/atlas/palette';

	let { data }: { data: PageData } = $props();

	// One --hue on the root tints masthead, rules, chips, toggles and rail.
	// An unknown doc type falls back to FALLBACK_HUE — no branching needed.
	let hue = $derived(docTypeHue(data.resource.doc_type_name));

	let rows = $derived(
		mergeProperties(
			data.resource.managed_meta as Record<string, unknown> | null | undefined,
			data.resource.open_meta as Record<string, unknown> | null | undefined,
			data.resource.doc_type_name
		)
	);
</script>

<svelte:head>
	<title>{data.resource.title} — temper</title>
</svelte:head>

<div class="rv" style="--hue: {hue}">
	<div class="main">
		<div class="masthead">
			<div class="eyebrow">{data.resource.doc_type_name}</div>
			<h1 class="title">{data.resource.title}</h1>
			<HomeChip row={data.resource} />
		</div>

		<PropertySet {rows} />

		<div class="body">
			<MarkdownRenderer markdown={data.content} />
		</div>
	</div>

	<aside class="rail">
		<EventHistory trail={data.trail} />
		<EdgeList edges={data.edges} />
	</aside>
</div>

<style>
	.rv {
		display: grid;
		grid-template-columns: 1fr 260px;
		min-height: 100%;
	}
	.main {
		min-width: 0;
	}
	.masthead {
		padding: 18px 22px;
		border-bottom: 1px solid var(--color-quiet-rule);
	}
	.eyebrow {
		font-family: var(--font-mono);
		font-size: 9px;
		letter-spacing: var(--track-label);
		text-transform: uppercase;
		color: color-mix(in srgb, var(--hue) 80%, white);
	}
	.title {
		font-family: var(--font-serif);
		font-weight: 400;
		font-size: 25px;
		line-height: 1.25;
		letter-spacing: -0.01em;
		color: var(--hue);
		margin: 8px 0 11px;
	}
	.body {
		padding: 18px 22px 24px;
	}
	.rail {
		background: var(--color-quiet-card);
		border-left: 1px solid color-mix(in srgb, var(--hue) 22%, transparent);
	}

	@media (max-width: 900px) {
		.rv {
			grid-template-columns: 1fr;
		}
		.rail {
			border-left: 0;
			border-top: 1px solid color-mix(in srgb, var(--hue) 22%, transparent);
		}
	}
</style>
