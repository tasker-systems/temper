<script lang="ts">
	/**
	 * Region hover card (Beat A): a floating <foreignObject> above a region circle
	 * showing resources · salience · coherence — the orientation metadata for the
	 * knowledge field. Decorative: pointer-events off so it never steals the hover.
	 * Modeled on NodeHoverCard.
	 */
	import { TERRITORY_TINTS } from '$lib/graph/atlas/palette';

	interface Props {
		x: number;
		y: number;
		r: number;
		label: string | null;
		memberCount: number;
		salience: number | null;
		coherence: number | null;
	}
	let { x, y, r, label, memberCount, salience, coherence }: Props = $props();

	const CARD_WIDTH = 210;
	const CARD_HEIGHT = 96;
	const GAP = 12;
	const hue = TERRITORY_TINTS.region;
	const left = $derived(x - CARD_WIDTH / 2);
	const top = $derived(y - r - GAP - CARD_HEIGHT);
	// Coherence (content_cohesion) is a 0..1 cosine → percent. Salience is an unbounded
	// blended score (can exceed 1), so it reads as a plain 2-decimal number, not a percent.
	const pct = (v: number | null): string => (v == null ? '—' : `${Math.round(v * 100)}%`);
	const num = (v: number | null): string => (v == null ? '—' : v.toFixed(2));
</script>

<foreignObject
	x={left}
	y={top}
	width={CARD_WIDTH}
	height={CARD_HEIGHT}
	style="overflow: visible; pointer-events: none;"
>
	<div class="hc-anchor">
		<div class="region-card" style={`--hue: ${hue};`}>
			<div class="rtitle">{label ?? 'Region'}</div>
			<div class="meta">{memberCount} {memberCount === 1 ? 'resource' : 'resources'}</div>
			<div class="stats">
				<span>salience {num(salience)}</span>
				<span>coherence {pct(coherence)}</span>
			</div>
			<div class="hint">click to enter →</div>
		</div>
	</div>
</foreignObject>

<style>
	.hc-anchor {
		height: 100%;
		display: flex;
		align-items: flex-end;
	}
	.region-card {
		position: relative;
		box-sizing: border-box;
		width: 100%;
		background: rgba(20, 23, 29, 0.97);
		backdrop-filter: blur(8px);
		border: 1px solid color-mix(in srgb, var(--hue) 34%, transparent);
		border-radius: 9px;
		box-shadow: 0 8px 30px rgba(0, 0, 0, 0.55);
		color: #c9d1d9;
		text-align: left;
		padding: 10px 12px;
		font: 12px/1.45 system-ui, sans-serif;
	}
	.region-card::after {
		content: '';
		position: absolute;
		left: 50%;
		bottom: -6px;
		transform: translateX(-50%) rotate(45deg);
		width: 10px;
		height: 10px;
		background: rgba(20, 23, 29, 0.97);
		border-right: 1px solid color-mix(in srgb, var(--hue) 34%, transparent);
		border-bottom: 1px solid color-mix(in srgb, var(--hue) 34%, transparent);
	}
	.rtitle {
		font-family: Georgia, serif;
		font-size: 15px;
		color: var(--hue);
		margin-bottom: 4px;
		line-height: 1.2;
	}
	.meta {
		color: #9aa3b0;
		font-size: 11.5px;
	}
	.stats {
		display: flex;
		gap: 12px;
		margin-top: 4px;
		font: 9px monospace;
		letter-spacing: 0.1em;
		color: #8a929e;
		text-transform: uppercase;
	}
	.hint {
		margin-top: 7px;
		font: 8px monospace;
		letter-spacing: 0.16em;
		color: #5a6270;
		text-transform: uppercase;
	}
</style>
