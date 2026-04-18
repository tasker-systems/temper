// Vault grid. Source: _source/lib/components/VaultGrid.svelte + editorial hero
const VaultGrid = ({ context, onOpenResource }) => {
  const [hoverId, setHoverId] = React.useState(null);
  const resources = RESOURCES[context] ?? [];

  return (
    <div style={{ padding: '40px 44px 64px', maxWidth: 1100 }}>
      <div style={{
        fontFamily: '"JetBrains Mono", monospace',
        fontSize: 10, letterSpacing: '0.28em', textTransform: 'uppercase',
        color: '#7eb8da', marginBottom: 14,
      }}>VAULT · {context}</div>

      <h1 style={{
        fontFamily: 'Georgia, serif', fontSize: 'clamp(2.4rem, 5vw, 3.6rem)',
        fontWeight: 400, letterSpacing: '-0.025em', lineHeight: 0.98,
        color: '#e8e4df', margin: 0,
      }}>All resources</h1>

      <div style={{
        marginTop: 10,
        fontFamily: 'Georgia, serif', fontSize: 15, fontStyle: 'italic',
        color: 'rgba(255,255,255,0.65)', lineHeight: 1.55,
      }}>
        <span style={{ fontStyle: 'normal', color: '#e8e4df' }}>{resources.length}</span>
        <span style={{ color: 'rgba(255,255,255,0.30)', margin: '0 0.55em' }}>·</span>
        indexed
        <span style={{ color: 'rgba(255,255,255,0.30)', margin: '0 0.55em' }}>·</span>
        <span style={{ fontStyle: 'normal', color: '#7eb8da' }}>ready</span>
      </div>

      <FacetChips />

      {resources.length === 0 ? (
        <div style={{ marginTop: 48, padding: '44px', border: '1px dashed rgba(255,255,255,0.1)', textAlign: 'center' }}>
          <div style={{ fontFamily: 'Georgia, serif', fontStyle: 'italic', fontSize: 15, color: 'rgba(255,255,255,0.45)' }}>
            No resources yet.
          </div>
          <div style={{ marginTop: 10, fontFamily: '"JetBrains Mono", monospace', fontSize: 11, color: '#7eb8da', letterSpacing: '0.1em' }}>
            temper add &lt;path&gt;
          </div>
        </div>
      ) : (
        <div style={{
          marginTop: 36,
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fill, minmax(260px, 1fr))',
          gap: 14,
        }}>
          {resources.map(r => (
            <div key={r.id}
                 onClick={() => onOpenResource(r)}
                 onMouseEnter={() => setHoverId(r.id)}
                 onMouseLeave={() => setHoverId(null)}
                 style={{
                   padding: '16px 18px',
                   border: `1px solid ${hoverId === r.id ? 'rgba(126,184,218,0.40)' : 'rgba(255,255,255,0.06)'}`,
                   background: hoverId === r.id ? 'rgba(126,184,218,0.04)' : 'rgba(255,255,255,0.02)',
                   cursor: 'pointer',
                   transition: 'border-color 0.15s, background 0.15s',
                   display: 'flex', flexDirection: 'column', gap: 8,
                   minHeight: 122,
                 }}>
              <span style={{
                fontFamily: '"JetBrains Mono", monospace',
                fontSize: 9.5, letterSpacing: '0.2em',
                color: KIND_COLOR[r.kind] ?? '#7eb8da',
              }}>{r.kind} · {r.seq}</span>
              <div style={{
                fontFamily: 'Georgia, serif', fontSize: 16, lineHeight: 1.3,
                color: '#e8e4df', flex: 1,
              }}>{r.title}</div>
              <div style={{
                display: 'flex', justifyContent: 'space-between',
                fontFamily: '"JetBrains Mono", monospace',
                fontSize: 9.5, letterSpacing: '0.12em', textTransform: 'uppercase',
                color: 'rgba(255,255,255,0.45)',
              }}>
                <span>{r.stage}</span>
                <span>{r.ago}</span>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

// Facet chips. Source: _source/lib/components/FacetChips.svelte
const FacetChips = () => {
  const [sel, setSel] = React.useState({ stage: null, mode: null });
  const Row = ({ label, name, options }) => (
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, alignItems: 'center', marginBottom: 8 }}>
      <span style={{
        fontFamily: '"JetBrains Mono", monospace',
        fontSize: 9, letterSpacing: '0.22em', textTransform: 'uppercase',
        color: 'rgba(255,255,255,0.30)', minWidth: 60, marginRight: 6,
      }}>{label}</span>
      {options.map(o => {
        const on = sel[name] === o;
        return (
          <button key={o}
            onClick={() => setSel(s => ({ ...s, [name]: on ? null : o }))}
            style={{
              fontFamily: '"JetBrains Mono", monospace',
              fontSize: 10.5,
              padding: '3px 10px',
              border: `1px solid ${on ? 'rgba(126,184,218,0.50)' : 'rgba(255,255,255,0.1)'}`,
              borderRadius: 3,
              background: on ? 'rgba(126,184,218,0.08)' : 'transparent',
              color: on ? '#7eb8da' : 'rgba(255,255,255,0.65)',
              cursor: 'pointer',
              letterSpacing: '0.02em',
            }}>{o}</button>
        );
      })}
    </div>
  );
  return (
    <div style={{ marginTop: 28 }}>
      <Row label="STAGE" name="stage" options={['research', 'planning', 'building', 'decided', 'deferred']} />
      <Row label="KIND"  name="mode"  options={['research', 'session', 'task', 'decision', 'concept', 'goal']} />
    </div>
  );
};

Object.assign(window, { VaultGrid, FacetChips });
