// ============================================================================
// ResourcePeek — right-docked resource preview
// ============================================================================
// Opens when a user clicks any graph node OR a child inside an expanded
// aggregator frame. Shows doc metadata, a content excerpt, and a list of
// neighboring resources. The "explore neighborhood" action emits a CustomEvent
// the KnowledgeGraph picks up to refocus + reveal the 1-hop.
//
// Contract:
//   <ResourcePeek node={nodeData} edges={GRAPH_EDGES} onClose={} onFocus={id} />
//
// Visual grammar from the language card:
//   • Hairline 1px left border in the resource's type color at low alpha
//   • Parchment typography for titles and body
//   • Mono-cap 8–10px for metadata labels and section markers
//   • No chrome — the panel is a frame, not a card
// ============================================================================

const { useEffect: _rp_useEffect, useMemo: _rp_useMemo } = React;

const ResourcePeek = ({ node, onClose, onFocus, trail = [], onCrumbClick, width = 420, topOffset = 0 }) => {
  if (!node) return null;

  const color = window.TYPE_COLORS[node.type] || '#e8e4df';
  const parch = window.TYPE_PARCHMENT?.[node.type] || '#e8e4df';
  const content = (window.GRAPH_CONTENT || {})[node.id];
  const sessCount = (window.SESSION_COUNTS || {})[node.id] || 0;

  // Breadcrumb: resolve each trail id to a node for rendering
  const crumbs = _rp_useMemo(() => {
    const nodesById = {};
    (window.GRAPH_NODES || []).forEach(n => { nodesById[n.id] = n; });
    return (trail || [])
      .map(id => nodesById[id])
      .filter(Boolean);
  }, [trail]);
  const showCrumbs = crumbs.length > 1;

  // Compute neighbors: list every edge touching this node, rendered as
  // "direction · type · other-node". Participants-only first, aggregators
  // after — the affordance of "explore neighborhood" applies to both.
  const neighbors = _rp_useMemo(() => {
    const edges = window.GRAPH_EDGES || [];
    const nodesById = {};
    (window.GRAPH_NODES || []).forEach(n => { nodesById[n.id] = n; });
    const out = [];
    for (const e of edges) {
      if (e.source === node.id) {
        const other = nodesById[e.target];
        if (other) out.push({ id: other.id, dir: '→', type: e.type, other });
      } else if (e.target === node.id) {
        const other = nodesById[e.source];
        if (other) out.push({ id: other.id, dir: '←', type: e.type, other });
      }
    }
    // Aggregators last, by-type so similar ones cluster
    out.sort((a, b) => {
      const aAgg = !!a.other.aggregator, bAgg = !!b.other.aggregator;
      if (aAgg !== bAgg) return aAgg ? 1 : -1;
      return a.type.localeCompare(b.type);
    });
    return out;
  }, [node.id]);

  // Escape closes
  _rp_useEffect(() => {
    const handler = e => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  // Metadata rows — take from seed content, or synth minimally
  const meta = content?.meta || {
    'DOCTYPE': node.type,
    'SLUG':    `${node.type}/${node.label.replace(/\s+/g, '-')}`,
    'EDGES':   `${node.edges || '?'} touching`,
  };

  const excerpt = content?.excerpt ||
    "No content indexed for this resource yet. It appears in the graph by " +
    "virtue of its edges alone — other resources reference it, but no body " +
    "copy has been stored against its vertex.";

  return (
    <>
      {/* Faint scrim so peek doesn't float without context */}
      <div
        onClick={onClose}
        style={{
          position: 'absolute', top: 0, left: 0, bottom: 0, right: width,
          background: 'transparent',
          zIndex: 14,
        }}
      />
      <aside style={{
        position: 'absolute', top: topOffset, right: 0, bottom: 0, width,
        background: 'rgba(10,10,15,0.92)',
        backdropFilter: 'blur(8px)',
        borderLeft: `1px solid ${color}55`,
        boxShadow: '-12px 0 28px -16px rgba(0,0,0,0.6)',
        zIndex: 15,
        display: 'flex', flexDirection: 'column',
        animation: 'peekSlide 240ms cubic-bezier(0.2, 0.7, 0.2, 1)',
        overflow: 'hidden',
      }}>
        <style>{`
          @keyframes peekSlide {
            from { transform: translateX(32px); opacity: 0; }
            to   { transform: translateX(0);    opacity: 1; }
          }
        `}</style>

        {/* Header: doctype marker + close */}
        <div style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          padding: '20px 28px 12px',
          borderBottom: `1px solid rgba(255,255,255,0.04)`,
        }}>
          <div style={{
            fontFamily: '"JetBrains Mono", monospace',
            fontSize: 9, letterSpacing: '0.24em',
            color: `${color}cc`,
            textTransform: 'uppercase',
          }}>
            {node.aggregator ? 'AGGREGATOR · ' : 'PARTICIPANT · '}{node.type}
          </div>
          <button
            onClick={onClose}
            style={{
              background: 'none', border: 'none', cursor: 'pointer',
              fontFamily: '"JetBrains Mono", monospace',
              fontSize: 9, letterSpacing: '0.22em',
              color: 'rgba(255,255,255,0.4)',
              padding: 0,
            }}
            onMouseEnter={e => e.currentTarget.style.color = '#e8e4df'}
            onMouseLeave={e => e.currentTarget.style.color = 'rgba(255,255,255,0.4)'}
          >CLOSE ✕</button>
        </div>

        {/* Breadcrumb — only when drilled (trail depth > 1). Current node is
            the last crumb and is not clickable; earlier crumbs click back. */}
        {showCrumbs && (
          <div style={{
            padding: '10px 28px 0',
            fontFamily: '"JetBrains Mono", monospace',
            fontSize: 8.5, letterSpacing: '0.16em',
            color: 'rgba(255,255,255,0.38)',
            display: 'flex', alignItems: 'baseline',
            gap: 6, flexWrap: 'wrap',
          }}>
            {(() => {
              // Collapse very long trails (5+) with an ellipsis in the middle
              let visible = crumbs.map((c, i) => ({ c, i }));
              if (crumbs.length >= 5) {
                visible = [
                  { c: crumbs[0], i: 0 },
                  { ellipsis: true, jumpTo: crumbs.length - 3 },
                  { c: crumbs[crumbs.length - 2], i: crumbs.length - 2 },
                  { c: crumbs[crumbs.length - 1], i: crumbs.length - 1 },
                ];
              }
              return visible.map((item, idx) => {
                if (item.ellipsis) {
                  return (
                    <React.Fragment key={`ellip-${idx}`}>
                      <span style={{ opacity: 0.5 }}>…</span>
                      <span style={{ opacity: 0.3 }}>›</span>
                    </React.Fragment>
                  );
                }
                const { c, i } = item;
                const isLast = i === crumbs.length - 1;
                const cColor = window.TYPE_COLORS[c.type] || '#e8e4df';
                return (
                  <React.Fragment key={`${c.id}-${i}`}>
                    {idx > 0 && <span style={{ opacity: 0.3 }}>›</span>}
                    {isLast ? (
                      <span style={{
                        color: `${cColor}cc`,
                        fontStyle: c.aggregator ? 'italic' : 'normal',
                        fontFamily: c.aggregator ? '"Source Serif 4", Georgia, serif' : '"JetBrains Mono", monospace',
                        fontSize: c.aggregator ? 11 : 8.5,
                        letterSpacing: c.aggregator ? 0 : '0.16em',
                      }}>{c.label}</span>
                    ) : (
                      <button
                        onClick={() => onCrumbClick?.(i)}
                        title={`Back to ${c.fullTitle || c.label}`}
                        style={{
                          background: 'none', border: 'none', padding: 0,
                          cursor: 'pointer',
                          fontFamily: c.aggregator ? '"Source Serif 4", Georgia, serif' : '"JetBrains Mono", monospace',
                          fontSize: c.aggregator ? 11 : 8.5,
                          fontStyle: c.aggregator ? 'italic' : 'normal',
                          letterSpacing: c.aggregator ? 0 : '0.16em',
                          color: `${cColor}88`,
                          transition: 'color 140ms',
                        }}
                        onMouseEnter={e => e.currentTarget.style.color = cColor}
                        onMouseLeave={e => e.currentTarget.style.color = `${cColor}88`}
                      >{c.label}</button>
                    )}
                  </React.Fragment>
                );
              });
            })()}
          </div>
        )}

        {/* Title */}
        <div style={{ padding: '18px 28px 12px' }}>
          <h2 style={{
            margin: 0,
            fontFamily: '"Source Serif 4", Georgia, serif',
            fontStyle: node.aggregator ? 'italic' : 'normal',
            fontWeight: 400,
            fontSize: 28, lineHeight: 1.15,
            color: color,
            letterSpacing: '-0.005em',
          }}>{node.fullTitle || node.label}</h2>
          {sessCount > 0 && (
            <div style={{
              marginTop: 10,
              fontFamily: '"JetBrains Mono", monospace',
              fontSize: 9, letterSpacing: '0.2em',
              color: '#9ed3af',
            }}>⌊{sessCount}⌋ SESSION{sessCount === 1 ? '' : 'S'} · ANNOTATION</div>
          )}
        </div>

        {/* Scrollable body */}
        <div style={{
          flex: 1, overflowY: 'auto', padding: '8px 28px 28px',
        }}>
          {/* Neighbors — placed first as primary navigation affordance */}
          {neighbors.length > 0 && (
            <>
              <div style={{
                display: 'flex', alignItems: 'baseline', justifyContent: 'space-between',
                marginBottom: 12,
              }}>
                <div style={{
                  fontFamily: '"JetBrains Mono", monospace',
                  fontSize: 8.5, letterSpacing: '0.22em',
                  color: 'rgba(255,255,255,0.35)',
                }}>{node.aggregator ? 'MEMBERS' : 'NEIGHBORS'} · {neighbors.length}</div>
                <button
                  onClick={() => onFocus?.(node.id)}
                  style={{
                    background: 'none',
                    border: `1px solid ${color}44`,
                    color: color,
                    padding: '4px 10px',
                    cursor: 'pointer',
                    fontFamily: '"JetBrains Mono", monospace',
                    fontSize: 8.5, letterSpacing: '0.2em',
                    textTransform: 'uppercase',
                    transition: 'background 160ms',
                  }}
                  onMouseEnter={e => e.currentTarget.style.background = `${color}18`}
                  onMouseLeave={e => e.currentTarget.style.background = 'transparent'}
                >Explore neighborhood →</button>
              </div>

              <div style={{ marginBottom: 24 }}>
                {neighbors.map((n, i) => {
                  const nColor = window.TYPE_COLORS[n.other.type];
                  return (
                    <button
                      key={`${n.id}-${i}`}
                      onClick={() => onFocus?.(n.other.id, { openPeek: true })}
                      style={{
                        display: 'grid',
                        gridTemplateColumns: '18px 72px 1fr',
                        alignItems: 'baseline',
                        gap: 10,
                        width: '100%',
                        textAlign: 'left',
                        background: 'none', border: 'none',
                        padding: '8px 0',
                        borderBottom: '1px solid rgba(255,255,255,0.04)',
                        cursor: 'pointer',
                        transition: 'background 120ms',
                      }}
                      onMouseEnter={e => {
                        e.currentTarget.style.background = 'rgba(255,255,255,0.02)';
                      }}
                      onMouseLeave={e => {
                        e.currentTarget.style.background = 'none';
                      }}
                    >
                      <span style={{
                        fontFamily: '"JetBrains Mono", monospace',
                        fontSize: 12,
                        color: 'rgba(255,255,255,0.3)',
                      }}>{n.dir}</span>
                      <span style={{
                        fontFamily: '"JetBrains Mono", monospace',
                        fontSize: 8, letterSpacing: '0.2em',
                        color: 'rgba(255,255,255,0.4)',
                      }}>{(n.type || '').toUpperCase().replace('_', ' ')}</span>
                      <span style={{
                        fontFamily: '"Source Serif 4", Georgia, serif',
                        fontStyle: n.other.aggregator ? 'italic' : 'normal',
                        fontSize: 13,
                        color: nColor,
                      }}>{n.other.fullTitle || n.other.label}</span>
                    </button>
                  );
                })}
              </div>
            </>
          )}

          {/* Metadata rows */}
          <div style={{
            display: 'grid', gridTemplateColumns: '90px 1fr',
            rowGap: 8, columnGap: 14,
            fontFamily: '"JetBrains Mono", monospace',
            fontSize: 10, lineHeight: 1.5,
            marginBottom: 22,
            paddingTop: 18,
            borderTop: '1px solid rgba(255,255,255,0.05)',
          }}>
            {Object.entries(meta).map(([k, v]) => (
              <React.Fragment key={k}>
                <div style={{
                  letterSpacing: '0.2em',
                  color: 'rgba(255,255,255,0.35)',
                }}>{k}</div>
                <div style={{
                  color: '#e8e4df',
                  fontFamily: k === 'SLUG'
                    ? '"JetBrains Mono", monospace'
                    : 'inherit',
                }}>{v}</div>
              </React.Fragment>
            ))}
          </div>

          {/* Excerpt */}
          <div style={{
            fontFamily: '"JetBrains Mono", monospace',
            fontSize: 8.5, letterSpacing: '0.22em',
            color: 'rgba(255,255,255,0.35)',
            marginBottom: 10,
          }}>EXCERPT</div>
          <p style={{
            fontFamily: '"Source Serif 4", Georgia, serif',
            fontSize: 14, lineHeight: 1.6,
            color: 'rgba(232,228,223,0.88)',
            margin: 0,
            marginBottom: 18,
            textWrap: 'pretty',
          }}>{excerpt}</p>
        </div>

        {/* Footer: link to full resource view */}
        <div style={{
          padding: '14px 28px',
          borderTop: '1px solid rgba(255,255,255,0.06)',
          display: 'flex', justifyContent: 'space-between', alignItems: 'center',
          fontFamily: '"JetBrains Mono", monospace',
          fontSize: 9, letterSpacing: '0.2em',
        }}>
          <span style={{ color: 'rgba(255,255,255,0.3)' }}>ESC · CLOSE</span>
          <a
            href="#"
            onClick={e => e.preventDefault()}
            style={{
              color: color, textDecoration: 'none',
              borderBottom: `1px solid ${color}55`,
              paddingBottom: 1,
            }}
          >OPEN RESOURCE →</a>
        </div>
      </aside>
    </>
  );
};

window.ResourcePeek = ResourcePeek;
