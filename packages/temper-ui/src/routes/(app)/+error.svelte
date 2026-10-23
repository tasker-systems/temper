<script lang="ts">
	import { page } from '$app/stores';
</script>

<div class="flex flex-col items-center justify-center h-full gap-4 text-tp-text-subtle">
	<div class="text-6xl font-light text-tp-text-subtle">{$page.status}</div>
	<h2 class="text-lg font-medium text-tp-text">{$page.error?.message ?? 'Something went wrong'}</h2>
	<a href="/vault/all" class="text-xs text-tp-accent hover:text-tp-text">&larr; Back to vault</a>
</div>
