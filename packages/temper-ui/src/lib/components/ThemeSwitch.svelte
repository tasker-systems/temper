<script lang="ts">
	/**
	 * The appearance control: text actions, worded as the desktop's `ThemeSwitch` words them —
	 * "Follow system", then each theme the design package publishes, by title. The choice is kept
	 * by `themeStore` (a cookie the server renders from), which the `(app)` layout initialises;
	 * this only offers it.
	 */
	import { THEMES } from '@tasker-systems/temper-design/theme';
	import { themeStore } from '$lib/stores/theme.svelte';

	const action =
		'block w-full px-3 py-1 text-left text-xs text-tp-text-subtle hover:text-tp-text aria-pressed:text-tp-text';
</script>

<div role="group" aria-label="Theme" class="py-1" data-testid="theme-switch">
	<button
		type="button"
		class={action}
		aria-pressed={themeStore.pref.follow === 'system'}
		onclick={() => themeStore.choose({ follow: 'system', family: themeStore.active })}
		>Follow system</button
	>
	{#each THEMES as theme (theme.name)}
		<button
			type="button"
			class={action}
			aria-pressed={themeStore.pref.follow === 'fixed' && themeStore.pref.name === theme.name}
			onclick={() => themeStore.choose({ follow: 'fixed', name: theme.name })}>{theme.title}</button
		>
	{/each}
</div>
