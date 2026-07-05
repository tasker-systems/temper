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
	}
};
