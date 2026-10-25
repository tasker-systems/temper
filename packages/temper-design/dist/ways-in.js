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
/** How many rows a way in shows before the reader asks for more. */
export const WAY_IN_BOUND = 8;
/** The order every way in is read in, in the words both surfaces state it. */
export const WAY_IN_ORDER = 'most recently updated first';
export const WAYS_IN = [
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
