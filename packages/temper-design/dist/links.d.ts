/**
 * The most links one body resolves: the id-set bound the server's `ids` filter enforces (the
 * `IdSet` schema's `maxItems` in the published contract), so one body is one request. Links past
 * it render unchecked.
 */
export declare const BODY_LINK_CAP = 256;
/** What temper answered for one id. Nothing answered is not an entry: it renders unchecked. */
export type LinkResolution = {
    state: 'resolved';
    title: string;
} | {
    state: 'absent';
};
/** Answers by lowercase id. */
export type LinkTable = ReadonlyMap<string, LinkResolution>;
/**
 * Hover words for a link temper answered for with nothing the reader can see. One sentence for
 * every cause: a reader must not be able to tell a deleted resource from one they may not read.
 */
export declare const ABSENT_WORDS = "Not found, or not visible to you";
/** Hover words for a link temper did not answer for: nothing was verified either way. */
export declare const UNCHECKED_WORDS = "Not checked against temper";
/** The lowercase id an href cites, or `null` when it is not exactly a vault link. */
export declare function vaultLinkId(href: string): string | null;
/**
 * The distinct vault-link ids in a body, in first-occurrence order, at most
 * {@link BODY_LINK_CAP}; `dropped` counts the distinct ids past the cap.
 */
export declare function extractVaultLinks(markdown: string): {
    ids: string[];
    dropped: number;
};
