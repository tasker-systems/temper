/**
 * Theme selection, one model for every surface. Every theme this package publishes is listed in
 * `themes.json`, read here, so a new theme needs no code in any consumer. A surface keeps the
 * reader's preference where it keeps device facts (the desktop's settings store, the web's
 * cookie) and resolves it to a theme name through this module, so "follow the system" means the
 * same thing on both.
 */
export type Appearance = 'dark' | 'light';
export interface ThemeMeta {
    name: string;
    title: string;
    appearance: Appearance;
    counterpart?: string;
}
/** Follow the system's appearance within a family (a theme and its counterpart), or fix one theme. */
export type ThemePreference = {
    follow: 'system';
    family: string;
} | {
    follow: 'fixed';
    name: string;
};
export declare const THEMES: ThemeMeta[];
export declare const DEFAULT_PREFERENCE: ThemePreference;
/** The theme name a preference selects, given the system's appearance. Never returns an unknown name. */
export declare function resolveTheme(pref: ThemePreference, systemDark: boolean, themes?: ThemeMeta[]): string;
/** Anything that is not a recognisable preference — including what a store of another
 *  vintage returns — falls back to the default rather than failing the whole surface. */
export declare function coercePreference(value: unknown): ThemePreference;
