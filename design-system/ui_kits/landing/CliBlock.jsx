// CLI block specimen. Source: _source/lib/components/landing/CliBlock.svelte
// Pass an array of lines. Each line has { prompt?, cmd?, flags?, arg?, out? }.
const CliBlock = ({ lines = [] }) => (
  <div style={{
    background: 'rgba(255,255,255,0.02)',
    border: '1px solid rgba(255,255,255,0.06)',
    borderRadius: 4,
    padding: '1rem 1.2rem',
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
    fontSize: 12.5, lineHeight: 1.75,
    margin: '1.5rem 0',
    overflowX: 'auto',
  }}>
    {lines.map((l, i) => (
      <div key={i} style={{ color: 'rgba(255,255,255,0.65)' }}>
        {l.out
          ? <span style={{ color: 'rgba(255,255,255,0.45)' }}>{'  → ' + l.out}</span>
          : (<>
              <span style={{ color: 'rgba(255,255,255,0.30)', userSelect: 'none' }}>$ </span>
              {l.cmd && <span style={{ color: '#7eb8da' }}>{l.cmd}</span>}
              {l.arg && <span style={{ color: '#e8e4df' }}>{' ' + l.arg}</span>}
              {l.flags && <span style={{ color: 'rgba(255,255,255,0.30)' }}>{' ' + l.flags}</span>}
              {l.val && <span style={{ color: 'rgba(255,255,255,0.65)' }}>{' ' + l.val}</span>}
            </>)
        }
      </div>
    ))}
  </div>
);

window.CliBlock = CliBlock;
