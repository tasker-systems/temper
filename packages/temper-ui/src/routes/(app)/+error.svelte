<script lang="ts">
	import { page } from '$app/stores';
</script>

<div class="flex flex-col items-center justify-center h-full gap-4 text-zinc-500">
	<div class="text-6xl font-light text-zinc-700">{$page.status}</div>
	<h2 class="text-lg font-medium text-zinc-200">{$page.error?.message ?? 'Something went wrong'}</h2>
	<a href="/vault/all" class="text-xs text-yellow-500 hover:text-yellow-400">&larr; Back to vault</a>
</div>
