<script lang="ts">
	/**
	 * Reusable temper brand mark + wordmark.
	 *
	 * Used by the public landing Nav/Footer and the authed app sidebar/footer.
	 * The mark currentColor inherits from the parent, so place inside an
	 * element with the desired text color.
	 */

	type Size = 'sm' | 'md';

	let { size = 'md', href = '/' }: { size?: Size; href?: string | null } = $props();

	const dims = $derived(size === 'sm' ? { mark: 16, font: '0.7rem' } : { mark: 20, font: '0.75rem' });
</script>

{#snippet inner()}
	<svg
		class="mark"
		width={dims.mark}
		height={dims.mark}
		viewBox="0 0 32 32"
		xmlns="http://www.w3.org/2000/svg"
		aria-hidden="true"
	>
		<path
			d="M 12 7 L 12 25"
			stroke="currentColor"
			stroke-width="3.5"
			stroke-linecap="round"
			fill="none"
		/>
		<path
			d="M 6 13 L 18 13 Q 23 13 25 16.5 Q 27 20 25 24"
			stroke="currentColor"
			stroke-width="2.8"
			stroke-linecap="round"
			fill="none"
		/>
	</svg>
	<span class="wordmark" style:font-size={dims.font}>temper</span>
{/snippet}

{#if href}
	<a class="logo" {href}>
		{@render inner()}
	</a>
{:else}
	<span class="logo">
		{@render inner()}
	</span>
{/if}

<style>
	.logo {
		display: inline-flex;
		align-items: center;
		gap: 0.5rem;
		text-decoration: none;
		color: inherit;
		transition: opacity 0.2s;
	}
	.logo:hover {
		opacity: 0.8;
	}
	.wordmark {
		font-family: 'JetBrains Mono', 'Fira Code', monospace;
		font-weight: 500;
		letter-spacing: 0.15em;
	}
</style>
