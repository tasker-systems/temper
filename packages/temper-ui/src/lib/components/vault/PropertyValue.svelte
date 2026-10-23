<script lang="ts">
	import { classifyValue } from '$lib/propertyValue';
	import Self from './PropertyValue.svelte';

	let { value }: { value: unknown } = $props();

	let v = $derived(classifyValue(value));
	let open = $state(false);
</script>

{#if v.kind === 'scalar'}
	<span class="scalar">{v.text}</span>
{:else}
	<button class="toggle" aria-expanded={open} onclick={() => (open = !open)}
		>{open ? '⌄' : '›'} {v.summary}</button
	>
	{#if open}
		<div class="sub">
			{#if v.kind === 'object'}
				{#each v.entries as [k, child] (k)}
					<div class="row">
						<span class="k">{k}</span>
						<span class="v"><Self value={child} /></span>
					</div>
				{/each}
			{:else}
				{#each v.items as item, i (i)}
					<div class="item"><span class="i">{i}</span><Self value={item} /></div>
				{/each}
			{/if}
		</div>
	{/if}
{/if}

<style>
	.scalar {
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--tp-text-muted);
		word-break: break-word;
	}
	.toggle {
		font-family: var(--font-mono);
		font-size: 11px;
		background: none;
		border: 0;
		padding: 0;
		cursor: pointer;
		color: var(--hue);
	}
	.toggle:hover {
		color: var(--hue);
	}
	.sub {
		border-left: 1px solid var(--tp-rule);
		margin: 5px 0 5px 3px;
		padding-left: 11px;
	}
	.row {
		display: grid;
		grid-template-columns: 116px 1fr;
		gap: 8px;
		padding: 2px 0;
		align-items: start;
	}
	.k {
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--tp-text-subtle);
	}
	.v {
		min-width: 0;
	}
	.item {
		font-family: var(--font-mono);
		font-size: 10.5px;
		color: var(--tp-text-muted);
		padding: 2px 0;
	}
	.item .i {
		color: var(--tp-text-subtle);
		margin-right: 7px;
	}
</style>
