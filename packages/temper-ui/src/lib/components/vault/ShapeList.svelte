<script lang="ts">
	import type { ShapeView } from '$lib/types/generated/data_artifact_shape';
	import { highlightCode } from '$lib/highlight';

	/**
	 * The data-artifact families a context governs — one row per family in force: the family
	 * name, its enforcement posture, the declaration's chain depth, and the schema itself on
	 * open. The shapes read arrives complete (`is_folded` rows are the registry's internal
	 * lineage — a folded declaration is superseded, not governance the reader can act on, and
	 * `shape_version` already tells that story on the live row).
	 *
	 * An `enforcing` posture is styled to be found, not to alarm: it is a property of future
	 * commits (they will be refused when they do not conform), never a verdict on the reader.
	 */
	let { shapes }: { shapes: ShapeView[] } = $props();

	let rows = $derived([...shapes].sort((a, b) => a.artifact_kind.localeCompare(b.artifact_kind)));
	let openShape = $state<string | null>(null);

	// hljs escapes the input and emits only its own spans, so {@html} of this is safe by
	// construction — the same property the markdown pipeline relies on, sanitizer or not.
	const schemaHtml = (schema: ShapeView['schema']): string =>
		highlightCode(JSON.stringify(schema, null, 2), 'json');
</script>

<section>
	<div class="label">Governed families · {rows.length}</div>
	{#each rows as shape (shape.shape_id)}
		<div class="shape">
			<button
				class="head"
				aria-expanded={openShape === shape.shape_id}
				onclick={() => (openShape = openShape === shape.shape_id ? null : shape.shape_id)}
			>
				<span class="family">{shape.artifact_kind}</span>
				<span class="posture">
					{#if shape.enforcement === 'enforcing'}
						<span class="enforcing">enforcing</span>
					{:else}
						<span class="advisory">advisory</span>
					{/if}
					· v{shape.shape_version}
				</span>
				<span class="chev">{openShape === shape.shape_id ? '⌄' : '›'}</span>
			</button>
			{#if openShape === shape.shape_id}
				<pre class="schema hljs">{@html schemaHtml(shape.schema)}</pre>
			{/if}
		</div>
	{/each}
</section>

<style>
	section {
		padding: 18px 22px 24px;
		border-top: 1px solid var(--color-quiet-rule);
	}
	.label {
		font-family: var(--font-mono);
		font-size: 9px;
		letter-spacing: var(--track-label);
		text-transform: uppercase;
		color: var(--color-quiet-dim);
		margin-bottom: 10px;
	}
	.shape {
		padding: 6px 0;
	}
	.shape + .shape {
		border-top: 1px solid var(--color-quiet-rule);
	}
	.head {
		display: flex;
		align-items: center;
		gap: 10px;
		width: 100%;
		background: none;
		border: 0;
		padding: 0;
		cursor: pointer;
		text-align: left;
	}
	.family {
		font-family: var(--font-mono);
		font-size: 11px;
		color: color-mix(in srgb, var(--hue) 70%, white);
	}
	.posture {
		font-family: var(--font-mono);
		font-size: 9px;
		color: var(--color-quiet-dim);
		margin-left: auto;
	}
	.enforcing {
		color: var(--decision-amber-lt);
		text-transform: uppercase;
		letter-spacing: var(--track-label);
	}
	.advisory {
		text-transform: uppercase;
		letter-spacing: var(--track-label);
	}
	.chev {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--color-quiet-dim);
	}
	.schema {
		margin: 6px 0 0;
		padding: 8px 10px;
		background: var(--color-quiet-card);
		border: 1px solid color-mix(in srgb, var(--hue) 18%, transparent);
		font-family: var(--font-mono);
		font-size: 10px;
		line-height: 1.5;
		color: var(--color-quiet-mid);
		overflow-x: auto;
		white-space: pre;
	}
</style>
