<script lang="ts">
	import { nodeColor, SESSION_GLYPH_COLOR } from '$lib/graph/styling';

	/**
	 * Standalone legend panel — mirrors the prototype
	 * ui_kits/app/KnowledgeGraph.jsx:322-354. Doctype color swatches plus
	 * the `⌊N⌋ SESSIONS · ANNOTATION, NOT EDGE` marker below.
	 *
	 * Positioned at top-right. The docked `ResourcePeek` passes
	 * `topOffset={168}` so it never eclipses this panel.
	 */
	const ROWS: { doctype: string; label: string }[] = [
		{ doctype: 'research', label: 'RESEARCH' },
		{ doctype: 'task', label: 'TASK' },
		{ doctype: 'goal', label: 'GOAL · italic' },
		{ doctype: 'concept', label: 'CONCEPT · italic' }
	];
</script>

<aside
	class="pointer-events-auto absolute top-4 right-4 z-[16] px-4 pt-3.5 pb-3 text-right font-mono text-[8.5px] leading-[1.9] tracking-[0.18em] text-white/45"
	style="
		background: rgba(10,10,15,0.88);
		backdrop-filter: blur(6px);
		border: 1px solid rgba(255,255,255,0.06);
	"
	data-testid="graph-legend"
>
	{#each ROWS as row (row.doctype)}
		<div>
			<span
				class="mr-1.5 inline-block h-[5px] w-5 align-middle"
				style="background: {nodeColor(row.doctype)};"
			></span>{row.label}
		</div>
	{/each}
	<div
		class="mt-2.5 border-t border-white/5 pt-2 text-[7.5px] tracking-[0.14em]"
		style="color: {SESSION_GLYPH_COLOR};"
	>
		⌊N⌋ SESSIONS · ANNOTATION, NOT EDGE
	</div>
	<div class="mt-2.5 text-[7.5px] text-white/30">— CLICK ANY NODE TO PEEK —</div>
</aside>
