<script lang="ts">
	interface Props {
		title: string;
		caption?: string | undefined;
	}

	let { title, caption }: Props = $props();
</script>

<div class="border-l-2 border-quiet-accent pl-3">
	<h2 class="text-lg font-medium text-zinc-100">{title}</h2>
	{#if caption}
		<p class="text-xs text-zinc-400">{caption}</p>
	{/if}
</div>
