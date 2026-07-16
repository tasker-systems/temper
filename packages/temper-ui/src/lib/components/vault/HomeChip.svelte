<script lang="ts">
	import type { ResourceRow } from '$lib/types';
	import { contextHref } from '$lib/vault-url';

	let { row }: { row: ResourceRow } = $props();

	// A resource is homed by exactly one anchor: a context or a cogmap
	// (kb_resource_homes.anchor_table). Cogmap-homed rows carry null context_*.
	let isCogmap = $derived(row.cogmap_id !== null);
	let label = $derived(
		isCogmap ? (row.cogmap_name ?? 'cogmap') : (row.context_name ?? row.context_slug ?? 'context')
	);
	let href = $derived(
		!isCogmap && row.context_owner_ref && row.context_slug
			? contextHref(row.context_owner_ref, row.context_slug)
			: null
	);
</script>

{#if href}
	<a class="chip" {href}>◆ CONTEXT · {label}</a>
{:else}
	<span class="chip">{isCogmap ? '◈ COGMAP' : '◆ CONTEXT'} · {label}</span>
{/if}

<style>
	.chip {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		font-family: var(--font-mono);
		font-size: 9px;
		letter-spacing: 0.08em;
		padding: 2px 7px;
		border-radius: 2px;
		text-decoration: none;
		border: 1px solid color-mix(in srgb, var(--hue) 40%, transparent);
		color: color-mix(in srgb, var(--hue) 72%, white);
		background: color-mix(in srgb, var(--hue) 8%, transparent);
	}
	a.chip:hover {
		border-color: color-mix(in srgb, var(--hue) 70%, transparent);
	}
</style>
