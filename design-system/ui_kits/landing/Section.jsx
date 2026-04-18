// Section with labeled eyebrow + left-rail h2 + body.
// Source: _source/lib/components/landing/Section.svelte
const Section = ({ label, children }) => (
  <section style={{ padding: '4.5rem 0', borderTop: '1px solid rgba(255,255,255,0.06)' }}>
    <div style={{ maxWidth: '50rem', margin: '0 auto', padding: '0 2rem' }}>
      <div style={{
        fontFamily: '"JetBrains Mono", ui-monospace, monospace',
        fontSize: 10.5, letterSpacing: '0.22em', textTransform: 'uppercase',
        color: '#7eb8da', marginBottom: 20,
      }}>{label}</div>
      <div style={{ borderLeft: '2px solid rgba(126,184,218,0.25)', paddingLeft: '2rem' }}>
        {children}
      </div>
    </div>
  </section>
);

// H2 with one italic word in blue
const H2 = ({ children }) => (
  <h2 style={{
    fontFamily: 'Georgia, serif', fontWeight: 300, fontSize: '1.7rem',
    lineHeight: 1.3, color: '#e8e4df', margin: '0 0 1.2rem 0',
  }}>
    {children}
  </h2>
);

const P = ({ children, dim }) => (
  <p style={{
    fontFamily: 'Georgia, serif', fontSize: '1rem', lineHeight: 1.8,
    color: dim ? 'rgba(255,255,255,0.45)' : 'rgba(255,255,255,0.65)',
    margin: '0 0 1.2rem 0',
  }}>
    {children}
  </p>
);

const Em = ({ children }) => (
  <em style={{ color: '#7eb8da', fontStyle: 'italic' }}>{children}</em>
);

const Strong = ({ children }) => (
  <strong style={{ color: '#e8e4df', fontWeight: 400 }}>{children}</strong>
);

Object.assign(window, { Section, H2, P, Em, Strong });
