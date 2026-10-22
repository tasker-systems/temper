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
    const after = more ? Math.max(0, total - offset - shown) : 0;
    const either = [offset > 0 && `${offset} before this page`, more && `${after} after it`]
        .filter(Boolean)
        .join(', ');
    const rows = shown === 0 ? `None of ${total}` : `${offset + 1}–${offset + shown} of ${total}`;
    return either ? `${rows} ${scope}; ${either}.` : `${rows} ${scope}.`;
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
