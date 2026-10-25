import { type LinkTable } from './links.js';
export declare const SANITIZE_CONFIG: {
    FORBID_ATTR: string[];
    FORBID_TAGS: string[];
};
/** Words for a withheld image; exported so the witnesses and the components agree. */
export declare const WITHHELD_IMAGE = "remote image withheld";
/** Words for withheld media, by element; exported for the same reason. */
export declare const WITHHELD_MEDIA: {
    readonly video: "video withheld";
    readonly audio: "audio withheld";
};
/**
 * How a body's vault links render: the answers temper gave and the surface's own resource
 * address, or `'pending'` while the body is still arriving (a streamed reply), when links are
 * inert typed text and say nothing yet.
 */
export type BodyLinks = {
    table: LinkTable;
    hrefFor: (id: string) => string;
} | 'pending';
export declare function sanitizeMarkdownHtml(dirty: string, links?: BodyLinks): string;
