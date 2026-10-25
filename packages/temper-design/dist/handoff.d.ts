/**
 * The latest handoff: the newest session the reader wrote, and the next steps it recorded. Both
 * surfaces show it on arrival under the same name, and both quote the next steps through
 * `nextSection` — moved here from the desktop (`temper-contrib`,
 * `apps/desktop/src/lib/shell/next-section.ts`) unchanged, so the two cannot quote the same
 * session differently.
 *
 * The next steps a session recorded: the section under the first heading that starts with "Next"
 * ("Next", "Next steps", "Next: the home view"), at any level, up to the next heading of the same
 * or a higher level. Quoted as written, trimmed and bounded — never summarised, and `null` when the
 * session has no such heading, so nothing is ever invented in its place.
 */
/** The name both surfaces give the latest handoff. */
export declare const HANDOFF_LABEL = "latest handoff";
/** How much of the section is quoted before it says it continues. */
export declare const NEXT_BOUND = 600;
export type NextSection = {
    heading: string;
    text: string;
    truncated: boolean;
};
export declare function nextSection(markdown: string, bound?: number): NextSection | null;
