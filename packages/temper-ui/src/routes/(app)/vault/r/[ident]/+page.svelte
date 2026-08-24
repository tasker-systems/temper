<script lang="ts">
	import type { ActionData, PageData } from './$types';
	import MarkdownRenderer from '$lib/components/MarkdownRenderer.svelte';
	import HomeChip from '$lib/components/vault/HomeChip.svelte';
	import PropertySet from '$lib/components/vault/PropertySet.svelte';
	import EventHistory from '$lib/components/vault/EventHistory.svelte';
	import EdgeList from '$lib/components/vault/EdgeList.svelte';
	import RegionState from '$lib/components/RegionState.svelte';
	import { mergeProperties } from '$lib/properties';
	import { docTypeHue } from '$lib/graph/palette';
	import { regionStateFor } from '$lib/region';

	let { data, form }: { data: PageData; form: ActionData } = $props();

	// One --hue on the root tints masthead, rules, chips, toggles and rail.
	// An unknown doc type falls back to FALLBACK_HUE — no branching needed.
	let hue = $derived(docTypeHue(data.resource.doc_type_name));

	// `stateVocabulary` is `null` both when the reader may not change this resource and when
	// the read that says which states its kind carries did not answer — `mergeProperties` offers
	// nothing for either, which is the honest response to both.
	let rows = $derived(
		mergeProperties(
			data.resource.managed_meta as Record<string, unknown> | null | undefined,
			data.resource.open_meta as Record<string, unknown> | null | undefined,
			data.resource.doc_type_name,
			{ states: data.stateVocabulary, descriptions: data.mayChange }
		)
	);

	// Only a degradation when the reader could otherwise have been offered something. A reader
	// without change authority was never going to see a control, so there is nothing to explain.
	let vocabularyUnread = $derived(data.mayChange && data.stateVocabulary === null);

	// A `fail()` from the action arrives with `{ field, message }`; a success arrives with
	// `{ field }` alone and needs no rendering — `load` has re-run and the table already shows
	// the state that now obtains.
	let refusal = $derived(
		form && typeof form.message === 'string' ? { field: form.field, message: form.message } : null
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

		<PropertySet {rows} {vocabularyUnread} {refusal} mayDescribe={data.mayChange} />

		<!--
			Everything above this point is the scaffold and renders from `data.resource`, which the
			load awaited. Nothing below it can delay any of it — that is C1, and it is structural
			rather than timed: hand the page a promise that never settles and the frame, the title,
			the chip and the properties are all still there.

			`document`, not `excerpt`: `data.content` is the whole markdown body of this resource, so
			"Excerpt unavailable" would be a false statement about what failed. (`excerpt` is the
			graph rail's word, for the thing that really is one.)

			The body is the one region with no component of its own to hold the verdict, so the
			emptiness test lives here. `MarkdownRenderer` renders `''` for a falsy input, which is a
			silent region — exactly what the vocabulary exists to prevent.
		-->
		<div class="body">
			{#await data.content}
				<RegionState state="arriving" label="document" />
			{:then markdown}
				{#if markdown.trim() === ''}
					<RegionState state="empty" label="document" />
				{:else}
					<MarkdownRenderer {markdown} />
				{/if}
			{:catch error}
				<RegionState state={regionStateFor(error)} label="document" />
			{/await}
		</div>
	</div>

	<!--
		The rail's two regions arrive independently of the body and of each other. Neither `{:then}`
		branch tests for emptiness: each component owns that verdict, because each derives it from
		the value in a way the page cannot see (`EventHistory` filters through `trailModel`). Two
		predicates in two places are two predicates that can disagree.

		`NodeRail.svelte:84-86` states the rule the other two branches used to break here — "The
		label sits OUTSIDE the await, and that is the rule rather than a layout preference" — and a
		region that vanishes while it is arriving cannot tell the reader that it is arriving. Both
		branches rendered a bare `RegionState`, so the heading was missing for exactly the two
		states that most need naming.

		`railHeading` is the settled components' heading minus its count, which is the one part of
		it a read that has not answered cannot know. It is a snippet rather than four literals
		because four spellings of one heading are four spellings that can drift.
	-->
	{#snippet railHeading(text: string)}
		<div class="label">{text}</div>
	{/snippet}

	<aside class="rail">
		{#await data.trail}
			<div class="rail-region">
				{@render railHeading('History')}
				<RegionState state="arriving" label="history" />
			</div>
		{:then trail}
			<EventHistory {trail} />
		{:catch error}
			<div class="rail-region">
				{@render railHeading('History')}
				<RegionState state={regionStateFor(error)} label="history" />
			</div>
		{/await}

		{#await data.edges}
			<div class="rail-region">
				{@render railHeading('Connections')}
				<RegionState state="arriving" label="connections" />
			</div>
		{:then edges}
			<EdgeList {edges} />
		{:catch error}
			<div class="rail-region">
				{@render railHeading('Connections')}
				<RegionState state={regionStateFor(error)} label="connections" />
			</div>
		{/await}
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
	/* The rail's sections carry their own padding; a bare region needs the same gutter. */
	.rail-region {
		padding: 12px 14px;
	}
	/* Same treatment as `EventHistory`'s and `EdgeList`'s own heading, so a region's label does not
	   change appearance depending on whether its read has answered. */
	.rail-region .label {
		font-family: var(--font-mono);
		font-size: 9px;
		letter-spacing: var(--track-label);
		text-transform: uppercase;
		color: var(--color-quiet-dim);
		margin-bottom: 6px;
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
