<script lang="ts">
	import VaultBrowser from '$lib/components/vault/VaultBrowser.svelte';
	import ShapeList from '$lib/components/vault/ShapeList.svelte';
	import RegionState from '$lib/components/RegionState.svelte';
	import { regionStateFor } from '$lib/region';
	import type { PageData } from './$types';

	let { data }: { data: PageData } = $props();
</script>

<svelte:head>
	<title>{data.context} — temper</title>
</svelte:head>

<VaultBrowser
	title={data.context}
	list={data.list}
	contexts={data.contexts}
	fixedContext
	captionPrefix={data.owner}
/>

<!--
	Governance posture for this context, below the list so an untouched page never shifts:
	like the resource page's artifacts region, a context declaring no families renders
	nothing (absence IS "ungoverned" — a placeholder here would be a changed layout for
	nearly every context), and `shapes === null` (the layout's context read never answered,
	or this context is not among the visible rows) renders nothing for the same
	honesty-of-absence reason: nobody resolved this context, so nothing is claimed about
	its governance. A FAILED shapes read, though, names itself — a failure degrading into
	"ungoverned" would be the failed-vs-empty defect again.
-->
{#if data.shapes}
	{#await data.shapes then shapes}
		{#if shapes.length > 0}
			<ShapeList {shapes} />
		{/if}
	{:catch error}
		<div class="shapes-failed">
			<RegionState state={regionStateFor(error)} label="governed families" />
		</div>
	{/await}
{/if}

<style>
	.shapes-failed {
		padding: 0 22px 18px;
	}
</style>
