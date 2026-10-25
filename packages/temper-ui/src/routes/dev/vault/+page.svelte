<script lang="ts">
	/**
	 * The vault render harness. Three shipped surfaces, rendered for real, judged by an eye:
	 *
	 * 1. **The resource-detail page** — the whole real page component (not a replica), with the
	 *    Data artifacts section in situ and an activity trail whose latest events are artifact
	 *    commits, so the summary line is readable where the reader actually stands.
	 * 2. **Governed families** — the context page's shapes section, both enforcement postures.
	 * 3. **The ownership filter** — the vault FilterBar, look-only: it is wired to the real
	 *    `$app/navigation`, so changing it navigates the real (authed) app. That is the honest
	 *    rendering of what shipped; a stubbed navigation would be a different component than
	 *    the one that merged.
	 *
	 * The toggle below exists because the artifacts section's absence contract — empty list
	 * renders NOTHING, page unchanged — is a behavior to be seen, not just tested.
	 */
	import { dev } from '$app/environment';
	import { error } from '@sveltejs/kit';
	import FilterBar from '$lib/components/vault/FilterBar.svelte';
	import ShapeList from '$lib/components/vault/ShapeList.svelte';
	import ResourcePage from '../../(app)/vault/r/[ident]/+page.svelte';
	import type { PageData as ResourcePageData } from '../../(app)/vault/r/[ident]/$types';
	import { emptyArtifacts, artifacts, edges, shapes, trail, resource } from './harness';
	import type { PageData } from './$types';

	let { data }: { data: PageData } = $props();

	if (!dev) error(404, 'Not found');

	let showEmpty = $state(false);

	// The resource page awaits these: hand it the same promise shape the real load does.
	// `$derived` because the toggle swaps the artifacts promise — the whole point of the arm.
	const resourceData = $derived({
		resource,
		content: Promise.resolve(
			[
				'# Quarterly latency measurements',
				'',
				'A fixture body, so the document region has something to render — including a fenced',
				'block or two, because the markdown renderer highlights the languages it knows.',
				'',
				'It cites three resources, one in each state a body link can read as: one temper',
				'[resolved](./00000000-0000-7000-8000-00000000000a) (this shows its held title), one',
				'[temper answered there is nothing to see](./00000000-0000-7000-8000-00000000000b), and one',
				'[nobody checked](./00000000-0000-7000-8000-00000000000c), inside a sentence that wraps, so',
				'a mark that moved the prose would show here.',
				'',
				'```json',
				'{',
				'  "route": "/api/resources",',
				'  "p50_ms": 412,',
				'  "p95_ms": 880,',
				'  "runs": 12,',
				'  "note": "sampled from the edge"',
				'}',
				'```',
				'',
				'And a TypeScript one, plus an unknown language rendering as plain text:',
				'',
				'```ts',
				'const percentile = (xs: number[], p: number): number => {',
				'  const sorted = [...xs].sort((a, b) => a - b); // ascending',
				'  return sorted[Math.floor((p / 100) * sorted.length)];',
				'};',
				'```',
				'',
				'```klingon',
				'Hab SoSlI’ Quch!',
				'```'
			].join('\n')
		),
		// The second phase: answered after a beat, so the links read inert first and settle in place.
		links: new Promise((settle) =>
			setTimeout(
				() =>
					settle([
						['00000000-0000-7000-8000-00000000000a', { state: 'resolved', title: 'Edge latency baseline' }],
						['00000000-0000-7000-8000-00000000000b', { state: 'absent' }]
					]),
				1200
			)
		),
		trail: Promise.resolve(trail),
		// The page reads the bounded connections read, not the bare edge list it once took.
		connections: Promise.resolve({
			rows: edges,
			total: edges.length,
			limit: edges.length,
			returned: edges.length
		}),
		// One bounded page of artifacts, as the page reads it.
		artifacts: Promise.resolve({
			artifacts: showEmpty ? emptyArtifacts : artifacts,
			olderHref: null,
			newestHref: null
		}),
		mayChange: false,
		stateVocabulary: null
	} as unknown as ResourcePageData);

	const filterValues = [
		{ label: 'Any', value: '' },
		{ label: 'has artifacts', value: 'true' },
		{ label: 'no artifacts', value: 'false' }
	];
</script>

<svelte:head><title>Vault render harness</title></svelte:head>

<div class="harness">
	<header class="controls">
		<span class="brand">⚙ Vault render harness</span>
		<span class="cap">fixtures are invented · route 404s outside dev</span>
	</header>

	<section class="panel">
		<h2>1 · Resource detail — Data artifacts section, in situ</h2>
		<p class="note">
			The whole real page: body, history (note the two <code>data_artifact_committed</code> rows
			and their summary lines), connections, and the artifacts section below the body. Toggle to
			see the absence contract — an empty list renders nothing at all, and the page reads
			exactly as a resource that owns no artifacts always has.
		</p>
		<label class="toggle">
			<input type="checkbox" bind:checked={showEmpty} />
			resource owns no artifacts (empty list → section renders nothing)
		</label>
		<div class="frame">
			<ResourcePage data={resourceData} form={null} />
		</div>
	</section>

	<section class="panel">
		<h2>2 · Context page — Governed families</h2>
		<p class="note">
			One advisory family and one enforcing (amber, at v3) — schema opens on click. On the real
			page this renders below the resource list, and renders nothing at all for a context that
			declares no families.
		</p>
		<div class="frame inset">
			<ShapeList {shapes} />
		</div>
	</section>

	<section class="panel">
		<h2>3 · Vault list — the data-artifacts ownership filter</h2>
		<p class="note">
			The tri-state select composes with the context filter like any other param.
			<strong>Look only:</strong> this control is wired to the real router — changing it
			navigates the real (auth-required) app, which locally means a login dead end.
		</p>
		<div class="frame inset">
			<FilterBar
				filters={{
					docTypes: [],
					stage: null,
					status: null,
					contextRef: null,
					q: null,
					tags: [],
					hasArtifacts: null
				}}
				revealed={null}
				fixedContext
				contexts={[]}
			/>
		</div>
	</section>
</div>

<style>
	.harness {
		max-width: 1100px;
		margin: 0 auto;
		padding: 16px 20px 48px;
	}
	.controls {
		display: flex;
		align-items: baseline;
		gap: 14px;
		padding: 6px 0 14px;
		border-bottom: 1px solid var(--tp-rule);
		margin-bottom: 18px;
	}
	.brand {
		font-family: var(--font-mono);
		font-size: 12px;
		color: var(--tp-text-muted);
	}
	.cap {
		font-family: var(--font-mono);
		font-size: 9px;
		letter-spacing: var(--track-label);
		text-transform: uppercase;
		color: var(--tp-text-subtle);
	}
	.panel {
		margin-bottom: 28px;
	}
	h2 {
		font-family: var(--font-mono);
		font-size: 12px;
		font-weight: 500;
		color: var(--tp-text-muted);
		margin: 0 0 6px;
	}
	.note {
		font-size: 12px;
		color: var(--tp-text-subtle);
		margin: 0 0 10px;
		max-width: 72ch;
	}
	.note code {
		font-family: var(--font-mono);
		font-size: 10px;
	}
	.toggle {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--tp-text-muted);
		margin-bottom: 12px;
		cursor: pointer;
	}
	.frame {
		border: 1px dashed var(--tp-rule-strong);
	}
	.frame.inset {
		background: var(--tp-surface);
	}
</style>
