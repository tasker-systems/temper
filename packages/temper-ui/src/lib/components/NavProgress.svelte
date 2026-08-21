<script lang="ts">
	import { navigating } from '$app/stores';

	// `to` is null when the navigation leaves the app entirely — then there is no destination
	// to name, only the fact that something is under way. Two channels, never one: the hairline
	// moves and the label says so, so the acknowledgement survives both a still frame and a
	// screen reader.
	let target = $derived($navigating?.to?.url.pathname ?? null);
	let label = $derived(target ? `Loading ${target}…` : 'Loading…');
</script>

{#if $navigating}
	<div
		data-testid="nav-progress"
		role="status"
		class="pointer-events-none fixed inset-x-0 top-0 z-50 flex items-center gap-2"
	>
		<div class="h-0.5 flex-1 animate-pulse bg-quiet-accent"></div>
		<span
			class="rounded-b border border-t-0 border-zinc-800 bg-zinc-900 px-2 py-0.5 text-[10px] text-zinc-400"
			>{label}</span
		>
	</div>
{/if}
