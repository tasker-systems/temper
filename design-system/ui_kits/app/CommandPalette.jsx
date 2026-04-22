// Command palette. Source: _source/lib/components/CommandPalette.svelte
const CommandPalette = ({ open, onClose, onPick }) => {
  const [q, setQ] = React.useState('');
  const [idx, setIdx] = React.useState(0);
  const inputRef = React.useRef(null);

  const all = Object.values(RESOURCES).flat();
  const results = q
    ? all.filter(r => r.title.toLowerCase().includes(q.toLowerCase())).slice(0, 6)
    : all.slice(0, 6);

  React.useEffect(() => {
    if (open) { setQ(''); setIdx(0); setTimeout(() => inputRef.current?.focus(), 10); }
  }, [open]);

  React.useEffect(() => { setIdx(0); }, [q]);

  if (!open) return null;

  const onKey = (e) => {
    if (e.key === 'Escape') onClose();
    if (e.key === 'ArrowDown') { e.preventDefault(); setIdx(i => Math.min(results.length - 1, i + 1)); }
    if (e.key === 'ArrowUp')   { e.preventDefault(); setIdx(i => Math.max(0, i - 1)); }
    if (e.key === 'Enter' && results[idx]) { onPick(results[idx]); }
  };

  return (
    <div onClick={onClose} style={{
      position: 'fixed', inset: 0, zIndex: 100,
      background: 'rgba(0,0,0,0.55)',
      backdropFilter: 'blur(3px)',
      display: 'flex', justifyContent: 'center', alignItems: 'flex-start',
      paddingTop: '14vh',
    }}>
      <div onClick={e => e.stopPropagation()} style={{
        width: 560, maxWidth: '90vw',
        background: '#12121a',
        border: '1px solid rgba(255,255,255,0.1)',
        borderRadius: 6,
        boxShadow: '0 24px 80px rgba(0,0,0,0.7)',
        overflow: 'hidden',
      }}>
        <input
          ref={inputRef}
          value={q}
          onChange={e => setQ(e.target.value)}
          onKeyDown={onKey}
          placeholder="Search resources by meaning or name…"
          style={{
            width: '100%', border: 'none', outline: 'none',
            padding: '14px 18px',
            background: 'transparent',
            fontFamily: '"JetBrains Mono", monospace',
            fontSize: 13.5,
            color: '#e8e4df',
            borderBottom: '1px solid rgba(255,255,255,0.06)',
          }}
        />
        <div style={{ maxHeight: 360, overflowY: 'auto' }}>
          {results.length === 0 && (
            <div style={{ padding: '18px 20px', fontFamily: 'Georgia, serif', fontStyle: 'italic', color: 'rgba(255,255,255,0.45)', fontSize: 13 }}>
              No matches. Try a different query.
            </div>
          )}
          {results.map((r, i) => (
            <div key={r.id}
                 onClick={() => onPick(r)}
                 onMouseEnter={() => setIdx(i)}
                 style={{
                   display: 'flex', justifyContent: 'space-between', alignItems: 'center',
                   padding: '10px 18px',
                   background: i === idx ? 'rgba(126,184,218,0.08)' : 'transparent',
                   borderLeft: i === idx ? '2px solid #7eb8da' : '2px solid transparent',
                   paddingLeft: i === idx ? 16 : 18,
                   cursor: 'pointer',
                 }}>
              <span style={{
                fontFamily: 'Georgia, serif', fontSize: 14,
                color: i === idx ? '#e8e4df' : 'rgba(255,255,255,0.65)',
              }}>{r.title}</span>
              <span style={{
                fontFamily: '"JetBrains Mono", monospace',
                fontSize: 9.5, letterSpacing: '0.18em',
                color: KIND_COLOR[r.kind] ?? '#7eb8da',
              }}>{r.kind}</span>
            </div>
          ))}
        </div>
        <div style={{
          display: 'flex', gap: 18,
          padding: '9px 18px',
          borderTop: '1px solid rgba(255,255,255,0.06)',
          fontFamily: '"JetBrains Mono", monospace',
          fontSize: 10, color: 'rgba(255,255,255,0.30)',
          letterSpacing: '0.1em',
        }}>
          <span>↑↓ navigate</span><span>↵ open</span><span>ESC close</span>
        </div>
      </div>
    </div>
  );
};

window.CommandPalette = CommandPalette;
