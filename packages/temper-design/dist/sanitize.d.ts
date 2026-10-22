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
export declare function sanitizeMarkdownHtml(dirty: string): string;
