<script lang="ts">
	interface Props {
		title: string;
		caption?: string | undefined;
	}

	let { title, caption }: Props = $props();
</script>

<div class="border-l-2 border-tp-accent pl-3">
	<h2 class="text-lg font-medium text-tp-text">{title}</h2>
	{#if caption}
		<p class="text-xs text-tp-text-muted">{caption}</p>
	{/if}
</div>
