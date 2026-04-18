// Sidebar: brand + context list + recents + user footer
// Source: _source/lib/components/Sidebar.svelte
const Sidebar = ({ activeContext, onSelectContext }) => {
  const groupStyle = {
    fontFamily: '"JetBrains Mono", monospace',
    fontSize: 9.5, letterSpacing: '0.2em', textTransform: 'uppercase',
    color: 'rgba(255,255,255,0.30)',
    padding: '8px 14px',
  };
  const itemStyle = (active) => ({
    display: 'flex', justifyContent: 'space-between', alignItems: 'center',
    padding: active ? '7px 10px 7px 10px' : '7px 14px',
    margin: '1px 8px',
    fontFamily: '"JetBrains Mono", monospace',
    fontSize: 12,
    color: active ? '#e8e4df' : 'rgba(255,255,255,0.65)',
    background: active ? 'rgba(126,184,218,0.08)' : 'transparent',
    borderLeft: active ? '2px solid #7eb8da' : '2px solid transparent',
    borderRadius: 2,
    cursor: 'pointer',
    transition: 'background 0.15s, color 0.15s',
  });
  const ctStyle = (active) => ({
    fontSize: 10,
    color: active ? '#7eb8da' : 'rgba(255,255,255,0.30)',
  });

  return (
    <aside style={{
      width: 264, flexShrink: 0,
      background: '#0c0c11',
      borderRight: '1px solid rgba(255,255,255,0.06)',
      display: 'flex', flexDirection: 'column',
      height: '100vh',
    }}>
      <div style={{ padding: '18px 16px', borderBottom: '1px solid rgba(255,255,255,0.06)' }}>
        <Wordmark size="sm" />
      </div>

      <div style={{ overflowY: 'auto', flex: 1, padding: '10px 0' }}>
        <div style={groupStyle}>Contexts</div>
        {CONTEXTS.map(c => (
          <div key={c.slug}
               onClick={() => onSelectContext(c.slug)}
               onMouseEnter={e => { if (c.slug !== activeContext) e.currentTarget.style.background = 'rgba(255,255,255,0.02)'; }}
               onMouseLeave={e => { if (c.slug !== activeContext) e.currentTarget.style.background = 'transparent'; }}
               style={itemStyle(c.slug === activeContext)}>
            <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{c.slug}</span>
            <span style={ctStyle(c.slug === activeContext)}>{c.count}</span>
          </div>
        ))}

        <div style={{ ...groupStyle, marginTop: 14 }}>Recent</div>
        {RECENTS.map(r => (
          <div key={r.title} style={itemStyle(false)}>
            <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{r.title}</span>
            <span style={ctStyle(false)}>{r.ago}</span>
          </div>
        ))}

        <div style={{ ...groupStyle, marginTop: 14 }}>Space</div>
        <div style={itemStyle(false)}>Settings</div>
        <div style={itemStyle(false)}>Teams</div>
      </div>

      <div style={{
        padding: '12px 16px',
        borderTop: '1px solid rgba(255,255,255,0.06)',
        display: 'flex', alignItems: 'center', gap: 10,
      }}>
        <div style={{
          width: 28, height: 28, borderRadius: 3,
          background: 'rgba(126,184,218,0.15)',
          color: '#7eb8da',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          fontFamily: '"JetBrains Mono", monospace', fontSize: 11, fontWeight: 500,
        }}>A</div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontFamily: 'Georgia, serif', fontSize: 13, color: '#e8e4df' }}>Alice Chen</div>
          <div style={{ fontFamily: '"JetBrains Mono", monospace', fontSize: 10, color: 'rgba(255,255,255,0.45)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>alice@acme.dev</div>
        </div>
      </div>
    </aside>
  );
};

window.Sidebar = Sidebar;
