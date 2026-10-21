import { describe, expect, it } from 'vitest';
import { formAction, withoutActionKeys } from './form-action';

const at = (search: string) => new URL(`http://ui.invalid/vault/r/r1${search}`);

describe('formAction', () => {
	it('is exactly `?/name` on a page with no query of its own', () => {
		expect(formAction(at(''), 'changeState')).toBe('?/changeState');
	});

	it("keeps the page's own query, so a POST does not drop the page's position", () => {
		const action = formAction(at('?artifacts_after_created=c&artifacts_after_id=i'), 'changeState');
		expect(action).toBe('?artifacts_after_created=c&artifacts_after_id=i&/changeState');
		// SvelteKit names the action by the first key that starts with `/`.
		const keys = [...new URL(action, 'http://ui.invalid/vault/r/r1').searchParams.keys()];
		expect(keys.find((k) => k.startsWith('/'))).toBe('/changeState');
	});

	it('replaces the action key an earlier POST left behind rather than stacking a second', () => {
		const action = formAction(at('?artifacts_after_id=i&/changeState'), 'attachDescription');
		expect(action).toBe('?artifacts_after_id=i&/attachDescription');
	});
});

describe('withoutActionKeys', () => {
	it('drops every `/`-prefixed key and keeps the rest, values included', () => {
		const kept = withoutActionKeys(new URLSearchParams('a=1&/changeState&b=2&/x='));
		expect(kept.toString()).toBe('a=1&b=2');
	});
});
