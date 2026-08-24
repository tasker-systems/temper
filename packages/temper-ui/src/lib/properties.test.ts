import { describe, expect, it } from 'vitest';
import { mergeProperties } from './properties';

describe('mergeProperties', () => {
	it('puts doc_type first, always', () => {
		const rows = mergeProperties({ 'temper-stage': 'done' }, { zebra: 1 }, 'concept');
		expect(rows[0]).toEqual({ key: 'doc_type', value: 'concept', managed: true });
	});

	it('orders managed keys by MANAGED_KEY_ORDER, not alphabetically', () => {
		const rows = mergeProperties(
			{ 'temper-provenance': 'user-created', 'temper-stage': 'done' },
			null,
			'task',
		);
		// stage precedes provenance in MANAGED_KEY_ORDER despite sorting after it
		expect(rows.map((r) => r.key)).toEqual(['doc_type', 'temper-stage', 'temper-provenance']);
	});

	it('orders open keys alphabetically, after all managed keys', () => {
		const rows = mergeProperties({ 'temper-stage': 'done' }, { zebra: 1, alpha: 2 }, 'task');
		expect(rows.map((r) => r.key)).toEqual(['doc_type', 'temper-stage', 'alpha', 'zebra']);
	});

	it('marks managed vs open', () => {
		const rows = mergeProperties({ 'temper-stage': 'done' }, { alpha: 2 }, 'task');
		expect(rows.find((r) => r.key === 'temper-stage')!.managed).toBe(true);
		expect(rows.find((r) => r.key === 'alpha')!.managed).toBe(false);
	});

	it('sorts an unrecognized temper-* key into open, not managed', () => {
		// readback's inverse fate does the same: an unknown key lands in open.
		const rows = mergeProperties(null, { 'temper-invented': 'x', alpha: 1 }, 'task');
		expect(rows.map((r) => r.key)).toEqual(['doc_type', 'alpha', 'temper-invented']);
		expect(rows.find((r) => r.key === 'temper-invented')!.managed).toBe(false);
	});

	it('drops null-valued keys', () => {
		const rows = mergeProperties({ 'temper-stage': null }, { alpha: null, beta: 0 }, 'task');
		expect(rows.map((r) => r.key)).toEqual(['doc_type', 'beta']);
	});

	it('keeps falsy-but-present values', () => {
		const rows = mergeProperties(null, { zero: 0, empty: '', no: false }, 'fact');
		expect(rows.map((r) => r.key)).toEqual(['doc_type', 'empty', 'no', 'zero']);
	});

	it('handles both tiers absent', () => {
		expect(mergeProperties(null, null, 'kernel_landmark')).toEqual([
			{ key: 'doc_type', value: 'kernel_landmark', managed: true },
		]);
	});

	// The guard that used to sit here asserted `MANAGED_KEY_ORDER` equalled
	// `MANAGED_PROPERTY_KEYS` key for key — a third copy of the ten names, guarding the
	// second. It is gone with the copy it guarded: the surface no longer decides which
	// keys are managed, so there is no set here to keep in step with the substrate's.
	// What replaces it is the two probes below, which assert the property that copy was
	// standing in for.

	it('marks a managed key the surface has never heard of as managed', () => {
		// THE BITE. `managed_meta` is a closed typed record (`ManagedMeta`,
		// `deny_unknown_fields`) whose tier the server assigns at readback via
		// `is_managed_property_key` — so every key arriving in this argument is managed by
		// construction. Re-deriving that from a local list is what made a key added to
		// `MANAGED_PROPERTY_KEYS` render silently in the OPEN run: untinted, alphabetical,
		// on the wrong side of the rule, with nothing detecting it.
		//
		// Adding a key to the substrate's set must reach the surface with no edit here.
		const rows = mergeProperties({ 'temper-newly-minted': 'v' }, { alpha: 1 }, 'task');
		expect(rows.map((r) => r.key)).toEqual(['doc_type', 'temper-newly-minted', 'alpha']);
		expect(rows.map((r) => r.managed)).toEqual([true, true, false]);
	});

	it('ranks editorial keys ahead of managed keys it has no opinion about', () => {
		// The other half of the bite: the rank list is an EDITORIAL ordering, not the set.
		// An unranked managed key must land inside the managed run rather than at its head,
		// and alphabetically among its unranked peers — so a new key is visible without
		// displacing the deliberate order.
		const rows = mergeProperties(
			{ 'temper-zeta': 1, 'temper-provenance': 'user-created', 'temper-alpha': 2 },
			null,
			'task',
		);
		expect(rows.map((r) => r.key)).toEqual([
			'doc_type',
			'temper-provenance',
			'temper-alpha',
			'temper-zeta',
		]);
		// The key order alone does NOT bite: today the unranked keys land in the open run and
		// happen to sort into the same sequence. The flag is what distinguishes "inside the
		// managed run" from "at the head of the open run", so it is what this asserts.
		expect(rows.map((r) => r.managed)).toEqual([true, true, true, true]);
	});

	describe('offered state controls', () => {
		// `enumFields` is `DocTypeDescription.enum_fields`, read from
		// `GET /api/schema/doc-types/{name}` — the doc-type's own schema, never a set this
		// surface chose. It is passed as `null` whenever the reader may not change the
		// resource, so a read-only view is byte-identical to what it was before this arm.
		const TASK = {
			'temper-stage': ['backlog', 'in-progress', 'done', 'cancelled'],
			'temper-mode': ['plan', 'build'],
			'temper-effort': ['small', 'medium', 'large'],
		};

		it('offers exactly the values the schema carries, on the field that carries them', () => {
			const rows = mergeProperties({ 'temper-stage': 'backlog' }, null, 'task', TASK);
			const stage = rows.find((r) => r.key === 'temper-stage');
			expect(stage?.choices).toEqual(['backlog', 'in-progress', 'done', 'cancelled']);
		});

		it('offers nothing on a field with no closed vocabulary', () => {
			// `temper-branch` is a managed key with no enum. A control here would be the
			// surface inventing a vocabulary — no more, no fewer, and not a set it chose.
			const rows = mergeProperties(
				{ 'temper-stage': 'done', 'temper-branch': 'jct/x' },
				{ alpha: 1 },
				'task',
				TASK,
			);
			expect(rows.find((r) => r.key === 'temper-branch')?.choices).toBeUndefined();
			expect(rows.find((r) => r.key === 'alpha')?.choices).toBeUndefined();
			expect(rows.find((r) => r.key === 'doc_type')?.choices).toBeUndefined();
		});

		it('offers a state the work carries but does not currently hold', () => {
			// "no more, no FEWER". A task carries `temper-mode` whether or not this one has a
			// value; a surface that only decorated existing rows would present a subset of the
			// states the work carries, and the reader could never set the missing ones.
			const rows = mergeProperties({ 'temper-stage': 'done' }, null, 'task', TASK);
			const mode = rows.find((r) => r.key === 'temper-mode');
			expect(mode).toEqual({
				key: 'temper-mode',
				value: null,
				managed: true,
				choices: ['plan', 'build'],
			});
			// It sits in the managed run at its editorial rank, not appended at the end.
			expect(rows.map((r) => r.key)).toEqual([
				'doc_type',
				'temper-stage',
				'temper-mode',
				'temper-effort',
			]);
		});

		it('offers nothing at all for a kind that carries no vocabulary', () => {
			// Measured across all 14 embedded schemas: only `task` and `goal` declare an enum
			// in their own properties. Twelve kinds get no control, and that is the clause
			// holding rather than a gap.
			const rows = mergeProperties({ 'temper-provenance': 'user-created' }, null, 'concept', {});
			expect(rows.every((r) => r.choices === undefined)).toBe(true);
			expect(rows.map((r) => r.key)).toEqual(['doc_type', 'temper-provenance']);
		});

		it('offers nothing when the vocabulary could not be read', () => {
			// A 404 (out-of-vocabulary doc type) or a failed read arrives as null. An
			// affordance must never be offered over an answer nobody got.
			const rows = mergeProperties({ 'temper-stage': 'done' }, null, 'task', null);
			expect(rows.every((r) => r.choices === undefined)).toBe(true);
			expect(rows.map((r) => r.key)).toEqual(['doc_type', 'temper-stage']);
		});
	});
});
