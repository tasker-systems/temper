<script lang="ts">
	/**
	 * A reference to a resource, as a read answered for it. The title shown is the one temper
	 * returned; a reference whose read returned no title reads as unresolved and does not link —
	 * a link to what temper did not answer for would overstate itself, and a placeholder in its
	 * place would read as one resource and be another. The desktop's `ResourceRef` keeps the same
	 * rule; this one takes the title from the read that carried the reference rather than
	 * resolving it again.
	 */
	import { resourceHref } from '$lib/vault-url';

	let {
		id,
		title,
		class: className = ''
	}: { id: string; title: string | null | undefined; class?: string } = $props();
</script>

{#if title}
	<a class={className} href={resourceHref({ id })}>{title}</a>
{:else}
	<span class="{className} unresolved" title={id} data-testid="unresolved-ref"
		><span aria-hidden="true">⊘ </span>Unresolved reference</span
	>
{/if}

<style>
	span.unresolved {
		color: var(--tp-text-subtle);
		font-style: italic;
	}
</style>
