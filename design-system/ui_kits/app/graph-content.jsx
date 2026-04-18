// ============================================================================
// Temper KG — per-resource doc metadata + content slice
// ============================================================================
// Seed data so the ResourcePeek panel reads like a real vault preview. Keyed
// by node id (matches graph-data.jsx). Each entry has:
//   • meta    — structured fields that render as the header rows
//   • excerpt — a short prose slice; real content in the app is markdown/ast
//   • wouldLink — 3–6 outbound links (id + relationship) shown as next-hop
// ============================================================================

/* eslint-disable no-unused-vars */
const GRAPH_CONTENT = {
  // ── RESEARCH ──────────────────────────────────────────────────────────────
  'r2': {
    meta: {
      'DOCTYPE': 'research',
      'SLUG':    'research/r2-data-model-foundations',
      'STAGE':   'DONE · JAN 2026',
      'OWNER':   'temper',
      'EDGES':   '11 in · 4 out',
    },
    excerpt:
      "The foundational research establishing the vertex-edge model for the temper vault. " +
      "Every durable artifact in the system is a vertex; every declared relationship is an " +
      "edge with a semantic type. Doctypes gate what edges are permitted to emit. This " +
      "document frozen the core invariants that R7, R9, and R11 all build on.",
  },
  'r7': {
    meta: {
      'DOCTYPE': 'research',
      'SLUG':    'research/r7-vertex-edge-knowledge-graph',
      'STAGE':   'DONE · FEB 2026',
      'OWNER':   'temper',
      'EDGES':   '18 in · 3 out',
    },
    excerpt:
      "Extends R2 into a full graph substrate. Defines the eight canonical edge types " +
      "(depends_on, extends, preceded_by, relates_to, references, parent_of, authored_by, " +
      "owned_by) and establishes that owner boundaries are absolute — edges never cross " +
      "contexts. This is the graph kernel everything else sits on.",
  },
  'r9': {
    meta: {
      'DOCTYPE': 'research',
      'SLUG':    'research/r9-svelte-ui-design',
      'STAGE':   'DONE · MAR 2026',
      'OWNER':   'temper',
      'EDGES':   '9 in · 2 out',
    },
    excerpt:
      "The UI design principles for the temper frontend: muted dark ground, Source Serif 4 " +
      "as the voice, hairline rules over chrome. Establishes the \"quiet instrument\" visual " +
      "doctrine — the word holds the color, the thread is a hairline, restraint is the point.",
  },
  'r11': {
    meta: {
      'DOCTYPE': 'research',
      'SLUG':    'research/r11-visualization-design',
      'STAGE':   'IN-PROGRESS · APR 2026',
      'OWNER':   'temper',
      'EDGES':   '5 in · 2 out',
    },
    excerpt:
      "The active design pass for the knowledge-graph visualization. Introduces the " +
      "participant/aggregator split (D1), the two-mode system of structural vs meta-doc (D2), " +
      "the Jaccard-based emergent edges for meta-doc projection (D3), and defers sessions " +
      "as graph participants in favor of annotation halos (D4).",
  },
  'r4': {
    meta: {
      'DOCTYPE': 'research',
      'SLUG':    'research/r4-session-continuity-foundations',
      'STAGE':   'DONE · FEB 2026',
      'OWNER':   'temper',
      'EDGES':   '7 in · 1 out',
    },
    excerpt:
      "Establishes how sessions as annotations preserve continuity across fragmented work " +
      "without becoming first-class graph participants. The key insight that later informs " +
      "R11-D4: sessions emit too many weak reference edges to be structural.",
  },

  // ── TASK ──────────────────────────────────────────────────────────────────
  't-temper-index': {
    meta: {
      'DOCTYPE':  'task',
      'SLUG':     'task/temper-index',
      'STAGE':    'IN-PROGRESS',
      'ASSIGNEE': '—',
      'EDGES':    '6 touching',
    },
    excerpt:
      "Build the searchable surface index over all temper vertices. Must respect owner " +
      "boundaries, emit doctype-qualified slugs (per T/doctype-slugs), and feed the kg-ui " +
      "autocomplete. Blocks llm-wiki assembly.",
  },
  't-doctype-slugs': {
    meta: {
      'DOCTYPE':  'task',
      'SLUG':     'task/doctype-qualified-slugs',
      'STAGE':    'DONE · APR 17',
      'ASSIGNEE': '—',
      'EDGES':    '5 touching',
    },
    excerpt:
      "Migrate all vertex ids to doctype-qualified form (e.g. research/r7 not r7). Removes " +
      "ambiguity when goal/temper-cloud and concept/temper-cloud both exist. Completed APR 17.",
  },
  't-graph-index': {
    meta: {
      'DOCTYPE': 'task',
      'SLUG':    'task/materialize-graph-index',
      'STAGE':   'DONE',
      'EDGES':   '4 touching',
    },
    excerpt:
      "Materialize the full edge list into a denormalized graph-index table for fast " +
      "neighborhood queries. Feeds both the visualization and the session-continuity tools.",
  },
  't-svar-grid': {
    meta: {
      'DOCTYPE': 'task',
      'SLUG':    'task/svar-grid-resource-table',
      'STAGE':   'BACKLOG',
      'EDGES':   '3 touching',
    },
    excerpt:
      "Replace the vault grid with a Svar-backed virtualized table supporting 10k+ rows. " +
      "Preserves the current hairline-divider aesthetic. Blocked by temper-index.",
  },
  't-kg-ui': {
    meta: {
      'DOCTYPE': 'task',
      'SLUG':    'task/knowledge-graph-ui',
      'STAGE':   'IN-PROGRESS',
      'EDGES':   '4 touching',
    },
    excerpt:
      "The Cytoscape-based graph visualization you are currently looking at. Implements the " +
      "R11 participant/aggregator distinction and the two-mode toggle.",
  },
  't-three-tier': {
    meta: {
      'DOCTYPE': 'task',
      'SLUG':    'task/three-tier-metadata',
      'STAGE':   'BACKLOG',
      'EDGES':   '3 touching',
    },
    excerpt:
      "Structured / freeform / computed metadata layers on every vertex. Structured is " +
      "schema-enforced, freeform is key-value, computed is derived on read. Informs the " +
      "ResourcePeek header this panel itself is rendering.",
  },
  't-kg-foundations': {
    meta: {
      'DOCTYPE': 'task',
      'SLUG':    'task/kg-foundations',
      'STAGE':   'DONE',
      'EDGES':   '5 touching',
    },
    excerpt:
      "Ship the minimal graph kernel: vertex CRUD, edge declaration, doctype registry, " +
      "owner boundary enforcement. The pre-work that made kg-ui possible.",
  },
  't-open-meta': {
    meta: {
      'DOCTYPE': 'task',
      'SLUG':    'task/open-meta-intentionality',
      'STAGE':   'BACKLOG',
      'EDGES':   '2 touching',
    },
    excerpt:
      "Extend the freeform metadata tier to allow user-declared semantics that the system " +
      "learns to type over time. Lowest priority in the temper-maintenance cluster.",
  },

  // ── GOAL (aggregator) ─────────────────────────────────────────────────────
  'g-llm-wiki': {
    meta: {
      'DOCTYPE': 'goal',
      'SLUG':    'goal/llm-wiki',
      'STAGE':   'IN-PROGRESS',
      'EDGES':   '6 members',
    },
    excerpt:
      "The flagship goal of the current quarter: ship an LLM-assisted wiki layer over the " +
      "vault. Assembles temper-index, doctype-slugs, graph-index, svar-grid, three-tier, " +
      "and R11 visualization into a single coherent surface.",
  },
  'g-maintenance': {
    meta: {
      'DOCTYPE': 'goal',
      'SLUG':    'goal/temper-maintenance',
      'STAGE':   'ONGOING',
      'EDGES':   '3 members',
    },
    excerpt:
      "Ongoing health and hygiene of the temper substrate itself. Catches kg-foundations, " +
      "open-meta, kg-ui — the meta-work that keeps the vault navigable.",
  },

  // ── CONCEPT (aggregator) ──────────────────────────────────────────────────
  'c-throughline': {
    meta: {
      'DOCTYPE': 'concept',
      'SLUG':    'concept/throughline',
      'EDGES':   '5 members',
    },
    excerpt:
      "The connective tissue of continuity across research chains. R7 → R4 → R11 all cite " +
      "throughline as the organizing metaphor: what persists between sessions, what a vault " +
      "remembers about its own evolution. More philosophical than structural.",
  },
  'c-kg': {
    meta: {
      'DOCTYPE': 'concept',
      'SLUG':    'concept/knowledge-graph',
      'EDGES':   '4 members',
    },
    excerpt:
      "The concept node that anchors all research about the graph itself — R7, R11, R2 all " +
      "register here. Not a document with content, but a locus of attention.",
  },
};

window.GRAPH_CONTENT = GRAPH_CONTENT;
