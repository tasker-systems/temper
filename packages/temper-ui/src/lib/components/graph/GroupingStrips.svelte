<script lang="ts">
	/**
	 * The groupings as marks on their own axes, one strip per quantity.
	 *
	 * `[from the reader session, 2026-08-21, and again 2026-08-22]` The table this stands in front of
	 * is *"a long, long grouping table … probably the wrong visual register for that much data
	 * density about things folks don't really understand"*, and what was asked for instead was
	 * *"a visualization that they could hover over pieces to understand the grouping"*.
	 *
	 * **Every grouping is a mark, and every mark is on the quantity's OWN axis**, labelled with the
	 * real span this place measures. Nothing is normalised, nothing is scored out of a hundred, and
	 * no two strips share a scale — a grouping sitting far right on one strip and far left on
	 * another is saying two things about two quantities, not one thing about its rank.
	 *
	 * Hovering anywhere on a strip picks the nearest grouping **and lights it on every other strip
	 * at once**, which is the whole point: a row of a table shows one grouping's figures, and this
	 * shows where those figures sit among the other five hundred. That comparison is the thing a
	 * reader cannot do by reading numbers off a column.
	 *
	 * The table is not deleted — it sits below, in a disclosure, so the raw figures stay reachable
	 * and nothing is withheld.
	 */
	import {
		type AnalysedRegion,
		type Axis,
		axisFor,
		describeAxis,
		describeConcentration,
		describeConstant,
		describeNulls,
		describeRange,
		distributionOf,
		formatValue,
		type MetricKey,
		METRICS,
		type MetricSpec,
		positionOn,
	} from '$lib/graph/analysis';

	let { regions }: { regions: AnalysedRegion[] } = $props();

	/** Geometry in user units; the SVG scales to its column, so these are not pixels on screen. */
	const W = 1000;
	const H = 44;
	const PAD = 10;

	interface Strip {
		spec: MetricSpec;
		/** Null when this quantity does not vary, or has no values — then it is SAID, never plotted. */
		marks: { region: AnalysedRegion; x: number; y: number }[] | null;
		min: number | null;
		max: number | null;
		median: number | null;
		constant: boolean;
		/** The median's REAL place on this axis, `0`–`1`. Not the middle of the strip. */
		medianAt: number | null;
		/** The median coincides with a bound, so the bound's own label carries it. */
		medianAtStart: boolean;
		medianAtEnd: boolean;
		range: string;
		nulls: string | null;
		/** Set when the axis is `log1p`-spaced, so the reader is told distance is not difference. */
		compressed: string | null;
		/** Set when most groupings share one value, which no spacing can spread out. */
		concentrated: string | null;
	}

	/**
	 * Deterministic vertical offset, so 501 marks at similar values do not stack into one line and
	 * hide their own density.
	 *
	 * Derived from the grouping's id rather than its index: a mark must not move when the list is
	 * re-ordered or filtered, or the reader would read the movement as meaning something.
	 */
	function jitter(id: string): number {
		let h = 0;
		for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) | 0;
		return ((h >>> 0) % 1000) / 1000;
	}

	const strips: Strip[] = $derived(
		METRICS.map((spec) => {
			const d = distributionOf(regions, spec.key);
			const axis: Axis | null = axisFor(d);
			const base = {
				spec,
				min: d.min,
				max: d.max,
				median: d.median,
				constant: d.constant,
				range: describeRange(d),
				nulls: describeNulls(d),
				compressed: axis ? describeAxis(axis) : null,
				concentrated: describeConcentration(regions, spec.key),
				medianAt: axis ? positionOn(axis.median, axis) : null,
				medianAtStart: axis ? positionOn(axis.median, axis) < 0.08 : false,
				medianAtEnd: axis ? positionOn(axis.median, axis) > 0.92 : false,
			};
			// A quantity with no spread is not a distribution. Plotting 501 marks on one point would
			// draw a picture of nothing and invite a reader to look for structure in it.
			if (axis === null) return { ...base, marks: null };
			return {
				...base,
				marks: regions.flatMap((region) => {
					const v = region.values[spec.key];
					if (v === null) return [];
					return [
						{
							region,
							x: PAD + positionOn(v, axis) * (W - PAD * 2),
							y: 9 + jitter(region.regionId) * (H - 18),
						},
					];
				}),
			};
		}),
	);

	/** The grouping under the pointer — lit on every strip, and named in the readout above them. */
	let active = $state<AnalysedRegion | null>(null);

	/**
	 * Pick the nearest mark to the pointer along the axis.
	 *
	 * Done against the strip's own marks rather than with a hit target per circle: at 501 marks the
	 * circles overlap, so per-mark targets would make the densest part of the plot — the part a
	 * reader most wants to interrogate — the hardest part to point at.
	 */
	function pick(event: PointerEvent, strip: Strip) {
		if (!strip.marks?.length) return;
		const box = (event.currentTarget as SVGElement).getBoundingClientRect();
		const x = ((event.clientX - box.left) / box.width) * W;
		let best = strip.marks[0];
		for (const m of strip.marks) if (Math.abs(m.x - x) < Math.abs(best.x - x)) best = m;
		active = best.region;
	}

	const valueOf = (r: AnalysedRegion, k: MetricKey) => formatValue(r.values[k]);
</script>

<div class="strips" data-testid="grouping-strips">
	<!-- The readout sits ABOVE the strips rather than following the pointer: a tooltip that tracks
	     the cursor across seven stacked strips covers the strips it is describing. -->
	<div class="readout" class:lit={active !== null} data-testid="strip-readout" aria-live="polite">
		{#if active}
			<span class="name" class:unnamed={active.label === null}
				>{active.label ?? 'An unnamed grouping'}</span
			>
			<span class="figures">
				{#each METRICS as m (m.key)}
					<span class="fig" class:absent={active.values[m.key] === null}>
						<span class="fig-label">{m.label}</span>
						<span class="fig-value">{valueOf(active, m.key)}</span>
					</span>
				{/each}
			</span>
		{:else}
			<span class="prompt">Point at a strip to read one grouping across all of them.</span>
		{/if}
	</div>

	{#each strips as strip (strip.spec.key)}
		<section class="strip">
			<header>
				<h3>{strip.spec.label} <code>{strip.spec.field}</code></h3>
				<!-- What it tells you AND what it does not. The second half is why this line exists. -->
				<p class="reading">{strip.spec.reading}</p>
			</header>

			{#if strip.marks === null}
				<!-- Said, not drawn. `internal_tension` is identically 0 across all 501 groupings of
				     one real context; an ordering over 501 identical values is an order made of noise. -->
				<p class="flat" data-testid={`flat-${strip.spec.key}`}>
					{strip.constant ? describeConstant(distributionOf(regions, strip.spec.key)) : strip.range}
				</p>
			{:else}
				<div class="plot">
				<svg
					viewBox={`0 0 ${W} ${H}`}
					preserveAspectRatio="none"
					role="img"
					aria-label={`${regions.length} groupings by ${strip.spec.label}. ${strip.range}.`}
					onpointermove={(e) => pick(e, strip)}
					onpointerleave={() => (active = null)}
				>
					<line class="axis" x1={PAD} y1={H - 2} x2={W - PAD} y2={H - 2} />
					<!-- The median, ticked WHERE IT IS. It used to be a label in the middle of the row
					     beneath, which put "median 0" at the centre of a strip whose median sits hard
					     against the left end — a label claiming a position it did not have, on every
					     strip, by up to half the width. -->
					{#if strip.medianAt !== null}
						<line
							class="median-tick"
							x1={PAD + strip.medianAt * (W - PAD * 2)}
							y1={4}
							x2={PAD + strip.medianAt * (W - PAD * 2)}
							y2={H - 2}
						/>
					{/if}
					{#each strip.marks as m (m.region.regionId + m.region.lensId)}
						<circle
							class="mark"
							class:lit={active?.regionId === m.region.regionId}
							cx={m.x}
							cy={m.y}
							r={active?.regionId === m.region.regionId ? 7 : 4}
						/>
					{/each}
				</svg>
				</div>
				<!-- The median rides at its own position. When it lands ON a bound it is not drawn a
				     second time — it IS that bound's value, and two labels stacked on one point would
				     read as two facts. The end label says so instead. -->
				<div class="scale">
					<span class:is-median={strip.medianAtStart}
						>{formatValue(strip.min)}{strip.medianAtStart ? ' · median' : ''}</span
					>
					{#if strip.medianAt !== null && !strip.medianAtStart && !strip.medianAtEnd}
						<span class="median-label" style={`left:${strip.medianAt * 100}%`}
							>median {formatValue(strip.median)}</span
						>
					{/if}
					<span class:is-median={strip.medianAtEnd}
						>{strip.medianAtEnd ? 'median · ' : ''}{formatValue(strip.max)}</span
					>
				</div>
			{/if}

			{#if strip.concentrated}
				<p class="nulls" data-testid={`concentrated-${strip.spec.key}`}>{strip.concentrated}</p>
			{/if}
			{#if strip.compressed}
				<!-- A compressed axis silently changes what distance means. Saying so is the same
				     obligation as never showing an unbounded quantity on a 0–100 scale. -->
				<p class="nulls" data-testid={`compressed-${strip.spec.key}`}>{strip.compressed}</p>
			{/if}
			{#if strip.nulls}
				<p class="nulls">{strip.nulls}</p>
			{/if}
		</section>
	{/each}
</div>

<style>
	.strips {
		display: flex;
		flex-direction: column;
		gap: 18px;
	}
	.readout {
		position: sticky;
		top: 0;
		z-index: 1;
		display: flex;
		flex-direction: column;
		gap: 6px;
		min-height: 54px;
		padding: 8px 12px;
		border: 1px solid #2b3140;
		border-radius: 4px;
		background: #12161d;
	}
	.readout.lit {
		border-color: #4a5568;
	}
	.prompt {
		color: #6f7886;
		font-size: 13px;
		font-style: italic;
	}
	.name {
		color: #e6edf5;
		font-size: 13px;
		font-weight: 600;
		line-height: 1.4;
	}
	.name.unnamed {
		color: #8b94a5;
		font-weight: 400;
		font-style: italic;
	}
	.figures {
		display: flex;
		flex-wrap: wrap;
		gap: 4px 16px;
	}
	.fig {
		display: flex;
		gap: 6px;
		font-size: 11.5px;
	}
	.fig-label {
		color: #6f7886;
	}
	.fig-value {
		color: #c3ccd9;
		font-variant-numeric: tabular-nums;
	}
	.fig.absent .fig-value {
		color: #4a5568;
	}
	.strip {
		display: flex;
		flex-direction: column;
		gap: 4px;
		border: 0;
		padding: 0;
	}
	header {
		display: flex;
		flex-direction: column;
		gap: 2px;
	}
	h3 {
		margin: 0;
		font-size: 13px;
		font-weight: 600;
		color: #c3ccd9;
	}
	h3 code {
		color: #6f7886;
		font-size: 11px;
		font-weight: 400;
	}
	.reading {
		margin: 0;
		color: #8b94a5;
		font-size: 12px;
		line-height: 1.5;
		max-width: 76ch;
	}
	svg {
		display: block;
		width: 100%;
		height: 44px;
		touch-action: none;
	}
	.plot {
		position: relative;
		margin-top: 4px;
	}
	.median-tick {
		stroke: #545c6b;
		stroke-width: 1;
		stroke-dasharray: 2 2;
		vector-effect: non-scaling-stroke;
	}
	.median-label {
		position: absolute;
		top: 0;
		transform: translateX(-50%);
		padding: 0 4px;
		color: #6f7886;
		font-size: 11px;
		font-variant-numeric: tabular-nums;
		white-space: nowrap;
		pointer-events: none;
	}
	.axis {
		stroke: #2b3140;
		stroke-width: 1;
		vector-effect: non-scaling-stroke;
	}
	.mark {
		fill: #7aa2c4;
		fill-opacity: 0.38;
	}
	.mark.lit {
		fill: #ffd479;
		fill-opacity: 1;
		stroke: #12161d;
		stroke-width: 2;
		vector-effect: non-scaling-stroke;
	}
	.scale {
		position: relative;
		display: flex;
		justify-content: space-between;
		color: #6f7886;
		font-size: 11px;
		font-variant-numeric: tabular-nums;
	}
	.scale .is-median {
		color: #8b94a5;
	}
	.flat,
	.nulls {
		margin: 0;
		color: #8b94a5;
		font-size: 12px;
	}
	.flat {
		padding: 6px 0;
		color: #aab4c0;
	}
</style>
