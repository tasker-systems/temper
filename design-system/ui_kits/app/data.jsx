// Fake data for the authed UI kit.
const CONTEXTS = [
  { slug: '@me/myapp',       count: 847, active: true },
  { slug: '@me/blog',        count: 112 },
  { slug: '@acme/platform',  count: 2314 },
  { slug: '@acme/design-system', count: 186 },
];

const RECENTS = [
  { title: 'invoice-flow-session', ago: '3h' },
  { title: 'auth-redesign',        ago: '1d' },
  { title: 'pricing-page-v2',      ago: '2d' },
];

const RESOURCES = {
  '@me/myapp': [
    { id: 'r1', kind: 'RESEARCH',  seq: '003', title: 'Choosing an invoice provider',  stage: 'DECIDED', effort: 'MEDIUM', ago: '3d' },
    { id: 'r2', kind: 'SESSION',   seq: '012', title: 'Wire up Stripe checkout',        stage: 'ACTIVE',  effort: 'MEDIUM', ago: '2h' },
    { id: 'r3', kind: 'DECISION',  seq: '007', title: 'Tax engine: Stripe Tax vs TaxJar', stage: 'DECIDED', effort: 'LARGE',  ago: '5d' },
    { id: 'r4', kind: 'TASK',      seq: '021', title: 'Handle webhook retries',         stage: 'DEFERRED', effort: 'SMALL',  ago: '4d' },
    { id: 'r5', kind: 'CONCEPT',   seq: '—',   title: 'Invoice line-items schema',      stage: 'STABLE',  effort: 'LOW',    ago: '1w' },
    { id: 'r6', kind: 'GOAL',      seq: '002', title: 'Ship billing to first customer', stage: 'IN FLIGHT', effort: 'HIGH', ago: '2w' },
    { id: 'r7', kind: 'SESSION',   seq: '011', title: 'Map PCI scope & compliance gap', stage: 'CLOSED',  effort: 'MEDIUM', ago: '4d' },
    { id: 'r8', kind: 'RESEARCH',  seq: '004', title: 'Pricing-page copy variants',     stage: 'DRAFT',   effort: 'LOW',    ago: '1d' },
  ],
  '@me/blog': [
    { id: 'b1', kind: 'CONCEPT', seq: '—', title: 'Temper voice guide', stage: 'STABLE', effort: 'LOW', ago: '3w' },
  ],
  '@acme/platform': [],
  '@acme/design-system': [],
};

const KIND_COLOR = {
  RESEARCH: '#7eb8da',
  SESSION:  '#82c99a',
  TASK:     '#f0a870',
  CONCEPT:  '#d48ac7',
  DECISION: '#fcd34d',
  GOAL:     '#e8e4df',
};

Object.assign(window, { CONTEXTS, RECENTS, RESOURCES, KIND_COLOR });
