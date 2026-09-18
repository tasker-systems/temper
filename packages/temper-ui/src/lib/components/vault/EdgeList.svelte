<script lang="ts">
	import type { ResourceConnections } from '$lib/types/generated/graph';
	import RegionState from '$lib/components/RegionState.svelte';

	/**
	 * The bounded connections read, consumed whole. The heading IS the bound line: `rows.length`
	 * of `total`, both read off the one envelope the server returned, so the count, the stated
	 * denominator and the rendered slice cannot disagree — and the line is chrome, present
	 * whether or not the read was clipped (spec D-F4 / `bound.ts` SeedAxis: complete is something
	 * the reader is TOLD, never something they infer from silence).
	 */
	let { connections }: { connections: ResourceConnections } = $props();
</script>

<!--
	This section used to render NOTHING when `edges` was empty — an absence that says nothing, and
	one a failed read degrading to `[]` produced identically. The region now states its own emptiness
	in the shared vocabulary, so a resource with no connections and a connections read that failed
	are two different things on screen.
-->
<section>
	<!-- "Connections", not "Edges": the region below says "No connections", and one region must not
	     name the same thing two ways. "Edges" was this file's only reader-facing use of the word
	     anywhere in the UI, so aligning it introduces no inconsistency elsewhere. -->
	<div class="label">Connections · {connections.rows.length} of {connections.total}</div>
	{#if connections.rows.length === 0}
		<RegionState state="empty" label="connections" />
	{:else}
		{#each connections.rows as edge (edge.edge_id)}
			<div class="edge">
				<!--
					The row states the relationship in the reader's terms — their `label`, verbatim —
					or states nothing: an empty label renders no relationship text at all. No
					`edge_kind` fallback and no placeholder prose, because the system's structural
					name for the edge is not a word the reader wrote, and inventing one would be a
					translation table pretending to be their vocabulary. The arrows are direction,
					not relationship, and stay.
				-->
				<span class="rel">
					{#if edge.label}{edge.direction === 'outgoing' ? '' : '← '}{edge.label}{edge.direction ===
						'outgoing'
						? ' →'
						: ''}{/if}
				</span>
				<!-- A blob peer is addressed by id alone (no title) and is not a resource: it
				     never links into /vault/r, which would route a blob id into a resource page. -->
				{#if edge.peer_table === 'kb_resources'}
					<a class="peer" href="/vault/r/{edge.peer_id}">{edge.peer_title}</a>
				{:else}
					<span class="peer">blob · {edge.peer_id.slice(0, 8)}</span>
				{/if}
			</div>
		{/each}
	{/if}
</section>

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
</style>
