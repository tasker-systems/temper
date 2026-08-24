/**
 * The JSON type a description's stored value has, when this surface can edit it faithfully.
 *
 * `null` means **offer nothing**: the value is a list, an object, or absent. Excluding
 * structured values is a decision, not an oversight — a real structured editor is a materially
 * larger surface than "attach a name and a value", and handing the reader raw JSON to type
 * would require them to hold the system's own vocabulary, which is the defect this whole arm
 * exists to remove. It is the exclusion most likely to be met immediately: the most common
 * recognized open keys (`tags`, `keywords`, `relates_to`) all hold lists.
 */
export type EditableKind = 'string' | 'number' | 'boolean';

export function editableKind(value: unknown): EditableKind | null {
	if (typeof value === 'string') return 'string';
	if (typeof value === 'number' && Number.isFinite(value)) return 'number';
	if (typeof value === 'boolean') return 'boolean';
	return null;
}

/**
 * Turn the text a reader typed into the value to send, keeping the type the description
 * already had.
 *
 * A form submits text and nothing else, so without this a revision of `priority: 3` would
 * store `"3"` — a change the reader did not ask for, which the table cannot show them
 * (both render as `3`) and which every downstream consumer would see. Preserving the stored
 * type is the smaller surprise, and it is bounded: `kind` only ever names a type this surface
 * already rendered, so the worst a forged one can do is give a reader's own description a type
 * they chose.
 *
 * Text that no longer fits the old type falls back to a string rather than being refused —
 * retyping `3` as `three` is a legitimate revision, not an error.
 *
 * **Attach has no stored type and does not call this**: a description the reader has just
 * invented is text, because nothing on this surface lets them say otherwise.
 */
export function revisedValue(text: string, kind: EditableKind): string | number | boolean {
	if (kind === 'number') {
		const n = Number(text);
		return text.trim() !== '' && Number.isFinite(n) ? n : text;
	}
	if (kind === 'boolean') {
		if (text === 'true') return true;
		if (text === 'false') return false;
		return text;
	}
	return text;
}
