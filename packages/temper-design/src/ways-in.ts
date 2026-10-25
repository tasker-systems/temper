/**
 * The standing ways into the work: the lists both surfaces offer a reader on arrival, declared
 * once so each is offered under the same name, answering the same question, bounded the same way
 * (`the-same-ways-in-on-both-surfaces`). The set is the one ruled for both — active goals,
 * in-progress tasks and sessions — with the latest handoff beside them (`./handoff`).
 *
 * The desktop contributes these through its `temper-workflows` plugin manifest; the web has no
 * plugin model and serves them as its own defaults. Each surface maps `filter` onto its own read.
 * Every list is read newest update first, and says so (`WAY_IN_ORDER`): a list that does not
 * state its order leaves the reader to guess what "first" means.
 */

/**
 * What a way in reads: the reader's own work of one doc type, narrowed by its workflow state where
 * one applies. `owner: '@me'` is ownership, which is the nearest read that exists of "yours": work
 * the reader does on what someone else owns is not in it, and the scope words say "of yours" rather
 * than claiming more. When a read of what the reader changed exists, this is where it moves.
 */
export type WayInFilter = { docType: string; owner: '@me'; status?: string; stage?: string };

export type WayIn = {
	id: string;
	/** The name both surfaces show. */
	label: string;
	/** What the list holds, as its omission sentence names it. */
	scope: string;
	filter: WayInFilter;
};

/** How many rows a way in shows before the reader asks for more. */
export const WAY_IN_BOUND = 8;

/** The order every way in is read in, in the words both surfaces state it. */
export const WAY_IN_ORDER = 'most recently updated first';

export const WAYS_IN: readonly WayIn[] = [
	{
		id: 'goals',
		label: 'goals',
		scope: 'active goals of yours',
		filter: { docType: 'goal', owner: '@me', status: 'active' },
	},
	{
		id: 'tasks',
		label: 'in progress',
		scope: 'tasks in progress of yours',
		filter: { docType: 'task', owner: '@me', stage: 'in-progress' },
	},
	{
		id: 'sessions',
		label: 'sessions',
		scope: 'sessions of yours',
		filter: { docType: 'session', owner: '@me' },
	},
];
