<script lang="ts">
	import type { PropertyRow } from '$lib/properties';
	import DescriptionControl from './DescriptionControl.svelte';
	import PropertyValue from './PropertyValue.svelte';
	import StateControl from './StateControl.svelte';

	let {
		rows,
		vocabularyUnread = false,
		refusal = null,
		mayDescribe = false,
	}: {
		rows: PropertyRow[];
		/**
		 * The reader may change this resource, and the read that says which states its kind of
		 * work carries did not answer. Distinct from *this kind carries none*: both offer
		 * nothing, and only one of them is a degradation the reader should be able to see.
		 */
		vocabularyUnread?: boolean;
		/** The field whose last attempted change was refused, and what it was refused with. */
		refusal?: { field: string | null; message: string } | null;
		/** Whether the reader may attach a description the system has no field for. */
		mayDescribe?: boolean;
	} = $props();

	// A description holding a list or a nested object is offered no control — excluded by
	// decision, because a real structured editor is a much larger surface and raw JSON would
	// require the reader to hold the system's own vocabulary. It is said out loud rather than
	// left as a silence, because it bites on the commonest keys there are (`tags`, `keywords`,
	// `relates_to` all hold lists) and a reader would otherwise read the missing control as a
	// bug in the one description they most wanted to change.
	let hasStructured = $derived(
		mayDescribe && rows.some((r) => !r.managed && r.editable === undefined),
	);

	// The rule between the managed run and the open run. Managed keys always
	// lead (mergeProperties guarantees the order), so this is the first open row.
	let firstOpenKey = $derived(rows.find((r) => !r.managed)?.key ?? null);
</script>

<div class="props">
	<div class="label">Properties · {rows.length}</div>
	<dl>
		{#each rows as row (row.key)}
			{#if row.key === firstOpenKey}
				<hr />
			{/if}
			<div class="row" class:is-managed={row.managed}>
				<dt>{row.key}</dt>
				<dd>
					{#if row.choices}
						<StateControl
							field={row.key}
							current={typeof row.value === 'string' ? row.value : null}
							choices={row.choices}
							error={refusal?.field === row.key ? refusal.message : null}
						/>
					{:else if row.editable}
						<DescriptionControl
							name={row.key}
							current={String(row.value)}
							kind={row.editable}
							error={refusal?.field === row.key ? refusal.message : null}
						/>
					{:else}
						<PropertyValue value={row.value} />
					{/if}
				</dd>
			</div>
		{/each}
	</dl>
	{#if mayDescribe}
		<form method="POST" action="?/attachDescription" class="attach">
			<input
				type="text"
				name="name"
				placeholder="name"
				aria-label="New description — name"
				autocomplete="off"
				spellcheck="false"
				required
			/>
			<input
				type="text"
				name="value"
				placeholder="value"
				aria-label="New description — value"
				autocomplete="off"
				spellcheck="false"
				required
			/>
			<button type="submit">Attach</button>
		</form>
		{#if refusal && refusal.field === '' && refusal.message}
			<p class="err" role="alert">{refusal.message}</p>
		{/if}
	{/if}
	{#if hasStructured}
		<p class="unread">
			Descriptions holding lists or nested values are not editable here.
		</p>
	{/if}
	{#if vocabularyUnread}
		<p class="unread" role="status">
			Could not read which states this kind of work carries, so none are offered.
		</p>
	{/if}
	{#if refusal && refusal.field === null}
		<p class="unread" role="alert">{refusal.message}</p>
	{/if}
</div>

<style>
	.props {
		padding: 14px 22px 16px;
		border-bottom: 1px solid var(--color-quiet-rule);
		background: rgba(255, 255, 255, 0.015);
	}
	.label {
		font-family: var(--font-mono);
		font-size: 9px;
		letter-spacing: var(--track-label);
		text-transform: uppercase;
		color: var(--color-quiet-dim);
		margin-bottom: 9px;
	}
	dl {
		margin: 0;
	}
	.row {
		display: grid;
		grid-template-columns: 132px 1fr;
		gap: 10px;
		padding: 3px 0;
		align-items: start;
	}
	dt {
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--color-quiet-dim);
	}
	dd {
		margin: 0;
		min-width: 0;
	}
	/* Managed keys tint toward the doc-type hue; open keys stay neutral (spec D2). */
	.row.is-managed dt {
		color: color-mix(in srgb, var(--hue) 52%, var(--color-quiet-dim));
	}
	.attach {
		display: flex;
		gap: 6px;
		align-items: center;
		margin-top: 11px;
	}
	.attach input {
		min-width: 0;
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--color-quiet-mid);
		background: rgba(255, 255, 255, 0.03);
		border: 1px solid var(--color-quiet-rule);
		border-radius: 3px;
		padding: 2px 5px;
	}
	.attach input[name='name'] {
		width: 132px;
		flex: none;
	}
	.attach input[name='value'] {
		flex: 1;
	}
	.attach button {
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
	.err {
		font-family: var(--font-mono);
		font-size: 10.5px;
		color: #e0755f;
		margin: 6px 0 0;
	}
	.unread {
		font-family: var(--font-mono);
		font-size: 10.5px;
		color: var(--color-quiet-dim);
		margin: 9px 0 0;
	}
	hr {
		border: 0;
		border-top: 1px dashed rgba(255, 255, 255, 0.1);
		margin: 8px 0;
	}
</style>
