// Single source of truth for the /operating tier structure. The hub stands
// alone for the cold enterprise evaluator; the four children are the dimensions
// a deployment shapes. The +layout derives prev/next from this reading order.
// "Name the thing, don't point" — links carry titles, never ordinals.

export interface OperatingPage {
  href: string;
  title: string;
}

/** The tier hub — start of the reading order. */
export const INDEX: OperatingPage = {
  href: '/operating',
  title: 'Operating Temper',
};

/** Reading order for the four dimensions a deployment shapes. Drives prev/next. */
export const PAGES: OperatingPage[] = [
  { href: '/operating/deployment', title: 'Deployment' },
  { href: '/operating/governance-and-administration', title: 'Governance & administration' },
  { href: '/operating/observability-and-audit', title: 'Observability & audit' },
  { href: '/operating/insights', title: 'Insights' },
];
