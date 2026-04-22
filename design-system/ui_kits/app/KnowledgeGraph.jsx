// ============================================================================
// Temper Knowledge Graph — Cytoscape prototype
// ============================================================================
// Visual grammar recorded in README.md → "Knowledge graph — visual language":
//   • Participants (research/task/session) are typeset words, 13px serif.
//   • Aggregators (goal/concept) are larger italic serif, higher simulation
//     mass pulls children close, faint radial wash hints at gravity.
//   • Edges hairline; stroke-dash encodes semantic.
//   • On hover: incident edges brighten, rest dim.
//   • Aggregator click → transient expand panel (planning view).
// ============================================================================

const { useEffect, useRef, useState, useMemo } = React;

const EDGE_LABELS = {
  depends_on:  'DEPENDS',
  extends:     'EXT. BY',
  preceded_by: 'AFTER',
  relates_to:  'RELATES',
  references:  'REFS',
};

function labelFill(type) {
  return window.TYPE_COLORS[type] || '#e8e4df';
}

// ── Component ────────────────────────────────────────────────────────────
const KnowledgeGraph = ({ contextName = 'temper' }) => {
  const containerRef = useRef(null);
  const cyRef = useRef(null);
  const [hoveredId, setHoveredId] = useState(null);
  const [peekTrail, setPeekTrail] = useState([]);
  const [mode, setMode] = useState('structural');

  const elements = useMemo(() => {
    const nodes = window.GRAPH_NODES.map(n => ({
      group: 'nodes',
      data: {
        id: n.id,
        type: n.type,
        aggregator: !!n.aggregator,
        label: n.label,
        fullTitle: n.fullTitle,
        edges: n.edges,
        stage: n.stage || '',
        sessions: (window.SESSION_COUNTS && window.SESSION_COUNTS[n.id]) || 0,
        dateStrip: n.dateStrip || '',
        // Precomputed style data
        fontSize: n.aggregator ? 19 : (n.edges >= 10 ? 14 : 13),
        widthPx: n.aggregator
          ? Math.max(180, n.label.length * 12) + 40
          : Math.max(60, n.label.length * 8),
        heightPx: n.aggregator ? 70 : 22,
        fill: labelFill(n.type),
      },
      classes: [
        `type-${n.type}`,
        n.aggregator ? 'aggregator' : 'participant',
      ].join(' '),
    }));
    const edges = window.GRAPH_EDGES.map((e, i) => ({
      group: 'edges',
      data: {
        id: `e${i}`,
        source: e.source,
        target: e.target,
        type: e.type,
        sourceFill: labelFill(
          (window.GRAPH_NODES.find(n => n.id === e.source) || {}).type
        ),
      },
      classes: `etype-${e.type}`,
    }));
    return [...nodes, ...edges];
  }, []);

  useEffect(() => {
    if (!containerRef.current || !window.cytoscape) return;

    const cy = window.cytoscape({
      container: containerRef.current,
      elements,
      minZoom: 0.25,
      maxZoom: 3,
      wheelSensitivity: 0.2,
      style: [
        // ── Base node: word IS the node ────────────────────────────────
        {
          selector: 'node',
          style: {
            'label': 'data(label)',
            'color': 'data(fill)',
            'font-family': '"Source Serif 4", Georgia, serif',
            'font-size': 'data(fontSize)',
            'font-weight': 500,
            'text-halign': 'center',
            'text-valign': 'center',
            'text-wrap': 'none',
            'background-opacity': 0,
            'border-width': 0,
            'width': 'data(widthPx)',
            'height': 'data(heightPx)',
            'text-events': 'yes',
            'overlay-opacity': 0,
          },
        },
        // Aggregator: larger italic serif + soft radial-ish bg (approximated via
        // a large background-opacity ellipse). Cytoscape does not do gradients
        // so we use a semi-transparent fill on an oversized ellipse.
        {
          selector: 'node.aggregator',
          style: {
            'shape': 'ellipse',
            'background-color': 'data(fill)',
            'background-opacity': 0.05,
            'width': 260,
            'height': 140,
            'font-size': 19,
            'font-style': 'italic',
            'font-weight': 600,
          },
        },
        // Hovered: text-background highlight in source color
        {
          selector: 'node.hovered',
          style: {
            'text-background-color': 'data(fill)',
            'text-background-opacity': 0.12,
            'text-background-padding': 4,
          },
        },
        // Dimmed when something else is hovered
        {
          selector: 'node.dim',
          style: {
            'text-opacity': 0.25,
          },
        },

        // ── Edges ──────────────────────────────────────────────────────
        {
          selector: 'edge',
          style: {
            'width': 0.75,
            'line-color': 'rgba(255,255,255,0.10)',
            'curve-style': 'straight',
            'target-arrow-shape': 'none',
            'opacity': 1,
          },
        },
        { selector: 'edge.etype-depends_on',  style: { 'line-style': 'solid' } },
        { selector: 'edge.etype-extends',     style: { 'line-style': 'solid' } },
        { selector: 'edge.etype-preceded_by', style: { 'line-style': 'dashed', 'line-dash-pattern': [6, 4] } },
        { selector: 'edge.etype-relates_to',  style: { 'line-style': 'dashed', 'line-dash-pattern': [3, 3] } },
        { selector: 'edge.etype-references',  style: { 'line-style': 'dotted' } },
        {
          selector: 'edge.incident',
          style: {
            'width': 1.2,
            'line-color': 'data(sourceFill)',
            'opacity': 0.85,
          },
        },
        {
          selector: 'edge.quiet',
          style: {
            'line-color': 'rgba(255,255,255,0.04)',
          },
        },
      ],
      layout: {
        name: 'fcose',
        animate: false,
        fit: true,
        padding: 100,
        randomize: true,
        // Cluster separation strategy: large ideal edge length + per-edge
        // length multiplier via data. fcose reads `data.idealEdgeLength` per
        // edge when present (and falls back to the layout's base value).
        idealEdgeLength: 180,
        nodeRepulsion: 25000,
        edgeElasticity: 0.35,
        gravity: 0.15,
        gravityRange: 5.0,
        gravityCompound: 1.2,
        numIter: 3500,
        tile: false,
        nodeSeparation: 180,
        packComponents: true,
        quality: 'proof',
        // Favor a wider aspect to match our canvas
        aspectRatio: 1.8,
      },
    });

    cyRef.current = cy;
    window.cyDebug = cy;

    // Force a paint after layout completes. Without this, fcose sometimes
    // finishes in a "quiescent" state and the canvas reads alpha-0 until a
    // user interaction triggers a render.
    const forcePaint = () => {
      requestAnimationFrame(() => {
        cy.fit(undefined, 100);
        cy.forceRender?.();
      });
    };
    cy.one('layoutstop', forcePaint);
    // Also force one immediately in case layoutstop already fired synchronously.
    setTimeout(forcePaint, 50);
    setTimeout(forcePaint, 300);

    // Interactions
    cy.on('mouseover', 'node', evt => {
      const node = evt.target;
      setHoveredId(node.id());
      const neighborhood = node.closedNeighborhood();
      cy.nodes().not(neighborhood).addClass('dim');
      cy.nodes().removeClass('hovered');
      node.addClass('hovered');
      cy.edges().forEach(e => {
        if (e.source().id() === node.id() || e.target().id() === node.id()) {
          e.addClass('incident').removeClass('quiet');
        } else {
          e.addClass('quiet').removeClass('incident');
        }
      });
    });

    cy.on('mouseout', 'node', () => {
      setHoveredId(null);
      cy.nodes().removeClass('dim hovered');
      cy.edges().removeClass('incident quiet');
    });

    cy.on('tap', 'node', evt => {
      const node = evt.target;
      // Fresh click on graph = fresh trail with this node as root
      setPeekTrail([node.id()]);
    });

    cy.on('tap', evt => {
      if (evt.target === cy) {
        setPeekTrail([]);
      }
    });

    return () => cy.destroy();
  }, [elements]);

  const hoveredNode = useMemo(() => {
    if (!hoveredId) return null;
    return window.GRAPH_NODES.find(n => n.id === hoveredId);
  }, [hoveredId]);

  const hoveredRelationships = useMemo(() => {
    if (!hoveredId) return [];
    const out = [];
    window.GRAPH_EDGES.forEach(e => {
      if (e.source === hoveredId) {
        const target = window.GRAPH_NODES.find(n => n.id === e.target);
        if (target) out.push({ dir: 'out', type: e.type, other: target });
      } else if (e.target === hoveredId) {
        const source = window.GRAPH_NODES.find(n => n.id === e.source);
        if (source) out.push({ dir: 'in', type: e.type, other: source });
      }
    });
    return out.slice(0, 7);
  }, [hoveredId]);

  return (
    <div style={{
      position: 'relative', width: '100%', height: '100%',
      background: '#0a0a0f', overflow: 'hidden',
    }}>
      {/* Watermark */}
      <div style={{
        position: 'absolute', left: 32, bottom: 24,
        fontFamily: '"Source Serif 4", Georgia, serif',
        fontSize: 88, fontStyle: 'italic',
        color: 'rgba(255,255,255,0.035)',
        letterSpacing: '-0.02em', lineHeight: 1,
        pointerEvents: 'none', zIndex: 0, userSelect: 'none',
      }}>
        <span style={{
          fontFamily: '"JetBrains Mono", monospace',
          fontSize: 14, fontStyle: 'normal', letterSpacing: '0.22em',
          verticalAlign: 'middle', marginRight: 14, opacity: 0.7,
        }}>CONTEXT</span>
        <em>{contextName}</em>
      </div>

      {/* Top chrome */}
      <div style={{
        position: 'absolute', top: 0, left: 0,
        padding: '16px 20px', zIndex: 5, pointerEvents: 'none',
      }}>
        <div style={{ pointerEvents: 'auto' }}>
          <div style={{
            fontFamily: '"JetBrains Mono", monospace',
            fontSize: 9, letterSpacing: '0.22em',
            color: 'rgba(255,255,255,0.38)', marginBottom: 6,
          }}>VIEW</div>
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 14 }}>
            <ModeWord active={mode === 'structural'} onClick={() => setMode('structural')}>structural</ModeWord>
            <ModeWord active={mode === 'meta-doc'}   onClick={() => setMode('meta-doc')}>meta-doc</ModeWord>
          </div>
          {mode === 'meta-doc' && (
            <div style={{
              marginTop: 8,
              fontFamily: '"Source Serif 4", Georgia, serif',
              fontStyle: 'italic', fontSize: 11,
              color: 'rgba(255,255,255,0.5)',
              maxWidth: 240,
            }}>
              Emergent view — not implemented in this prototype yet.
            </div>
          )}
        </div>
      </div>

      {/* Legend — standalone panel, stays visible when peek docks */}
      <div style={{
        position: 'absolute', top: 16, right: 16,
        padding: '14px 16px 12px',
        background: 'rgba(10,10,15,0.88)',
        backdropFilter: 'blur(6px)',
        border: '1px solid rgba(255,255,255,0.06)',
        fontFamily: '"JetBrains Mono", monospace',
        fontSize: 8.5, letterSpacing: '0.18em',
        color: 'rgba(255,255,255,0.45)', textAlign: 'right', lineHeight: 1.9,
        zIndex: 16,
        pointerEvents: 'auto',
      }}>
        <LegendRow color="#8cc5e2" label="RESEARCH" />
        <LegendRow color="#f0a870" label="TASK" />
        <LegendRow color="#f5d277" label="GOAL · italic" />
        <LegendRow color="#d89ccb" label="CONCEPT · italic" />
        <div style={{
          marginTop: 10,
          paddingTop: 8,
          borderTop: '1px solid rgba(255,255,255,0.06)',
          fontSize: 7.5,
          color: '#9ed3af',
          letterSpacing: '0.14em',
          fontStyle: 'normal',
          textAlign: 'right',
        }}>
          ⌊N⌋ SESSIONS · ANNOTATION, NOT EDGE
        </div>
        <div style={{ marginTop: 10, opacity: 0.6, fontSize: 7.5 }}>
          — CLICK ANY NODE TO PEEK —
        </div>
      </div>

      {/* Cytoscape canvas */}
      <div
        ref={containerRef}
        style={{ position: 'absolute', inset: 0, zIndex: 1 }}
      />

      {/* Hover inspector */}
      {hoveredNode && (
        <HoverInspector node={hoveredNode} relationships={hoveredRelationships} />
      )}

      {/* Right-docked resource preview */}
      {peekTrail.length > 0 && (() => {
        const currentId = peekTrail[peekTrail.length - 1];
        const node = window.GRAPH_NODES.find(n => n.id === currentId);
        if (!node) return null;
        return (
          <window.ResourcePeek
            node={node}
            trail={peekTrail}
            topOffset={168}
            onClose={() => setPeekTrail([])}
            onCrumbClick={(i) => {
              // Click a breadcrumb: slice trail back to that depth
              setPeekTrail(peekTrail.slice(0, i + 1));
              // Also recenter the camera on that node
              const target = cyRef.current?.$id(peekTrail[i]);
              if (target && target.length) {
                cyRef.current.animate({
                  center: { eles: target },
                  zoom: Math.max(cyRef.current.zoom(), 0.9),
                  duration: 380,
                  easing: 'ease-in-out',
                });
              }
            }}
            onFocus={(id, opts = {}) => {
              // For now: recenter camera on focus target.
              // Step 4 will add 1-hop neighborhood reveal.
              const target = cyRef.current?.$id(id);
              if (target && target.length) {
                cyRef.current.animate({
                  center: { eles: target },
                  zoom: Math.max(cyRef.current.zoom(), 0.9),
                  duration: 380,
                  easing: 'ease-out-cubic',
                });
              }
              if (opts.openPeek) {
                // Drilling deeper: push onto trail
                setPeekTrail([...peekTrail, id]);
              }
            }}
          />
        );
      })()}
    </div>
  );
};

// ── Subcomponents ────────────────────────────────────────────────────────
const ModeWord = ({ active, onClick, children }) => (
  <span
    onClick={onClick}
    style={{
      cursor: 'pointer',
      fontFamily: active ? '"Source Serif 4", Georgia, serif' : '"JetBrains Mono", monospace',
      fontSize: active ? 14 : 10,
      letterSpacing: active ? 0 : '0.18em',
      textTransform: active ? 'none' : 'uppercase',
      color: active ? '#e8e4df' : 'rgba(255,255,255,0.35)',
      transition: 'color 220ms',
    }}
  >{children}</span>
);

const LegendRow = ({ color, label }) => (
  <div>
    <span style={{
      display: 'inline-block', width: 20, height: 5,
      background: color, marginRight: 8, verticalAlign: 'middle',
    }}/>{label}
  </div>
);

const HoverInspector = ({ node, relationships }) => {
  const fill = labelFill(node.type);
  return (
    <div style={{
      position: 'absolute', top: 90, right: 20, width: 240,
      padding: '14px 16px',
      borderLeft: `1px solid ${fill}66`,
      background: 'rgba(12,12,17,0.85)',
      backdropFilter: 'blur(6px)',
      zIndex: 10, pointerEvents: 'none',
    }}>
      <div style={{
        fontFamily: '"JetBrains Mono", monospace',
        fontSize: 8.5, letterSpacing: '0.22em',
        color: `${fill}d0`,
        marginBottom: 8,
      }}>
        {node.type.toUpperCase()}{node.aggregator ? ' · AGGREGATOR' : ''}
      </div>
      <div style={{
        fontFamily: '"Source Serif 4", Georgia, serif',
        fontSize: 14, fontWeight: 500, color: '#e8e4df',
        marginBottom: 12, lineHeight: 1.3,
      }}>{node.fullTitle}</div>
      {relationships.map((r, i) => (
        <div key={i} style={{
          fontFamily: '"Source Serif 4", Georgia, serif',
          fontSize: 11.5, color: 'rgba(255,255,255,0.72)',
          marginBottom: 5, lineHeight: 1.4,
        }}>
          <em style={{
            fontFamily: '"JetBrains Mono", monospace',
            fontStyle: 'normal', fontSize: 8.5, letterSpacing: '0.18em',
            color: 'rgba(255,255,255,0.38)', marginRight: 8,
          }}>{EDGE_LABELS[r.type] || r.type.toUpperCase()}</em>
          {r.other.label}
        </div>
      ))}
      {node.sessions > 0 && (
        <div style={{
          fontFamily: '"JetBrains Mono", monospace',
          fontSize: 9, letterSpacing: '0.18em',
          color: 'rgba(158,211,175,0.75)',
          marginTop: 10, paddingTop: 10,
          borderTop: '1px solid rgba(255,255,255,0.06)',
        }}>
          ⌊{node.sessions}⌋ SESSIONS REFERENCE THIS
        </div>
      )}
      {node.aggregator && (
        <div style={{
          marginTop: 10, paddingTop: 10,
          borderTop: '1px solid rgba(255,255,255,0.06)',
          fontFamily: '"Source Serif 4", Georgia, serif',
          fontStyle: 'italic', fontSize: 11,
          color: 'rgba(255,255,255,0.48)',
        }}>
          Click to expand children into a planning panel.
        </div>
      )}
    </div>
  );
};

window.KnowledgeGraph = KnowledgeGraph;
