<script lang="ts">
	import type { GraphEdgeRow } from '$lib/types/generated/graph';

	let { edges }: { edges: GraphEdgeRow[] } = $props();
</script>

{#if edges.length > 0}
	<section>
		<div class="label">Edges · {edges.length}</div>
		{#each edges as edge (edge.edge_id)}
			<div class="edge">
				<span class="rel">
					{edge.direction === 'out' ? '' : '← '}{edge.label || edge.edge_kind}{edge.direction ===
					'out'
						? ' →'
						: ''}
				</span>
				<a class="peer" href="/vault/r/{edge.peer_resource_id}">{edge.peer_title}</a>
				<span class="w">
					· {edge.weight.toFixed(1)}{#if edge.polarity !== 'forward'}
						· {edge.polarity}{/if}
				</span>
			</div>
		{/each}
	</section>
{/if}

<style>
	section {
		padding: 12px 14px;
		border-top: 1px solid var(--color-quiet-rule);
	}
	.label {
		font-family: var(--font-mono);
		font-size: 9px;
		letter-spacing: var(--track-label);
		text-transform: uppercase;
		color: var(--color-quiet-dim);
		margin-bottom: 6px;
	}
	.edge {
		padding: 4px 0;
		font-family: var(--font-mono);
		font-size: 10.5px;
	}
	.rel {
		color: var(--color-quiet-dim);
	}
	.peer {
		color: var(--color-quiet-mid);
		text-decoration: none;
	}
	.peer:hover {
		color: var(--color-quiet-fg);
		text-decoration: underline;
	}
	.w {
		color: var(--color-quiet-dim);
		font-size: 9px;
	}
</style>
