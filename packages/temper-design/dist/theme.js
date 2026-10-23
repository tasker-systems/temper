/**
 * Theme selection, one model for every surface. Every theme this package publishes is listed in
 * `themes.json`, read here, so a new theme needs no code in any consumer. A surface keeps the
 * reader's preference where it keeps device facts (the desktop's settings store, the web's
 * cookie) and resolves it to a theme name through this module, so "follow the system" means the
 * same thing on both.
 */
import index from '@tasker-systems/temper-design/themes.json' with { type: 'json' };
export const THEMES = index
    .map(({ name, title, appearance, counterpart }) => ({ name, title, appearance, counterpart }))
    .sort((a, b) => a.name.localeCompare(b.name));
export const DEFAULT_PREFERENCE = { follow: 'system', family: 'quiet-instrument' };
/** The theme name a preference selects, given the system's appearance. Never returns an unknown name. */
export function resolveTheme(pref, systemDark, themes = THEMES) {
    const find = (name) => themes.find((t) => t.name === name);
    const fallback = find('quiet-instrument') ?? themes[0];
    if (pref.follow === 'fixed')
        return (find(pref.name) ?? fallback).name;
    const base = find(pref.family) ?? fallback;
    const want = systemDark ? 'dark' : 'light';
    if (base.appearance === want)
        return base.name;
    const other = base.counterpart ? find(base.counterpart) : undefined;
    return other && other.appearance === want ? other.name : base.name;
}
/** Anything that is not a recognisable preference — including what a store of another
 *  vintage returns — falls back to the default rather than failing the whole surface. */
export function coercePreference(value) {
    return isPreference(value) ? value : DEFAULT_PREFERENCE;
}
function isPreference(value) {
    if (typeof value !== 'object' || value === null)
        return false;
    const v = value;
    return ((v.follow === 'system' && typeof v.family === 'string') ||
        (v.follow === 'fixed' && typeof v.name === 'string'));
}
