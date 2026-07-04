// camera.ts
/**
 * d3-zoom camera: applies pan/zoom transforms to the viewport <g> inside the SVG.
 * Decoupled from tier — zoom is within-tier observability only (spec D2). Returns a
 * handle with destroy() to unwire on component teardown.
 */
import { select } from 'd3-selection';
import { zoom, type ZoomBehavior } from 'd3-zoom';

export interface Camera {
	destroy(): void;
}

export function attachCamera(
	svgEl: SVGSVGElement,
	viewportEl: SVGGElement,
	opts: { min: number; max: number }
): Camera {
	const svg = select(svgEl);
	const viewport = select(viewportEl);

	const behavior: ZoomBehavior<SVGSVGElement, unknown> = zoom<SVGSVGElement, unknown>()
		.scaleExtent([opts.min, opts.max])
		.on('zoom', (event) => {
			viewport.attr('transform', event.transform.toString());
		});

	svg.call(behavior);

	return {
		destroy() {
			svg.on('.zoom', null);
		}
	};
}
