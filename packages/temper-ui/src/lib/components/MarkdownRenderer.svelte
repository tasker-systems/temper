<script lang="ts">
	import { browser } from '$app/environment';
	import type { LinkResolution } from '@tasker-systems/temper-design/links';
	import { prepareMarkdown } from '@tasker-systems/temper-design/markdown';
	import type { BodyLinks } from '@tasker-systems/temper-design/sanitize';
	import { resourceHref } from '$lib/vault-url';

	interface Props {
		markdown: string;
		/**
		 * What temper answered for the body's `./<uuid>` links, as the page's load read them. An
		 * id with no answer — or every id, when this is absent — renders unchecked: marked, and
		 * still followable to its resource page, whose own read answers. `'pending'` while the
		 * answers are still out: the links read as their typed text, inert and unmarked, and
		 * settle in place when the answers land.
		 */
		links?: [string, LinkResolution][] | 'pending';
	}

	let { markdown, links = [] }: Props = $props();

	// The sanitize pass is client-only and gates the {@html} below: while the sanitizer is
	// unavailable — on the server render, and for the instant before the dynamic import
	// resolves on the client — this component renders no body at all. It never falls back to
	// unsanitized output, which is why `prepareMarkdown`'s parse result is safe to hold here.
	// The pass itself (config and the attribute hooks) lives in `@tasker-systems/temper-design/sanitize`.
	let sanitizer: ((dirty: string, links?: BodyLinks) => string) | null = $state(null);

	if (browser) {
		import('@tasker-systems/temper-design/sanitize').then((m) => {
			sanitizer = m.sanitizeMarkdownHtml;
		});
	}

	let parsed = $derived(prepareMarkdown(markdown));
	let bodyLinks: BodyLinks = $derived(
		links === 'pending'
			? 'pending'
			: { table: new Map(links), hrefFor: (id: string) => resourceHref({ id }) },
	);

	// How the body reads is the contract's `md-body` recipe (`contract/markdown.css`, imported in
	// app.css), shared with the desktop; this component paints nothing of its own.
</script>

{#if markdown}
	{#if sanitizer}
		<div class="md-body">
			{@html sanitizer(parsed, bodyLinks)}
		</div>
	{/if}
{:else}
	<div class="py-8 text-center text-sm text-tp-text-subtle italic">
		No content available.
	</div>
{/if}
