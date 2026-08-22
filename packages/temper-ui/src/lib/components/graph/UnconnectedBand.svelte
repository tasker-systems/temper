<script lang="ts">
	/**
	 * The unconnected band: **a sentence, and a disclosure that opens a list.**
	 *
	 * `[ruled — 2026-08-22, Pete]` *"a bar of lined up dots just does not work visually … the visual
	 * disjoint between a force graph and a row of dot-like-resources really clashes."* The band used
	 * to be a row of `.node-chip` marks under the canvas. The canvas earns its marks — position
	 * carries meaning there, because a force layout puts related things near each other — and the row
	 * borrowed that grammar with none of the semantics. A reader who has just learned to read
	 * position read the row the same way and got a claim that was not there.
	 *
	 * So this list makes **no spatial claim at all**, which is the entire repair. It is also not a
	 * plainer band: it is the same ruling this surface already made one rung up — *"dots a reader
	 * cannot use are not more honest than a sentence"* — applied consistently rather than argued
	 * again.
	 *
	 * **A row opens the rail, exactly as clicking the mark did.** The marks it replaces were
	 * interactive, and a list that only displayed names would be a regression dressed as a fix. It
	 * calls the same `onSelect` the canvas hands its marks; the rail is the peek, and no hover card
	 * belongs on a list row.
	 *
	 * **The caption is not decoration and must not be dropped.** It is what closed
	 * [degree 87 draws zero links](./01a024d3-2a16-78b1-9e7e-a0e98bd87e0e) — a reader meeting a
	 * stranded hub can tell *why* it is stranded without hovering it. The per-row figure states the
	 * same fact per item and **reinforces** it; it is not a replacement that lets the sentence go.
	 *
	 * Three fields per row, and that is the ruling's list: title, doc type, elsewhere-count. Where a
	 * resource *lives* is deliberately not a fourth — the rail a row opens carries it, and so does
	 * the accessibility list beside the canvas, which is every node rather than this subset.
	 *
	 * `[ruled — 2026-08-22, Pete]` **This list reads as ranked, and that is fine. Do not re-open it.**
	 * Observed on production: the rows descend by corpus connectivity, 87 down to 11 — monotonic
	 * across all 26. **Nothing here sorts them.** They are rendered in the order `parts.unconnected`
	 * arrives, and the entry read returns its nodes top-K by degree, so the band inherits that order;
	 * §2.3's *unranked-everything is the design* is untouched in the code.
	 *
	 * What changed is the **medium, not the data**. As a row of identical dots the order was
	 * unreadable; as a list it is the first thing a reader takes in. The list did not create a
	 * ranking — it exposed one that was always there and could not previously be read.
	 *
	 * It is fine **because every row states its own figure**, so the order is *self-evidencing*
	 * rather than an implied claim: a reader can see why the first row is first and check it against
	 * the next. That is what would change the answer if this is ever revisited — rows losing their
	 * own counts is what turns a visible order back into a claim nobody can verify.
	 */
	import type { GraphNode } from '$lib/graph/model';
	import { describeNodeLinks, describeUnconnected } from '$lib/graph/presentation';
	import { docTypeHue } from '$lib/graph/palette';

	interface Props {
		/** The members, in the order the answer returned them. Placing these must never rank them. */
		nodes: GraphNode[];
		/** Everything this answer drew, so the caption can say *N of these M*. */
		total: number;
		/** Opens the rail for that node — the same call the canvas hands its marks. */
		onSelect: (id: string) => void;
		/** Which row's rail is open, if any. */
		selected: string | null;
	}
	let { nodes, total, onSelect, selected }: Props = $props();

	// The corpus figures are read off the nodes rather than passed in: `describeUnconnected` requires
	// one per member and states the answer-scoped sentence unless every one of them is reported, so
	// a read that measured nothing cannot inherit a claim it never made.
	const caption = $derived(
		describeUnconnected(
			nodes.length,
			total,
			nodes.map((n) => n.corpusDegree),
		),
	);
</script>

{#if caption}
	<details class="band" data-testid="unconnected-band">
		<summary>
			<span data-testid="unconnected-caption">{caption}</span>
		</summary>
		<ul data-testid="unconnected-list">
			{#each nodes as n (n.id)}
				<li>
					<button
						type="button"
						class="row"
						aria-current={selected === n.id ? 'true' : undefined}
						onclick={() => onSelect(n.id)}
					>
						<span class="title">{n.title}</span>
						<span class="kind" style="color: {docTypeHue(n.doc_type)}"
							>{n.doc_type ?? 'kind not reported'}</span
						>
						<span class="elsewhere">{describeNodeLinks(n)}</span>
					</button>
				</li>
			{/each}
		</ul>
	</details>
{/if}

<style>
	.band {
		flex: none;
		max-height: 40%;
		overflow: auto;
		padding: 8px 14px 10px;
		border-top: 1px solid rgba(255, 255, 255, 0.07);
		background: rgba(255, 255, 255, 0.015);
		color: #8b94a5;
		font-size: 11.5px;
		line-height: 1.6;
	}
	summary {
		cursor: pointer;
		color: #8b94a5;
	}
	ul {
		margin: 8px 0 0;
		padding: 0;
		list-style: none;
		display: grid;
		gap: 2px;
	}
	.row {
		display: grid;
		grid-template-columns: minmax(0, 1fr) auto auto;
		gap: 10px;
		align-items: baseline;
		width: 100%;
		padding: 4px 6px;
		border: 0;
		border-radius: 5px;
		background: none;
		color: inherit;
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.row:hover,
	.row:focus-visible {
		background: rgba(255, 255, 255, 0.05);
	}
	.row[aria-current='true'] {
		background: rgba(255, 255, 255, 0.08);
	}
	.title {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		color: #c3cbd6;
		font-size: 12.5px;
	}
	.kind {
		font:
			9px/1.6 ui-monospace,
			Menlo,
			monospace;
		letter-spacing: 0.14em;
		text-transform: uppercase;
	}
	.elsewhere {
		color: #79828f;
		font-variant-numeric: tabular-nums;
	}
	@media (max-width: 900px) {
		.row {
			grid-template-columns: minmax(0, 1fr);
			gap: 0;
		}
	}
</style>
