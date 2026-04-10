<script lang="ts">
	import { marked } from 'marked';
	import { browser } from '$app/environment';

	interface Props {
		markdown: string;
	}

	let { markdown }: Props = $props();

	let sanitizer: ((dirty: string) => string) | null = $state(null);

	// Load DOMPurify on the client only — it requires a DOM.
	if (browser) {
		import('dompurify').then((mod) => {
			sanitizer = (dirty: string) => mod.default.sanitize(dirty);
		});
	}

	let html = $derived.by(() => {
		if (!markdown) return '';
		const raw = marked.parse(markdown, { async: false }) as string;
		return sanitizer ? sanitizer(raw) : raw;
	});
</script>

{#if markdown}
	<div class="md-body">
		{@html html}
	</div>
{:else}
	<div class="py-8 text-center text-sm text-zinc-500 italic">
		No content available.
	</div>
{/if}

<style>
	/* ── Rendered markdown — editorial dark theme ─────────────────────── */

	.md-body {
		font-family: var(--font-sans);
		font-size: 0.935rem;
		line-height: 1.75;
		color: rgba(255, 255, 255, 0.72);
	}

	/* ── Headings ─────────────────────────────────────────────────────── */

	.md-body :global(h1) {
		font-size: 1.65rem;
		font-weight: 600;
		color: var(--color-quiet-fg);
		margin: 2.4rem 0 0.8rem;
		letter-spacing: -0.01em;
		line-height: 1.3;
	}

	.md-body :global(h2) {
		font-size: 1.25rem;
		font-weight: 600;
		color: var(--color-quiet-fg);
		margin: 2rem 0 0.6rem;
		padding-bottom: 0.35rem;
		border-bottom: 1px solid var(--color-quiet-rule);
		letter-spacing: -0.005em;
		line-height: 1.35;
	}

	.md-body :global(h3) {
		font-size: 1.05rem;
		font-weight: 600;
		color: rgba(255, 255, 255, 0.88);
		margin: 1.6rem 0 0.4rem;
	}

	.md-body :global(h4),
	.md-body :global(h5),
	.md-body :global(h6) {
		font-size: 0.92rem;
		font-weight: 600;
		color: rgba(255, 255, 255, 0.8);
		margin: 1.2rem 0 0.3rem;
	}

	.md-body :global(h1:first-child),
	.md-body :global(h2:first-child),
	.md-body :global(h3:first-child) {
		margin-top: 0;
	}

	/* ── Body text ────────────────────────────────────────────────────── */

	.md-body :global(p) {
		margin: 0 0 1rem;
	}

	.md-body :global(strong) {
		color: rgba(255, 255, 255, 0.92);
		font-weight: 600;
	}

	.md-body :global(em) {
		color: rgba(255, 255, 255, 0.78);
	}

	/* ── Links ────────────────────────────────────────────────────────── */

	.md-body :global(a) {
		color: var(--color-quiet-accent);
		text-decoration: none;
		border-bottom: 1px solid transparent;
		transition: border-color 0.15s;
	}

	.md-body :global(a:hover) {
		border-bottom-color: var(--color-quiet-accent);
	}

	/* ── Lists ────────────────────────────────────────────────────────── */

	.md-body :global(ul),
	.md-body :global(ol) {
		margin: 0 0 1rem;
		padding-left: 1.6rem;
	}

	.md-body :global(li) {
		margin-bottom: 0.3rem;
	}

	.md-body :global(li::marker) {
		color: var(--color-quiet-accent);
	}

	.md-body :global(ul > li) {
		list-style-type: disc;
	}

	.md-body :global(ol > li) {
		list-style-type: decimal;
	}

	.md-body :global(li > ul),
	.md-body :global(li > ol) {
		margin-top: 0.3rem;
		margin-bottom: 0;
	}

	/* ── Inline code ──────────────────────────────────────────────────── */

	.md-body :global(code) {
		font-family: var(--font-mono);
		font-size: 0.82em;
		color: var(--color-quiet-accent);
		background: rgba(255, 255, 255, 0.06);
		padding: 0.15em 0.4em;
		border-radius: 3px;
	}

	/* ── Code blocks ──────────────────────────────────────────────────── */

	.md-body :global(pre) {
		background: rgba(255, 255, 255, 0.03);
		border: 1px solid var(--color-quiet-rule);
		border-radius: 4px;
		padding: 0.9rem 1rem;
		margin: 0 0 1rem;
		overflow-x: auto;
	}

	.md-body :global(pre code) {
		background: none;
		padding: 0;
		color: rgba(255, 255, 255, 0.78);
		font-size: 0.8rem;
		line-height: 1.65;
	}

	/* ── Blockquotes ──────────────────────────────────────────────────── */

	.md-body :global(blockquote) {
		border-left: 3px solid var(--color-quiet-border);
		margin: 0 0 1rem;
		padding: 0.4rem 0 0.4rem 1.2rem;
		color: rgba(255, 255, 255, 0.55);
	}

	.md-body :global(blockquote p:last-child) {
		margin-bottom: 0;
	}

	/* ── Horizontal rules ─────────────────────────────────────────────── */

	.md-body :global(hr) {
		border: none;
		border-top: 1px solid var(--color-quiet-rule);
		margin: 2rem 0;
	}

	/* ── Tables ───────────────────────────────────────────────────────── */

	.md-body :global(table) {
		width: 100%;
		border-collapse: collapse;
		margin: 0 0 1rem;
		font-size: 0.87rem;
	}

	.md-body :global(th) {
		text-align: left;
		font-weight: 600;
		color: rgba(255, 255, 255, 0.85);
		padding: 0.5rem 0.75rem;
		border-bottom: 2px solid rgba(255, 255, 255, 0.1);
		font-size: 0.82rem;
		text-transform: uppercase;
		letter-spacing: 0.03em;
	}

	.md-body :global(td) {
		padding: 0.45rem 0.75rem;
		border-bottom: 1px solid var(--color-quiet-rule);
	}

	.md-body :global(tr:last-child td) {
		border-bottom: none;
	}

	/* ── Images ───────────────────────────────────────────────────────── */

	.md-body :global(img) {
		max-width: 100%;
		border-radius: 4px;
		margin: 0.5rem 0 1rem;
	}
</style>
