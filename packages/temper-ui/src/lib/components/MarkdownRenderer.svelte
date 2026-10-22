<script lang="ts">
	import { browser } from '$app/environment';
	import { prepareMarkdown } from '@tasker-systems/temper-design/markdown';

	interface Props {
		markdown: string;
	}

	let { markdown }: Props = $props();

	// The sanitize pass is client-only and gates the {@html} below: while the sanitizer is
	// unavailable — on the server render, and for the instant before the dynamic import
	// resolves on the client — this component renders no body at all. It never falls back to
	// unsanitized output, which is why `prepareMarkdown`'s parse result is safe to hold here.
	// The pass itself (config and the attribute hooks) lives in `@tasker-systems/temper-design/sanitize`.
	let sanitizer: ((dirty: string) => string) | null = $state(null);

	if (browser) {
		import('@tasker-systems/temper-design/sanitize').then((m) => {
			sanitizer = m.sanitizeMarkdownHtml;
		});
	}

	let parsed = $derived(prepareMarkdown(markdown));

	// How the body reads is the contract's `md-body` recipe (`contract/markdown.css`, imported in
	// app.css), shared with the desktop; this component paints nothing of its own.
</script>

{#if markdown}
	{#if sanitizer}
		<div class="md-body">
			{@html sanitizer(parsed)}
		</div>
	{/if}
{:else}
	<div class="py-8 text-center text-sm text-tp-text-subtle italic">
		No content available.
	</div>
{/if}
