// bytes.ts — human legibility for a byte count, shared by the surfaces that show one.
//
// A legibility aid, never a claim about exactness: the payload or row that carries the
// count stays the contract (event payloads carry `content_bytes`; `ArtifactView` does too).
// 1024-based, one decimal, trailing .0 trimmed.
export function humanBytes(v: unknown): string | null {
	const n = typeof v === 'bigint' ? Number(v) : v;
	if (typeof n !== 'number' || !Number.isFinite(n) || n < 0) return null;
	const units = ['B', 'KB', 'MB', 'GB', 'TB'];
	let m = n;
	let u = 0;
	while (m >= 1024 && u < units.length - 1) {
		m /= 1024;
		u += 1;
	}
	const magnitude = u === 0 ? String(m) : m.toFixed(1).replace(/\.0$/, '');
	return `${magnitude} ${units[u]}`;
}
