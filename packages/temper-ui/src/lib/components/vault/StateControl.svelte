<script lang="ts">
	/**
	 * One state the system defines, offered for change where it is read.
	 *
	 * **It submits on an explicit act and nothing else.** A `<select>` wired to submit on
	 * `change` would turn a reading act into a changing one: arrowing through a native select
	 * fires `change` per option on several platforms, so a keyboard reader looking at what the
	 * options *are* would write three states into the ledger on the way past. That is
	 * `no-reading-act-becomes-a-changing-one` failing exactly the way the clause says it
	 * fails — silently, and only for some readers. The button is the act.
	 *
	 * The values come from `choices`, which came from the doc-type's own schema. This component
	 * knows no vocabulary and must never acquire one.
	 *
	 * The submit stays disabled until the selection differs from what is stored, so the reader
	 * cannot spend a ledger event restating a value. On success SvelteKit re-runs the page's
	 * `load` and this remounts against the state that now obtains — `current` is the fresh read,
	 * never the value that was requested.
	 */
	let {
		field,
		current,
		choices,
		error,
	}: {
		field: string;
		/** The stored value, or `null` for a state this kind of work carries and this resource has not got. */
		current: string | null;
		choices: readonly string[];
		/** The refusal this field's last attempted change came back with, if it was refused. */
		error: string | null;
	} = $props();

	// The stored value, as the select's baseline. `selected` seeds from it and then diverges as
	// the reader chooses — so it is `$state`, and the seeding is deliberately a snapshot.
	// svelte-ignore state_referenced_locally
	let selected = $state(current ?? '');
	let stored = $derived(current ?? '');
	let dirty = $derived(selected !== stored);

	// Resync when the stored value changes underneath the selection. Today this never fires:
	// a plain form POST is a full navigation, so a successful change remounts this component
	// against the fresh read. It is here so that stops being load-bearing — the moment anyone
	// enhances the form, `current` starts updating in place and a stale selection would read as
	// the reader's unsaved choice over a state that has already moved.
	$effect(() => {
		selected = stored;
	});
</script>

<form method="POST" action="?/changeState" class="ctl">
	<input type="hidden" name="field" value={field} />
	<select name="value" bind:value={selected} aria-label="{field} — change">
		{#if current === null}
			<!-- Unset, and it stays unsettable-to-nothing: no door can retract a managed
			     property, so an empty option would be an affordance that overstates itself. -->
			<option value="" disabled>—</option>
		{/if}
		{#each choices as choice (choice)}
			<option value={choice}>{choice}</option>
		{/each}
	</select>
	<button type="submit" disabled={!dirty}>Change</button>
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
	select {
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--color-quiet-mid);
		background: rgba(255, 255, 255, 0.03);
		border: 1px solid var(--color-quiet-rule);
		border-radius: 3px;
		padding: 1px 4px;
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
	button:not(:disabled):hover {
		border-color: color-mix(in srgb, var(--hue) 55%, transparent);
		color: color-mix(in srgb, var(--hue) 90%, white);
	}
	.err {
		font-family: var(--font-mono);
		font-size: 10.5px;
		color: #e0755f;
		margin: 4px 0 0;
	}
</style>
