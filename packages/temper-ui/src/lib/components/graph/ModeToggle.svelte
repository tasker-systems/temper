<script lang="ts">
	/**
	 * The top-left `VIEW` toggle — `structural` / `meta-doc`. Mirrors the
	 * prototype ui_kits/app/KnowledgeGraph.jsx:293-320 so the production
	 * graph view stays shape-compatible with the settled visual language.
	 *
	 * Wired to a no-op selector today: Jaccard meta-doc mode is PR 6 in
	 * kg-handoff.md and deferred per kg-handoff-next.md. When the user
	 * clicks `meta-doc` we still flip the active state so the "Emergent
	 * view — not implemented" stub surfaces, but nothing downstream reacts
	 * until the emergent-edge computation lands.
	 */
	export type GraphMode = 'structural' | 'meta-doc';

	interface Props {
		mode: GraphMode;
		onModeChange: (next: GraphMode) => void;
	}

	let { mode, onModeChange }: Props = $props();

	const WORDS: { value: GraphMode; label: string }[] = [
		{ value: 'structural', label: 'structural' },
		{ value: 'meta-doc', label: 'meta-doc' }
	];
</script>

<!-- Top-left chrome. pointer-events-none on the wrapper so the rest of
     the graph remains interactive; the inner cluster re-enables pointer
     events for the word toggles. -->
<div
	class="pointer-events-none absolute top-4 left-5 z-[12] select-none"
	data-testid="graph-mode-toggle"
>
	<div class="pointer-events-auto">
		<div
			class="mb-1.5 font-mono text-[9px] tracking-[0.22em] text-white/40 uppercase"
		>
			VIEW
		</div>
		<div class="flex items-baseline gap-3.5">
			{#each WORDS as word (word.value)}
				{@const active = mode === word.value}
				<button
					type="button"
					class="cursor-pointer border-0 bg-transparent p-0 transition-colors"
					class:text-[13px]={active}
					class:font-serif={active}
					class:text-[#e8e4df]={active}
					class:tracking-normal={active}
					class:text-[10px]={!active}
					class:font-mono={!active}
					class:tracking-[0.18em]={!active}
					class:uppercase={!active}
					class:text-white={false}
					style={active ? '' : 'color: rgba(255,255,255,0.35);'}
					onclick={() => onModeChange(word.value)}
				>
					{word.label}
				</button>
			{/each}
		</div>
		{#if mode === 'meta-doc'}
			<div
				class="mt-2 max-w-[240px] font-serif text-[11px] text-white/50 italic"
				data-testid="graph-mode-meta-doc-stub"
			>
				Emergent view — not implemented in this prototype yet.
			</div>
		{/if}
	</div>
</div>
