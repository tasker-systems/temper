<script lang="ts">
	/**
	 * Throughline layers diagram — the six doctypes as typeset words aligned
	 * to a central thread. Each word pours its semantic color from a
	 * saturated top into a parchment baseline; the thread carries the
	 * current from one layer to the next.
	 *
	 * Matches `design-system/preview/diagram-layers.html`.
	 */

	type Props = { id?: string; caption?: boolean };
	let { id = 'layers', caption = true }: Props = $props();
</script>

<figure class="diagram-frame">
	<svg viewBox="0 0 640 260" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Throughline layers — six doctypes from goal to concept">
		<defs>
			<linearGradient id="{id}-goal" x1="0" y1="0" x2="0" y2="1">
				<stop offset="0%" stop-color="#8cc5e2" />
				<stop offset="100%" stop-color="#e8e4df" />
			</linearGradient>
			<linearGradient id="{id}-research" x1="0" y1="0" x2="0" y2="1">
				<stop offset="0%" stop-color="#8cc5e2" />
				<stop offset="100%" stop-color="#e8e4df" />
			</linearGradient>
			<linearGradient id="{id}-decision" x1="0" y1="0" x2="0" y2="1">
				<stop offset="0%" stop-color="#f5d277" />
				<stop offset="100%" stop-color="#ebe2d0" />
			</linearGradient>
			<linearGradient id="{id}-task" x1="0" y1="0" x2="0" y2="1">
				<stop offset="0%" stop-color="#f0a870" />
				<stop offset="100%" stop-color="#ece0d2" />
			</linearGradient>
			<linearGradient id="{id}-session" x1="0" y1="0" x2="0" y2="1">
				<stop offset="0%" stop-color="#9ed3af" />
				<stop offset="100%" stop-color="#e5e6de" />
			</linearGradient>
			<linearGradient id="{id}-concept" x1="0" y1="0" x2="0" y2="1">
				<stop offset="0%" stop-color="#d89ccb" />
				<stop offset="100%" stop-color="#e8dde4" />
			</linearGradient>
		</defs>

		<!-- Vertical thread -->
		<line x1="320" y1="28" x2="320" y2="236" stroke="rgba(126,184,218,0.22)" stroke-width="1" />

		<!-- 01 GOAL -->
		<g>
			<text x="40" y="44" font-family="var(--font-mono)" font-size="9.5" fill="rgba(255,255,255,0.38)" letter-spacing="2.2">01 · GOAL</text>
			<text x="320" y="44" text-anchor="middle" font-family="var(--font-serif)" font-size="15" font-weight="500" fill="url(#{id}-goal)">outcome</text>
			<text x="600" y="44" text-anchor="end" font-family="var(--font-serif)" font-style="italic" font-size="12" fill="rgba(255,255,255,0.52)">what we are building</text>
			<line x1="320" y1="49" x2="320" y2="54" stroke="#8cc5e2" stroke-width="1" opacity="0.75" />
		</g>

		<!-- 02 RESEARCH -->
		<g>
			<text x="40" y="80" font-family="var(--font-mono)" font-size="9.5" fill="rgba(255,255,255,0.38)" letter-spacing="2.2">02 · RESEARCH</text>
			<text x="320" y="80" text-anchor="middle" font-family="var(--font-serif)" font-size="15" font-weight="500" fill="url(#{id}-research)">survey</text>
			<text x="600" y="80" text-anchor="end" font-family="var(--font-serif)" font-style="italic" font-size="12" fill="rgba(255,255,255,0.52)">alternatives considered</text>
			<line x1="320" y1="85" x2="320" y2="90" stroke="#8cc5e2" stroke-width="1" opacity="0.75" />
		</g>

		<!-- 03 DECISION -->
		<g>
			<text x="40" y="116" font-family="var(--font-mono)" font-size="9.5" fill="rgba(255,255,255,0.38)" letter-spacing="2.2">03 · DECISION</text>
			<text x="320" y="116" text-anchor="middle" font-family="var(--font-serif)" font-size="15" font-weight="500" fill="url(#{id}-decision)">choice</text>
			<text x="600" y="116" text-anchor="end" font-family="var(--font-serif)" font-style="italic" font-size="12" fill="rgba(255,255,255,0.52)">what we decided</text>
			<line x1="320" y1="121" x2="320" y2="126" stroke="#f5d277" stroke-width="1" opacity="0.8" />
		</g>

		<!-- 04 TASK -->
		<g>
			<text x="40" y="152" font-family="var(--font-mono)" font-size="9.5" fill="rgba(255,255,255,0.38)" letter-spacing="2.2">04 · TASK</text>
			<text x="320" y="152" text-anchor="middle" font-family="var(--font-serif)" font-size="15" font-weight="500" fill="url(#{id}-task)">unit of work</text>
			<text x="600" y="152" text-anchor="end" font-family="var(--font-serif)" font-style="italic" font-size="12" fill="rgba(255,255,255,0.52)">what comes next</text>
			<line x1="320" y1="157" x2="320" y2="162" stroke="#f0a870" stroke-width="1" opacity="0.8" />
		</g>

		<!-- 05 SESSION -->
		<g>
			<text x="40" y="188" font-family="var(--font-mono)" font-size="9.5" fill="rgba(255,255,255,0.38)" letter-spacing="2.2">05 · SESSION</text>
			<text x="320" y="188" text-anchor="middle" font-family="var(--font-serif)" font-size="15" font-weight="500" fill="url(#{id}-session)">the work</text>
			<text x="600" y="188" text-anchor="end" font-family="var(--font-serif)" font-style="italic" font-size="12" fill="rgba(255,255,255,0.52)">what happened</text>
			<line x1="320" y1="193" x2="320" y2="198" stroke="#9ed3af" stroke-width="1" opacity="0.8" />
		</g>

		<!-- 06 CONCEPT — terminus, no tick -->
		<g>
			<text x="40" y="224" font-family="var(--font-mono)" font-size="9.5" fill="rgba(255,255,255,0.38)" letter-spacing="2.2">06 · CONCEPT</text>
			<text x="320" y="224" text-anchor="middle" font-family="var(--font-serif)" font-size="15" font-weight="500" fill="url(#{id}-concept)">vocabulary</text>
			<text x="600" y="224" text-anchor="end" font-family="var(--font-serif)" font-style="italic" font-size="12" fill="rgba(255,255,255,0.52)">shared meaning</text>
		</g>
	</svg>

	{#if caption}
		<figcaption>
			The thread pours colour into each word; the word settles it. A tick below carries the current onward — until the last layer, where it pools.
		</figcaption>
	{/if}
</figure>

<style>
	.diagram-frame {
		background: var(--obsidian-3);
		padding: 1.5rem 1.75rem;
		margin: 2rem 0 1.5rem;
	}
	svg {
		display: block;
		width: 100%;
		height: auto;
	}
	figcaption {
		margin-top: 0.75rem;
		font-family: var(--font-serif);
		font-style: italic;
		font-size: 0.8rem;
		color: var(--graphite);
		line-height: 1.55;
	}
</style>
