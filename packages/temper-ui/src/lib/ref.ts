/** Extract the trailing UUID from a decorated ref (`<slug>-<uuid>`) or a bare UUID.
 *  Trailing-UUID-only: the slug half (which may contain hyphens) is ignored. */
export function parseRef(ident: string): string {
	const groups = ident.split('-');
	if (groups.length < 5) return ident; // not decorated; pass through (bare id or legacy)
	return groups.slice(-5).join('-');
}

/** Build the decorated ref `<slug>-<uuid>` for a resource (slug is presentation, uuid is identity).
 *  Falls back to the bare id when no slug is available. */
export function decoratedRef(slug: string | null | undefined, id: string): string {
	return slug ? `${slug}-${id}` : id;
}
