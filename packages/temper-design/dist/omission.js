export function omissionSentence(total, shown, scope, page) {
    if (!page) {
        const omitted = Math.max(0, total - shown);
        return omitted === 0
            ? `All ${total} ${scope}.`
            : `${shown} of ${total} ${scope}; ${omitted} not shown.`;
    }
    const { offset, more } = page;
    if (offset === 0 && !more && shown === total)
        return `All ${total} ${scope}.`;
    // A page past the end (an offset beyond the total) has every row before it, not `offset` rows.
    const before = Math.min(offset, total);
    const after = more ? Math.max(0, total - offset - shown) : 0;
    const either = [before > 0 && `${before} before this page`, more && `${after} after it`]
        .filter(Boolean)
        .join(', ');
    const rows = shown === 0 ? `None of ${total}` : `${offset + 1}–${offset + shown} of ${total}`;
    return either ? `${rows} ${scope}; ${either}.` : `${rows} ${scope}.`;
}
/**
 * The omission sentence for a view whose read reports no total. An unpaged read that says whether
 * more exist is told "all" or "more not shown"; a paged one says which side more lie on. A read that
 * does not say at all is told exactly that, rather than "more" inferred from a full page.
 */
export function uncountedSentence(shown, scope, cursor) {
    if (!cursor)
        return `${shown} ${scope}; more not reported.`;
    if (!('first' in cursor))
        return cursor.more ? `${shown} ${scope}; more not shown.` : `All ${shown} ${scope}.`;
    const { first, more } = cursor;
    if (first && !more)
        return `All ${shown} ${scope}.`;
    if (first)
        return `${shown} ${scope}; more after this page.`;
    return `${shown} ${scope}; more before this page${more ? ', more after it' : ''}.`;
}
const counted = (n, word) => `${n} ${word}${n === 1 ? '' : 's'}`;
/** The clause naming where a graph figure was read from, for its description. */
export function graphStanding(arm, nameOf) {
    if (arm.read === 'walk')
        return `reached from ${arm.from.map(nameOf).join(', ')} within ${arm.depth} hops`;
    return `entered at ${arm.in.map(nameOf).join(', ')} within a draw of ${arm.k}`;
}
/** The omission sentence for a graph view, composed from the read's own numbers. */
export function graphSentence(total, shown, scope, { arm, bounds, cut }) {
    if (!arm)
        return omissionSentence(total, shown, scope, null);
    const clauses = [];
    let head;
    if (arm.read === 'walk') {
        head = shown === total ? `All ${total} ${scope}` : `${shown} of ${total} ${scope}`;
        clauses.push('deeper not reported');
    }
    else {
        const { drawn, eligible, inScope } = bounds ?? {
            drawn: total,
            eligible: total,
            inScope: total,
            truncated: false,
        };
        head =
            eligible > drawn
                ? `Drawn ${drawn} of ${eligible} eligible, ${scope} at a draw of ${arm.k}`
                : `All ${total} ${scope} at a draw of ${arm.k}`;
        const unconnected = Math.max(0, inScope - eligible);
        if (unconnected > 0)
            clauses.push(`${unconnected} in scope not connected, not drawn`);
    }
    if (cut) {
        const parts = [
            cut.nodes ? counted(cut.nodes, 'node') : null,
            cut.edges ? counted(cut.edges, 'edge') : null,
        ].filter(Boolean);
        clauses.push(`${parts.join(' and ')} past the drawing’s bounds not drawn`);
    }
    return clauses.length ? `${head}; ${clauses.join('; ')}.` : `${head}.`;
}
