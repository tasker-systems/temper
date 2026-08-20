// relativeTime.ts — humanize an ISO timestamp as "2h ago", falling back to a plain
// date for anything older than a week. Pure; `now` is injectable for tests.
export function relativeTime(iso: string, now: Date = new Date()): string {
	const then = new Date(iso).getTime();
	const secs = Math.round((now.getTime() - then) / 1000);
	if (secs < 45) return 'just now';
	const mins = Math.round(secs / 60);
	if (mins < 60) return `${mins}m ago`;
	const hours = Math.round(mins / 60);
	if (hours < 24) return `${hours}h ago`;
	const days = Math.round(hours / 24);
	if (days <= 7) return `${days}d ago`;
	return iso.slice(0, 10);
}
