<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import { docTypeChips, parseFilters, toggleDocType } from '$lib/vault-filters';

	interface Props {
		facets: Record<string, number> | null;
	}

	let { facets }: Props = $props();

	let activeDocTypes = $derived(parseFilters($page.url).docTypes);

	// `docTypeChips` — not the histogram directly — because the door emits no zero-count keys,
	// so a selected doc-type that admits nothing would render no chip at all, and the chip is
	// the only control that can deselect it.
	let sorted = $derived(docTypeChips(facets, activeDocTypes));

	function toggle(name: string) {
		goto(toggleDocType($page.url, name), { replaceState: true });
	}
</script>

{#if sorted.length > 0}
	<div class="flex flex-wrap gap-1.5">
		{#each sorted as { name, count, active } (name)}
			<button
				class="inline-flex items-center gap-1.5 rounded px-2.5 py-1 text-xs font-mono tracking-wide transition-colors
					{active
					? 'bg-tp-accent-wash text-tp-accent border border-tp-accent-line-soft'
					: 'bg-tp-surface-raised text-tp-text-muted border border-tp-rule hover:text-tp-text hover:border-tp-rule-strong'}"
				onclick={() => toggle(name)}
				aria-pressed={active}
			>
				{name}
				<span class="text-[10px] {active ? 'text-tp-accent-muted' : 'text-tp-text-subtle'}">{count}</span>
			</button>
		{/each}
	</div>
{/if}
