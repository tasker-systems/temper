<script lang="ts">
	/**
	 * One description the reader attached, offered for revision where it is read.
	 *
	 * Free text, deliberately — there is no vocabulary to check it against, which is exactly
	 * what makes this a different act from changing a state the system defines. The two share a
	 * storage layer and nothing else: this one cannot be validated before it is sent, so it is
	 * sent and the server's answer is what the reader is shown.
	 *
	 * The value keeps the type it already had — a form submits text, and without that a revision
	 * of `priority: 3` would silently store `"3"`. The type is re-derived **server-side from the
	 * stored value**; it used to travel here as a hidden `kind` field, which let a hand-written
	 * submission choose the key *and* its JSON type. See `revisedValue` and the action.
	 *
	 * **Revision only. There is no remove.** No door retracts an open key — the write path skips
	 * nulls rather than deleting the row — so a delete control would be an affordance for
	 * something the system cannot do. Excluded by decision, not overlooked.
	 */
	let {
		name,
		current,
		error,
	}: {
		name: string;
		current: string;
		error: string | null;
	} = $props();

	// svelte-ignore state_referenced_locally
	let draft = $state(current);
	let dirty = $derived(draft !== current);

	// Resync when the stored value moves underneath an untouched draft. Today a successful
	// change is a full navigation, so this component remounts; the effect is what keeps that
	// from being load-bearing if the form is ever enhanced.
	$effect(() => {
		draft = current;
	});
</script>

<form method="POST" action="?/changeDescription" class="ctl">
	<input type="hidden" name="name" value={name} />
	<input
		type="text"
		name="value"
		bind:value={draft}
		aria-label="{name} — revise"
		autocomplete="off"
		spellcheck="false"
	/>
	<button type="submit" disabled={!dirty}>Save</button>
</form>
{#if error}
	<p class="err" role="alert">{error}</p>
{/if}

<style>
	.ctl {
		display: flex;
		gap: 6px;
		align-items: center;
	}
	input[type='text'] {
		flex: 1;
		min-width: 0;
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--color-quiet-mid);
		background: rgba(255, 255, 255, 0.03);
		border: 1px solid var(--color-quiet-rule);
		border-radius: 3px;
		padding: 2px 5px;
	}
	button {
		font-family: var(--font-mono);
		font-size: 10px;
		letter-spacing: var(--track-label);
		text-transform: uppercase;
		background: none;
		border: 1px solid color-mix(in srgb, var(--hue) 30%, transparent);
		border-radius: 3px;
		padding: 2px 7px;
		color: color-mix(in srgb, var(--hue) 70%, white);
		cursor: pointer;
	}
	button:disabled {
		border-color: var(--color-quiet-rule);
		color: var(--color-quiet-dim);
		cursor: default;
	}
	.err {
		font-family: var(--font-mono);
		font-size: 10.5px;
		color: #e0755f;
		margin: 4px 0 0;
	}
</style>
