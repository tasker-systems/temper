<script lang="ts">
	import RuleHeading from './RuleHeading.svelte';
	import type { ResourceRow } from '$lib/types';

	interface Props {
		resource: ResourceRow;
	}

	let { resource }: Props = $props();

	let dateStr = $derived(
		new Date(resource.updated).toLocaleDateString('en-US', {
			month: 'short',
			day: 'numeric',
			year: 'numeric'
		})
	);

	let caption = $derived(
		`${resource.doc_type_name} · ${resource.context_name} · ${dateStr}`
	);

	interface Chip {
		label: string;
		value: string;
	}

	let chips = $derived(
		(
			[
				resource.seq != null ? { label: 'seq', value: String(resource.seq) } : null,
				resource.stage ? { label: 'stage', value: resource.stage } : null,
				resource.mode ? { label: 'mode', value: resource.mode } : null,
				resource.effort ? { label: 'effort', value: resource.effort } : null,
				{ label: 'owner', value: resource.owner_handle }
			] as (Chip | null)[]
		).filter((c): c is Chip => c !== null)
	);
</script>

<div class="space-y-3">
	<RuleHeading title={resource.title} {caption} />

	{#if chips.length > 0}
		<div class="flex flex-wrap gap-2">
			{#each chips as chip}
				<span
					class="inline-flex items-center gap-1 rounded bg-zinc-800/60 px-2 py-0.5 text-xs font-mono text-zinc-400"
				>
					<span class="text-zinc-600">{chip.label}:</span>
					{chip.value}
				</span>
			{/each}
		</div>
	{/if}
</div>
