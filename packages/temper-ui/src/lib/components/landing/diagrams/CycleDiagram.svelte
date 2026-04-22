<script lang="ts">
	/**
	 * Session continuity cycle — warm up → work → decide → record, looping
	 * around "the vault grows". Each step word pours its doctype color
	 * (blue for warmup/research, gold for decide, green for record) into a
	 * parchment baseline; "work" stays parchment — the uncoloured doing.
	 *
	 * Matches `design-system/preview/diagram-cycle.html`.
	 */

	type Props = { id?: string; caption?: boolean };
	let { id = 'cycle', caption = true }: Props = $props();
</script>

<figure class="diagram-frame">
	<svg viewBox="0 0 640 280" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Session continuity cycle — warm up, work, decide, record">
		<defs>
			<linearGradient id="{id}-warmup" x1="0" y1="0" x2="0" y2="1">
				<stop offset="0%" stop-color="#8cc5e2" />
				<stop offset="100%" stop-color="#e8e4df" />
			</linearGradient>
			<linearGradient id="{id}-decide" x1="0" y1="0" x2="0" y2="1">
				<stop offset="0%" stop-color="#f5d277" />
				<stop offset="100%" stop-color="#ebe2d0" />
			</linearGradient>
			<linearGradient id="{id}-record" x1="0" y1="0" x2="0" y2="1">
				<stop offset="0%" stop-color="#9ed3af" />
				<stop offset="100%" stop-color="#e5e6de" />
			</linearGradient>
		</defs>

		<!-- Center thread (the loop) -->
		<circle cx="320" cy="140" r="95" stroke="rgba(126,184,218,0.20)" stroke-width="0.75" fill="none" />

		<!-- 01 WARM UP (top, blue) -->
		<g>
			<text x="320" y="38" text-anchor="middle" font-family="var(--font-mono)" font-size="9" fill="rgba(255,255,255,0.32)" letter-spacing="2.5">01</text>
			<text x="320" y="58" text-anchor="middle" font-family="var(--font-serif)" font-size="18" font-weight="500" fill="url(#{id}-warmup)">warm up</text>
			<line x1="294" y1="66" x2="346" y2="66" stroke="#8cc5e2" stroke-width="0.75" opacity="0.55" />
		</g>

		<!-- 02 WORK (right, neutral) -->
		<g>
			<text x="530" y="134" text-anchor="middle" font-family="var(--font-mono)" font-size="9" fill="rgba(255,255,255,0.32)" letter-spacing="2.5">02</text>
			<text x="530" y="154" text-anchor="middle" font-family="var(--font-serif)" font-size="18" font-weight="500" fill="#e8e4df">work</text>
			<line x1="512" y1="162" x2="548" y2="162" stroke="rgba(255,255,255,0.28)" stroke-width="0.75" />
		</g>

		<!-- 03 DECIDE (bottom, amber) -->
		<g>
			<text x="320" y="234" text-anchor="middle" font-family="var(--font-mono)" font-size="9" fill="rgba(255,255,255,0.32)" letter-spacing="2.5">03</text>
			<text x="320" y="254" text-anchor="middle" font-family="var(--font-serif)" font-size="18" font-weight="500" fill="url(#{id}-decide)">decide</text>
			<line x1="296" y1="262" x2="344" y2="262" stroke="#f5d277" stroke-width="0.75" opacity="0.6" />
		</g>

		<!-- 04 RECORD (left, green) -->
		<g>
			<text x="110" y="134" text-anchor="middle" font-family="var(--font-mono)" font-size="9" fill="rgba(255,255,255,0.32)" letter-spacing="2.5">04</text>
			<text x="110" y="154" text-anchor="middle" font-family="var(--font-serif)" font-size="18" font-weight="500" fill="url(#{id}-record)">record</text>
			<line x1="86" y1="162" x2="134" y2="162" stroke="#9ed3af" stroke-width="0.75" opacity="0.6" />
		</g>

		<!-- Arrow tips along the circle -->
		<g fill="#7eb8da" opacity="0.75">
			<polygon points="413,76 416,86 406,82" transform="rotate(45 410 80)" />
			<polygon points="413,204 416,214 406,210" transform="rotate(135 410 208)" />
			<polygon points="227,204 230,214 220,210" transform="rotate(-135 224 208)" />
			<polygon points="227,76 230,86 220,82" transform="rotate(-45 224 80)" />
		</g>

		<!-- Center label -->
		<text x="320" y="138" text-anchor="middle" font-family="var(--font-mono)" font-size="9" fill="rgba(255,255,255,0.30)" letter-spacing="3">THE VAULT</text>
		<text x="320" y="156" text-anchor="middle" font-family="var(--font-serif)" font-style="italic" font-size="13" fill="rgba(255,255,255,0.48)">grows</text>
	</svg>

	{#if caption}
		<figcaption>
			Warm up with recall. Work. Decide. <em>Record.</em> The vault grows on every loop — and the colours match the doc types they produce.
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
	figcaption em {
		color: var(--temper-blue);
		font-style: italic;
	}
</style>
