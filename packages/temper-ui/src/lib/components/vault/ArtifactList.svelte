<script lang="ts">
	import type { ArtifactView } from '$lib/types';
	import { humanBytes } from '$lib/bytes';
	import { highlightCode } from '@tasker-systems/temper-design/highlight';
	import { uncountedSentence } from '@tasker-systems/temper-design/omission';
	import { flattenPayload } from '$lib/graph/payloadRows';
	import { relativeTime } from '$lib/graph/relativeTime';

	/**
	 * The data artifacts a resource owns, whole — metadata plus the content payload itself.
	 *
	 * The load hands over ONE bounded page (`$lib/artifact-page`), folded artifacts included
	 * (`include_folded=true`): a folded artifact is rendered folded and stays reachable, because
	 * whether an artifact is live is often exactly the reader's question, and defaulting the read
	 * to hiding the folded ones would make this section answer it silently wrong. The read is
	 * newest first, so the page after this one holds OLDER artifacts; `olderHref` and
	 * `newestHref` move between pages, and when either is present the count is this page's, not
	 * the resource's. Rows also sort newest-first here, so the order the reader sees runs one
	 * way across every page.
	 *
	 * `never_declared` is omitted from the summary line rather than hidden everywhere: it is the
	 * vocabulary's "no shape is in force, there is nothing to conform to" — a non-fact, not a
	 * verdict, and the Rust enum's own doc says the state is not a degradation. A state that IS
	 * in force (`declared_*`) always shows, unchecked included — unchecked never reads as checked.
	 */
	let {
		artifacts,
		olderHref = null,
		newestHref = null,
	}: { artifacts: ArtifactView[]; olderHref?: string | null; newestHref?: string | null } =
		$props();

	let paged = $derived(olderHref !== null || newestHref !== null);
	// The read counts nothing: the page knows only whether it is the first and whether older ones
	// follow, both from the links the answer carried.
	let omission = $derived(
		uncountedSentence(
			artifacts.length,
			`${artifacts.length === 1 ? 'artifact' : 'artifacts'}, newest first`,
			{ first: newestHref === null, more: olderHref !== null }
		)
	);

	let rows = $derived(
		[...artifacts].sort((a, b) => (a.created < b.created ? 1 : a.created > b.created ? -1 : 0))
	);
	let openArtifact = $state<string | null>(null);

	const shapeClass = (s: string): string | null =>
		s === 'declared_satisfied'
			? 'shape satisfied'
			: s === 'declared_not_satisfied'
				? 'shape not-satisfied'
				: s === 'declared_not_yet_checked'
					? 'shape'
					: null;

	const shapeLabel = (s: string): string | null =>
		s === 'never_declared' ? null : s.replaceAll('_', ' ');

	const metadataRows = (a: ArtifactView): { key: string; value: string }[] => {
		const { content: _content, ...meta } = a;
		return flattenPayload(meta);
	};

	// hljs escapes the input and emits only its own spans, so {@html} of this is safe by
	// construction — the same property the markdown pipeline relies on, sanitizer or not.
	const contentHtml = (content: ArtifactView['content']): string | null =>
		content === null || content === undefined
			? null
			: highlightCode(JSON.stringify(content, null, 2), 'json');
</script>

<section>
	<div class="label">Data artifacts · {rows.length}{paged ? ' on this page' : ''}</div>
	<p class="bound" data-testid="omission">{omission}</p>
	{#each rows as artifact (artifact.artifact_id)}
		{@const content = contentHtml(artifact.content)}
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
					· <span class={shapeClass(artifact.shape_state)}>{shapeLabel(artifact.shape_state)}</span>{/if}{#if humanBytes(artifact.content_bytes)}
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
					<pre class="content hljs">{@html content}</pre>
				{:else}
					<div class="meta">no content — committed with an empty payload</div>
				{/if}
			{/if}
		</div>
	{/each}
	{#if paged}
		<nav class="pages" aria-label="Data artifact pages">
			{#if newestHref}<a href={newestHref}>← Newest artifacts</a>{/if}
			{#if olderHref}<a class="older" href={olderHref}>Older artifacts →</a>{/if}
		</nav>
	{/if}
</section>

<style>
	section {
		padding: 18px 22px 24px;
		border-top: 1px solid var(--tp-rule);
	}
	.bound {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--tp-text-subtle);
		margin: 0 0 8px;
	}
	.label {
		font-family: var(--font-mono);
		font-size: 9px;
		letter-spacing: var(--track-label);
		text-transform: uppercase;
		color: var(--tp-text-subtle);
		margin-bottom: 10px;
	}
	.artifact {
		padding: 8px 0;
	}
	.artifact + .artifact {
		border-top: 1px solid var(--tp-rule);
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
		color: var(--hue);
	}
	.chev {
		color: var(--tp-text-subtle);
	}
	.summary {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--tp-text-muted);
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
	/* Conformance, colored to be found rather than to alarm: satisfied in `success`, as the
	   event history's confidence is; non-conforming in `notice` — it is a recorded advisory
	   verdict or a badge on a folded row, never a refusal aimed at the reader; not-yet-checked
	   stays quiet, because "unchecked" must not dress up as either. */
	.shape.satisfied {
		color: var(--tp-success);
	}
	.shape.not-satisfied {
		color: var(--tp-notice);
	}
	.meta {
		font-family: var(--font-mono);
		font-size: 9px;
		color: var(--tp-text-subtle);
	}
	.meta-table {
		margin: 6px 0 0;
		border-left: 1px solid var(--tp-rule);
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
		color: var(--tp-text-subtle);
	}
	.meta-table dd {
		color: var(--tp-text-muted);
	}
	.pages {
		display: flex;
		justify-content: space-between;
		gap: 12px;
		margin-top: 10px;
		font-family: var(--font-mono);
		font-size: 10px;
	}
	.pages a {
		color: var(--hue);
	}
	.pages .older {
		margin-left: auto;
	}
	.content {
		margin: 6px 0 0;
		padding: 8px 10px;
		background: var(--tp-surface);
		border: 1px solid var(--tp-rule);
		font-family: var(--font-mono);
		font-size: 10px;
		line-height: 1.5;
		color: var(--tp-text-muted);
		overflow-x: auto;
		white-space: pre;
	}
</style>
