// ============================================================================
// Temper Knowledge Graph — seed data
// ============================================================================
// Shape matches the R11 participant/aggregator model:
//   - participants: research / task / session
//   - aggregators:  goal / concept / decision
//
// Names mirror the actual research chain in the uploaded vault (R7 → R11,
// the llm-wiki goal, the throughline concept, the April 2026 sessions).
// Dates are stripped from labels at render time (moved to marginalia).
// ============================================================================

/* eslint-disable no-unused-vars */
const GRAPH_NODES = [
  // ── RESEARCH ─────────────────────────────────────────────────────────────
  { id: 'r2',  type: 'research', label: 'r2 data-model',              fullTitle: 'R2 — Data Model Foundations',                      edges: 11, stage: 'DONE',        sessions: 0 },
  { id: 'r7',  type: 'research', label: 'r7 vertex-edge',             fullTitle: 'R7 — Vertex-Edge Knowledge Graph',                  edges: 18, stage: 'DONE',        sessions: 5 },
  { id: 'r9',  type: 'research', label: 'r9 svelte ui',               fullTitle: 'R9 — Svelte UI Design',                             edges: 9,  stage: 'DONE',        sessions: 2 },
  { id: 'r11', type: 'research', label: 'r11 visualization',          fullTitle: 'R11 — Knowledge-Graph Visualization Design',        edges: 5,  stage: 'IN-PROGRESS', sessions: 3 },
  { id: 'r4',  type: 'research', label: 'r4 session-continuity',      fullTitle: 'R4 — Session Continuity Foundations',               edges: 7,  stage: 'DONE',        sessions: 1 },

  // ── TASK ─────────────────────────────────────────────────────────────────
  { id: 't-temper-index',    type: 'task', label: 'temper index',         fullTitle: 'Develop temper index',              edges: 6, stage: 'IN-PROGRESS' },
  { id: 't-doctype-slugs',   type: 'task', label: 'doctype slugs',        fullTitle: 'Doctype-qualified slug migration',  edges: 5, stage: 'DONE', dateStrip: 'APR 17' },
  { id: 't-graph-index',     type: 'task', label: 'graph-index',          fullTitle: 'Materialize graph-index',           edges: 4, stage: 'DONE' },
  { id: 't-svar-grid',       type: 'task', label: 'svar-grid',            fullTitle: 'Svar-grid resource table',           edges: 3, stage: 'BACKLOG' },
  { id: 't-kg-ui',           type: 'task', label: 'kg-ui',                fullTitle: 'Knowledge-graph-ui implementation', edges: 4, stage: 'IN-PROGRESS' },
  { id: 't-three-tier',      type: 'task', label: 'three-tier-metadata', fullTitle: 'Three-tier metadata system',         edges: 3, stage: 'BACKLOG' },
  { id: 't-kg-foundations',  type: 'task', label: 'kg foundations',       fullTitle: 'Knowledge-graph foundations',        edges: 5, stage: 'DONE' },
  { id: 't-open-meta',       type: 'task', label: 'open-meta',            fullTitle: 'Open-meta intentionality',           edges: 2, stage: 'BACKLOG' },

  // ── SESSION ──────────────────────────────────────────────────────────────
  // R11 D4: sessions are deferred as graph participants — they emit too many
  // weak "references" edges (every session touches ~3–6 things, always dotted,
  // never structural). We aggregate them into a per-target count rendered as
  // ⌊N⌋ SESS. marginalia on the target node. The raw session→target list is
  // preserved below in SESSION_REFERENCES for the session-index panel.

  // ── GOAL (aggregator) ────────────────────────────────────────────────────
  { id: 'g-llm-wiki',    type: 'goal', label: 'llm-wiki',         fullTitle: 'llm-wiki', edges: 6, aggregator: true },
  { id: 'g-maintenance', type: 'goal', label: 'temper-maintenance', fullTitle: 'temper-maintenance', edges: 3, aggregator: true },

  // ── CONCEPT (aggregator) ─────────────────────────────────────────────────
  { id: 'c-throughline', type: 'concept', label: 'throughline',      fullTitle: 'throughline', edges: 5, aggregator: true },
  { id: 'c-kg',          type: 'concept', label: 'knowledge-graph',  fullTitle: 'knowledge-graph', edges: 4, aggregator: true },
];

const GRAPH_EDGES = [
  // RESEARCH chain
  { source: 'r11', target: 'r7',  type: 'depends_on' },
  { source: 'r11', target: 'r9',  type: 'depends_on' },
  { source: 'r7',  target: 'r2',  type: 'depends_on' },
  { source: 'r4',  target: 'r2',  type: 'depends_on' },
  { source: 'r7',  target: 'r4',  type: 'relates_to' },

  // RESEARCH → TASK (extends)
  { source: 't-kg-ui',          target: 'r11',  type: 'extends' },
  { source: 't-graph-index',    target: 'r7',   type: 'extends' },
  { source: 't-temper-index',   target: 'r2',   type: 'extends' },
  { source: 't-doctype-slugs',  target: 'r7',   type: 'extends' },
  { source: 't-svar-grid',      target: 'r9',   type: 'relates_to' },
  { source: 't-kg-foundations', target: 'r7',   type: 'extends' },

  // TASK chain (preceded_by)
  { source: 't-doctype-slugs',  target: 't-graph-index',    type: 'preceded_by' },
  { source: 't-temper-index',   target: 't-doctype-slugs',  type: 'preceded_by' },
  { source: 't-kg-ui',          target: 't-kg-foundations', type: 'preceded_by' },

  // TASK → GOAL (aggregator)
  { source: 't-temper-index',   target: 'g-llm-wiki', type: 'relates_to' },
  { source: 't-doctype-slugs',  target: 'g-llm-wiki', type: 'relates_to' },
  { source: 't-graph-index',    target: 'g-llm-wiki', type: 'relates_to' },
  { source: 't-svar-grid',      target: 'g-llm-wiki', type: 'relates_to' },
  { source: 't-three-tier',     target: 'g-llm-wiki', type: 'relates_to' },
  { source: 'r11',              target: 'g-llm-wiki', type: 'relates_to' },
  { source: 't-kg-foundations', target: 'g-maintenance', type: 'relates_to' },
  { source: 't-open-meta',      target: 'g-maintenance', type: 'relates_to' },
  { source: 't-kg-ui',          target: 'g-maintenance', type: 'relates_to' },

  // RESEARCH → CONCEPT (aggregator)
  { source: 'r7',  target: 'c-throughline', type: 'relates_to' },
  { source: 'r4',  target: 'c-throughline', type: 'relates_to' },
  { source: 'r11', target: 'c-throughline', type: 'relates_to' },
  { source: 'r7',  target: 'c-kg',          type: 'relates_to' },
  { source: 'r11', target: 'c-kg',          type: 'relates_to' },
  { source: 'r2',  target: 'c-kg',          type: 'relates_to' },

  // SESSION → targets: not rendered as edges. R11 D4 defers these.
  // See SESSION_REFERENCES below for the raw refs (consumed as ⌊N⌋ halos).
];

// Raw session references (the deferred data). Aggregated into SESSION_COUNTS
// by the component so nodes can render ⌊N⌋ SESS. marginalia.
const SESSION_REFERENCES = [
  { session: '2026-04-13 R11 build',        date: 'APR 13', targets: ['r11', 't-kg-ui', 'r7'] },
  { session: '2026-04-15 llm-wiki planning', date: 'APR 15', targets: ['g-llm-wiki', 't-temper-index'] },
  { session: '2026-04-17 doctype slugs',    date: 'APR 17', targets: ['t-doctype-slugs', 'r7'] },
  { session: '2026-04-09 r9 svelte',        date: 'APR 09', targets: ['r9'] },
];

// Precompute session counts per target id (what the halo reads)
const SESSION_COUNTS = (() => {
  const counts = {};
  for (const s of SESSION_REFERENCES) {
    for (const t of s.targets) counts[t] = (counts[t] || 0) + 1;
  }
  return counts;
})();

// Doc-type color lookup (matches lib/graph/styling.ts + README extension for goal)
const TYPE_COLORS = {
  research: '#8cc5e2',  // steel blue
  task:     '#f0a870',  // warm ochre
  session:  '#9ed3af',  // moss green
  concept:  '#d89ccb',  // dusty pink
  goal:     '#f5d277',  // warm gold
  decision: '#c5a3e0',  // lavender (TBD)
};

// Slightly parchment-ier companion for gradient "pour" (saturated → parchment)
const TYPE_PARCHMENT = {
  research: '#e8e4df',
  task:     '#ece0d2',
  session:  '#e5e6de',
  concept:  '#e8dde4',
  goal:     '#ebe2d0',
  decision: '#e6dee8',
};

window.GRAPH_NODES = GRAPH_NODES;
window.GRAPH_EDGES = GRAPH_EDGES;
window.SESSION_REFERENCES = SESSION_REFERENCES;
window.SESSION_COUNTS = SESSION_COUNTS;
window.TYPE_COLORS = TYPE_COLORS;
window.TYPE_PARCHMENT = TYPE_PARCHMENT;
