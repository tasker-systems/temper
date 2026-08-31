<script lang="ts">
	import type { ArtifactView } from '$lib/types';
	import { humanBytes } from '$lib/bytes';
	import { flattenPayload } from '$lib/graph/payloadRows';
	import { relativeTime } from '$lib/graph/relativeTime';

	/**
	 * The data artifacts a resource owns, whole — metadata plus the content payload itself.
	 *
	 * The list arrives complete from the load (`include_folded=true`): a folded artifact is
	 * rendered folded and stays reachable, because whether an artifact is live is often exactly
	 * the reader's question, and defaulting the read to hiding the folded ones would make this
	 * section answer it silently wrong. Rows sort newest-first here — the read makes no ordering
	 * promise, so the one that renders must.
	 *
	 * `never_declared` is omitted from the summary line rather than hidden everywhere: it is the
	 * vocabulary's "no shape is in force, there is nothing to conform to" — a non-fact, not a
	 * verdict, and the Rust enum's own doc says the state is not a degradation. A state that IS
	 * in force (`declared_*`) always shows, unchecked included — unchecked never reads as checked.
	 */
	let { artifacts }: { artifacts: ArtifactView[] } = $props();

	let rows = $derived(
		[...artifacts].sort((a, b) => (a.created < b.created ? 1 : a.created > b.created ? -1 : 0))
	);
	let openArtifact = $state<string | null>(null);

	const shapeLabel = (s: string): string | null =>
		s === 'never_declared' ? null : s.replaceAll('_', ' ');

	const metadataRows = (a: ArtifactView): { key: string; value: string }[] => {
		const { content: _content, ...meta } = a;
		return flattenPayload(meta);
	};

	const contentText = (content: ArtifactView['content']): string | null =>
		content === null || content === undefined ? null : JSON.stringify(content, null, 2);
</script>

<section>
	<div class="label">Data artifacts · {rows.length}</div>
	{#each rows as artifact (artifact.artifact_id)}
		{@const content = contentText(artifact.content)}
		<div class="artifact" class:folded={artifact.is_folded}>
			<button
				class="head"
				aria-expanded={openArtifact === artifact.artifact_id}
				onclick={() =>
					(openArtifact = openArtifact === artifact.artifact_id ? null : artifact.artifact_id)}
			>
				<span class="family">{artifact.artifact_kind}</span>
				<span class="chev">{openArtifact === artifact.artifact_id ? '⌄' : '›'}</span>
			</button>
			<div class="summary">
				{artifact.intent}{#if artifact.is_folded}
					· <span class="fold">folded</span>{/if}{#if shapeLabel(artifact.shape_state)}
					· <span class="shape">{shapeLabel(artifact.shape_state)}</span>{/if}{#if humanBytes(artifact.content_bytes)}
					· {humanBytes(artifact.content_bytes)}{/if}
			</div>
			<div class="meta">{relativeTime(artifact.created)}</div>
			{#if openArtifact === artifact.artifact_id}
				<dl class="meta-table">
					{#each metadataRows(artifact) as row (row.key)}
						<div><dt>{row.key}</dt><dd>{row.value}</dd></div>
					{/each}
				</dl>
				{#if content}
					<pre class="content">{content}</pre>
				{:else}
					<div class="meta">no content — committed with an empty payload</div>
				{/if}
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
	.artifact {
		padding: 8px 0;
	}
	.artifact + .artifact {
		border-top: 1px solid var(--color-quiet-rule);
	}
	.artifact.folded .head,
	.artifact.folded .summary {
		opacity: 0.55;
	}
	.head {
		display: flex;
		justify-content: space-between;
		align-items: center;
		width: 100%;
		background: none;
		border: 0;
		padding: 0;
		cursor: pointer;
		font-family: var(--font-mono);
		font-size: 11px;
		color: color-mix(in srgb, var(--hue) 70%, white);
	}
	.chev {
		color: var(--color-quiet-dim);
	}
	.summary {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--color-quiet-mid);
		margin: 2px 0 1px;
	}
	.fold {
		font-style: italic;
	}
	.shape {
		text-transform: uppercase;
		letter-spacing: var(--track-label);
		font-size: 9px;
	}
	.meta {
		font-family: var(--font-mono);
		font-size: 9px;
		color: var(--color-quiet-dim);
	}
	.meta-table {
		margin: 6px 0 0;
		border-left: 1px solid color-mix(in srgb, var(--hue) 25%, transparent);
		padding-left: 8px;
	}
	.meta-table div {
		display: grid;
		grid-template-columns: 150px 1fr;
		gap: 6px;
	}
	.meta-table dt,
	.meta-table dd {
		font-family: var(--font-mono);
		font-size: 9px;
		margin: 0;
		word-break: break-word;
	}
	.meta-table dt {
		color: var(--color-quiet-dim);
	}
	.meta-table dd {
		color: var(--color-quiet-mid);
	}
	.content {
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
