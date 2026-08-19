import { browser } from '$app/environment';

const KEY = 'temper.sidebar.collapsed';

/** Graph routes default to a collapsed rail (they want the width). Pure. */
export function defaultCollapsed(pathname: string): boolean {
	return pathname.startsWith('/graph');
}

function load(): boolean | null {
	if (!browser) return null;
	const v = localStorage.getItem(KEY);
	return v === null ? null : v === '1';
}

let collapsed = $state(false);

export const sidebarCollapsed = {
	get value() {
		return collapsed;
	},
	set(v: boolean) {
		collapsed = v;
		if (browser) localStorage.setItem(KEY, v ? '1' : '0');
	},
	toggle() {
		this.set(!collapsed);
	},
	/** Seed from stored preference, else the route default. Explicit user choice wins. */
	initFor(pathname: string) {
		const stored = load();
		collapsed = stored === null ? defaultCollapsed(pathname) : stored;
	},
};

// ---------------------------------------------------------------------------
// Per-group collapse.
//
// Deliberately the same mechanism as the rail's own collapse above — same
// module, same `temper.sidebar.*` key namespace, same browser guard, same
// "explicit user choice wins over the default" rule. A second persistence
// mechanism for the same surface is the drift this replaces.
//
// Only groups the reader has EXPLICITLY collapsed are stored, so a group that
// appears later (a new team, a newly shared context) defaults to expanded — the
// nav never hides a place because of a preference set before it existed.
// ---------------------------------------------------------------------------

const GROUPS_KEY = 'temper.sidebar.groups.collapsed';

/**
 * Read the persisted collapsed-group keys. Anything unparseable or not a list
 * of strings reads as "nothing collapsed": a corrupt preference must not hide
 * places, and the reader can always re-collapse. Pure.
 */
export function parseCollapsedGroups(raw: string | null): string[] {
	if (raw === null) return [];
	try {
		const parsed: unknown = JSON.parse(raw);
		if (!Array.isArray(parsed)) return [];
		return parsed.filter((k): k is string => typeof k === 'string');
	} catch {
		return [];
	}
}

/** Add or remove `key`, preserving order. Pure. */
export function toggleCollapsedGroup(keys: readonly string[], key: string): string[] {
	return keys.includes(key) ? keys.filter((k) => k !== key) : [...keys, key];
}

let collapsedGroups = $state<string[]>([]);

export const sidebarGroups = {
	isCollapsed(key: string): boolean {
		return collapsedGroups.includes(key);
	},
	toggle(key: string) {
		collapsedGroups = toggleCollapsedGroup(collapsedGroups, key);
		if (browser) localStorage.setItem(GROUPS_KEY, JSON.stringify(collapsedGroups));
	},
	/** Seed from the stored preference. Nothing stored → every group expanded. */
	init() {
		if (browser) collapsedGroups = parseCollapsedGroups(localStorage.getItem(GROUPS_KEY));
	},
};
