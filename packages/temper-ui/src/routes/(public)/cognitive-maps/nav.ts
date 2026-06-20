// Single source of truth for the /cognitive-maps tier structure. Mirrors the
// `order` / `parent` / `genre` / `title` frontmatter of the markdown source in
// docs/cognitive-maps/. The +layout derives prev/next from the flattened
// reading order. "Name the thing, don't point" — links carry titles, never
// ordinals.

export type Genre = 'show' | 'invite';

export interface CogMapPage {
  href: string;
  title: string;
  genre: Genre;
}

/** Reading order — flattened. Drives prev/next. The operating set has been
    promoted to the top-level /operating tier; movement 7 ("Operating Temper")
    remains here as the bridge that hands the journeyer across to it. */
export const PAGES: CogMapPage[] = [
  { href: '/cognitive-maps/what-a-cognitive-map-is', title: 'What a cognitive map is', genre: 'show' },
  { href: '/cognitive-maps/the-substrate-beneath-it', title: 'The substrate beneath it', genre: 'show' },
  { href: '/cognitive-maps/what-lives-in-a-map', title: 'What lives in a map', genre: 'show' },
  { href: '/cognitive-maps/how-a-map-grows', title: 'How a map grows', genre: 'show' },
  { href: '/cognitive-maps/how-maps-relate', title: 'How maps relate', genre: 'show' },
  { href: '/cognitive-maps/whats-visible-from-here', title: "What's visible from here", genre: 'show' },
  { href: '/cognitive-maps/operating-temper', title: 'Operating Temper', genre: 'invite' },
];

/** The tier index, the start of the reading order. */
export const INDEX: { href: string; title: string } = {
  href: '/cognitive-maps',
  title: 'Cognitive maps',
};

/** The visual fly-over — an interstitial gallery between the index and the
    first page, in the prev/next flow (index → the-set → what-a-cognitive-map-is). */
export const OVERVIEW: { href: string; title: string } = {
  href: '/cognitive-maps/the-set',
  title: 'The set, at a glance',
};
