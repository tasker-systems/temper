import { describe, expect, it } from 'vitest';
import { relativeTime } from './relativeTime';

const now = new Date('2026-07-06T12:00:00Z');
describe('relativeTime', () => {
	it('renders seconds/minutes/hours/days ago', () => {
		expect(relativeTime('2026-07-06T11:59:30Z', now)).toBe('just now');
		expect(relativeTime('2026-07-06T11:30:00Z', now)).toBe('30m ago');
		expect(relativeTime('2026-07-06T10:00:00Z', now)).toBe('2h ago');
		expect(relativeTime('2026-07-04T12:00:00Z', now)).toBe('2d ago');
	});
	it('falls back to a date for old events', () => {
		expect(relativeTime('2026-05-01T12:00:00Z', now)).toBe('2026-05-01');
	});
});
