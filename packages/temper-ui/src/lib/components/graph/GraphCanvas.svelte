<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { drag as d3drag, type D3DragEvent } from 'd3-drag';
	import {
		forceCenter,
		forceCollide,
		forceLink,
		forceManyBody,
		forceSimulation,
		type Simulation
	} from 'd3-force';
	import { select, type Selection } from 'd3-selection';
	import { zoom as d3zoom, type D3ZoomEvent } from 'd3-zoom';

	import type { GraphEdge, GraphNode } from '$lib/types/generated/graph';
	import { buildForceConfig } from '$lib/graph/force-config';
	import { shouldShowLabel, truncateLabel } from '$lib/graph/labels';
	import { resourceHref } from '$lib/graph/navigation';
	import { seedPositions } from '$lib/graph/positions';
	import {
		toSimulationLinks,
		toSimulationNodes,
		type SimulationLink,
		type SimulationNode
	} from '$lib/graph/simulation-input';
	import { edgeStrokeDasharray, nodeColor, nodeRadius } from '$lib/graph/styling';
	import GraphTooltip from './GraphTooltip.svelte';

	interface Props {
		nodes: GraphNode[];
		edges: GraphEdge[];
		owner: string;
		context: string;
		width?: number;
		height?: number;
	}

	let {
		nodes,
		edges,
		owner,
		context,
		width = 1100,
		height = 700
	}: Props = $props();

	let svgEl: SVGSVGElement | undefined = $state();
	let hoverNode: GraphNode | null = $state(null);
	let hoverX = $state(0);
	let hoverY = $state(0);
	let currentZoom = $state(1);

	let sim: Simulation<SimulationNode, SimulationLink> | undefined;
	let gRoot: Selection<SVGGElement, unknown, null, undefined> | undefined;

	onMount(() => {
		if (!svgEl) return;

		const positions = seedPositions(nodes, edges, { width, height });
		const simNodes = toSimulationNodes(nodes, positions);
		const simLinks = toSimulationLinks(edges);
		const cfg = buildForceConfig({ width, height });

		const svg = select(svgEl);
		gRoot = svg.append('g');

		// Zoom / pan
		const zoomBehavior = d3zoom<SVGSVGElement, unknown>()
			.scaleExtent([0.3, 3])
			.on('zoom', (ev: D3ZoomEvent<SVGSVGElement, unknown>) => {
				gRoot?.attr('transform', ev.transform.toString());
				currentZoom = ev.transform.k;
				updateLabelVisibility();
			});
		svg.call(zoomBehavior);

		// Edges (drawn first so they render under nodes)
		const linkSel = gRoot
			.append('g')
			.attr('stroke', '#94a3b8')
			.attr('stroke-opacity', 0.6)
			.selectAll<SVGLineElement, SimulationLink>('line')
			.data(simLinks)
			.enter()
			.append('line')
			.attr('stroke-dasharray', (d) => edgeStrokeDasharray(d.edge_type));

		// Node groups (circle + label)
		const nodeGroup = gRoot
			.append('g')
			.selectAll<SVGGElement, SimulationNode>('g')
			.data(simNodes)
			.enter()
			.append('g')
			.attr('cursor', 'pointer')
			.on('click', (_ev, d) => {
				goto(resourceHref(owner, context, d));
			})
			.on('mouseenter', function (ev, d) {
				hoverNode = d;
				const rect = svgEl!.getBoundingClientRect();
				hoverX = ev.clientX - rect.left;
				hoverY = ev.clientY - rect.top;
			})
			.on('mousemove', function (ev) {
				const rect = svgEl!.getBoundingClientRect();
				hoverX = ev.clientX - rect.left;
				hoverY = ev.clientY - rect.top;
			})
			.on('mouseleave', function () {
				hoverNode = null;
			});

		nodeGroup
			.append('circle')
			.attr('r', (d) => nodeRadius(d))
			.attr('fill', (d) => nodeColor(d.doc_type))
			.attr('stroke', '#111827')
			.attr('stroke-width', 1);

		const labelSel = nodeGroup
			.append('text')
			.attr('dy', '.35em')
			.attr('text-anchor', 'middle')
			.attr('y', (d) => -nodeRadius(d) - 4)
			.attr('font-size', 11)
			.attr('pointer-events', 'none')
			.attr('fill', '#1f2937')
			.text((d) => truncateLabel(d.title, 30));

		// Drag
		const dragBehavior = d3drag<SVGGElement, SimulationNode>()
			.on('start', (ev: D3DragEvent<SVGGElement, SimulationNode, unknown>, d) => {
				if (!ev.active && sim) sim.alphaTarget(0.3).restart();
				d.fx = d.x;
				d.fy = d.y;
			})
			.on('drag', (ev, d) => {
				d.fx = ev.x;
				d.fy = ev.y;
			})
			.on('end', (ev, d) => {
				if (!ev.active && sim) sim.alphaTarget(0);
				d.fx = null;
				d.fy = null;
			});
		nodeGroup.call(dragBehavior);

		// Simulation
		sim = forceSimulation<SimulationNode>(simNodes)
			.force(
				'link',
				forceLink<SimulationNode, SimulationLink>(simLinks)
					.id((d) => d.id)
					.distance(cfg.linkDistance)
			)
			.force('charge', forceManyBody().strength(cfg.charge))
			.force('center', forceCenter(cfg.centerX, cfg.centerY))
			.force(
				'collide',
				forceCollide<SimulationNode>((d) => nodeRadius(d) + cfg.collisionPadding)
			);

		sim.on('tick', () => {
			linkSel
				.attr('x1', (d) => (d.source as unknown as SimulationNode).x ?? 0)
				.attr('y1', (d) => (d.source as unknown as SimulationNode).y ?? 0)
				.attr('x2', (d) => (d.target as unknown as SimulationNode).x ?? 0)
				.attr('y2', (d) => (d.target as unknown as SimulationNode).y ?? 0);
			nodeGroup.attr('transform', (d) => `translate(${d.x ?? 0}, ${d.y ?? 0})`);
		});

		function updateLabelVisibility() {
			labelSel.attr('display', (d) => (shouldShowLabel(d, currentZoom) ? null : 'none'));
		}
		updateLabelVisibility();
	});

	onDestroy(() => {
		sim?.stop();
	});
</script>

<div class="relative" style="width: {width}px; height: {height}px;">
	<svg bind:this={svgEl} {width} {height} class="block bg-neutral-50 dark:bg-neutral-950"></svg>
	<GraphTooltip node={hoverNode} x={hoverX} y={hoverY} />
</div>
