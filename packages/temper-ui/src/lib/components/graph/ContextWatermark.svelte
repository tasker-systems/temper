<script lang="ts">
	interface Props {
		context: string;
	}

	let { context }: Props = $props();
</script>

<!-- Bottom-left context watermark — mirrors the prototype
     ui_kits/app/KnowledgeGraph.jsx:276-291. Huge italic serif behind
     everything, with a mono-caps `CONTEXT` preamble. z-index 0 keeps
     it beneath the canvas so node hits are unaffected; pointer-events
     none for good measure. -->
<div
	class="pointer-events-none absolute bottom-6 left-8 z-0 select-none font-serif text-[88px] italic leading-none tracking-[-0.02em]"
	style="color: rgba(255,255,255,0.035);"
	data-testid="graph-context-watermark"
>
	<span
		class="mr-3.5 font-mono text-[14px] align-middle not-italic tracking-[0.22em]"
		style="opacity: 0.7;"
	>
		CONTEXT
	</span>
	<em>{context}</em>
</div>
