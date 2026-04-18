<script lang="ts">
	/**
	 * Context-rot diagram — fidelity decay over time. Memory decays (red),
	 * chat history becomes illegible (dashed grey), the throughline holds
	 * flat (temper blue).
	 *
	 * Matches `design-system/preview/diagram-context-rot.html`.
	 */

	type Props = { caption?: boolean };
	let { caption = true }: Props = $props();
</script>

<figure class="diagram-frame">
	<svg viewBox="0 0 640 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Context rot — fidelity decays over time except along the throughline">
		<!-- Y axis: fidelity -->
		<line x1="40" y1="30" x2="40" y2="170" stroke="rgba(255,255,255,0.15)" stroke-width="0.75" />
		<text x="28" y="36" font-family="var(--font-mono)" font-size="9" fill="rgba(255,255,255,0.45)" letter-spacing="2" transform="rotate(-90, 28, 36)">FIDELITY</text>

		<!-- X axis: time markers -->
		<line x1="40" y1="170" x2="610" y2="170" stroke="rgba(255,255,255,0.15)" stroke-width="0.75" />
		<text x="50" y="188" font-family="var(--font-mono)" font-size="9" fill="rgba(255,255,255,0.45)" letter-spacing="2">DAY 0</text>
		<text x="200" y="188" font-family="var(--font-mono)" font-size="9" fill="rgba(255,255,255,0.45)" letter-spacing="2">DAY 7</text>
		<text x="360" y="188" font-family="var(--font-mono)" font-size="9" fill="rgba(255,255,255,0.45)" letter-spacing="2">DAY 30</text>
		<text x="530" y="188" font-family="var(--font-mono)" font-size="9" fill="rgba(255,255,255,0.45)" letter-spacing="2">DAY 90</text>

		<!-- Decay curve: human memory -->
		<path d="M 40 45 Q 140 62 220 95 T 400 148 T 600 165" stroke="rgba(252,165,165,0.55)" stroke-width="1" fill="none" stroke-linecap="round" />
		<text x="92" y="56" font-family="var(--font-mono)" font-size="10" fill="rgba(252,165,165,0.75)" letter-spacing="1">memory</text>

		<!-- Decay curve: chat history -->
		<path d="M 40 50 Q 110 76 180 110 Q 260 140 360 158 Q 450 166 600 168" stroke="rgba(255,255,255,0.30)" stroke-width="1" fill="none" stroke-linecap="round" stroke-dasharray="2 3" />
		<text x="230" y="132" font-family="var(--font-mono)" font-size="10" fill="rgba(255,255,255,0.45)" letter-spacing="1">chat history</text>

		<!-- Flat line: temper throughline -->
		<line x1="40" y1="50" x2="600" y2="50" stroke="#7eb8da" stroke-width="1.25" stroke-linecap="round" />
		<text x="525" y="43" font-family="var(--font-mono)" font-size="10" fill="#7eb8da" letter-spacing="1.5">throughline</text>

		<!-- Tick dots on throughline -->
		<circle cx="40" cy="50" r="1.5" fill="#7eb8da" />
		<circle cx="200" cy="50" r="1.5" fill="#7eb8da" />
		<circle cx="360" cy="50" r="1.5" fill="#7eb8da" />
		<circle cx="530" cy="50" r="1.5" fill="#7eb8da" />
	</svg>

	{#if caption}
		<figcaption>
			Memory decays. Chat history becomes illegible. The <em>throughline</em> holds.
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
