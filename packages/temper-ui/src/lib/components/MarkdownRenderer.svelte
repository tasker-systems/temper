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
	<div class="prose prose-invert prose-sm max-w-none
		prose-headings:text-zinc-100 prose-headings:font-medium
		prose-p:text-zinc-300 prose-p:leading-relaxed
		prose-a:text-yellow-400 prose-a:no-underline hover:prose-a:underline
		prose-code:text-zinc-300 prose-code:bg-zinc-800/50 prose-code:rounded prose-code:px-1
		prose-pre:bg-zinc-900 prose-pre:border prose-pre:border-zinc-800
		prose-strong:text-zinc-200
		prose-li:text-zinc-300
		prose-blockquote:border-yellow-500/40 prose-blockquote:text-zinc-400">
		{@html html}
	</div>
{:else}
	<div class="py-8 text-center text-sm text-zinc-500 italic">
		No content available.
	</div>
{/if}
