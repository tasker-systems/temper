<script lang="ts">
	interface Props {
		message: string;
		action?: { label: string; href: string } | undefined;
	}

	let { message, action }: Props = $props();
</script>

<div class="flex flex-col items-center justify-center gap-3 py-16 text-zinc-500">
	<p class="text-sm">{message}</p>
	{#if action}
		<a
			href={action.href}
			class="text-xs text-quiet-accent hover:text-quiet-fg border border-zinc-700 rounded px-3 py-1"
		>
			{action.label}
		</a>
	{/if}
</div>
