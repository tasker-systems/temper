import { describe, expect, it } from 'vitest';
import { mergeProperties, MANAGED_KEY_ORDER } from './properties';

describe('mergeProperties', () => {
	it('puts doc_type first, always', () => {
		const rows = mergeProperties({ 'temper-stage': 'done' }, { zebra: 1 }, 'concept');
		expect(rows[0]).toEqual({ key: 'doc_type', value: 'concept', managed: true });
	});

	it('orders managed keys by MANAGED_KEY_ORDER, not alphabetically', () => {
		const rows = mergeProperties(
			{ 'temper-provenance': 'user-created', 'temper-stage': 'done' },
			null,
			'task'
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
			{ key: 'doc_type', value: 'kernel_landmark', managed: true }
		]);
	});

	it('MANAGED_KEY_ORDER matches the substrate const', () => {
		// Mirrors MANAGED_PROPERTY_KEYS in crates/temper-substrate/src/keys.rs:42.
		expect(MANAGED_KEY_ORDER).toEqual([
			'temper-stage',
			'temper-mode',
			'temper-effort',
			'temper-status',
			'temper-seq',
			'temper-llm-model',
			'temper-llm-run',
			'temper-provenance',
			'temper-branch',
			'temper-pr'
		]);
	});
});
