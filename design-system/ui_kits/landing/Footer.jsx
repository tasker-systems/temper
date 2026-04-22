// Footer. Source: _source/lib/components/landing/Footer.svelte
const Footer = () => (
  <footer style={{
    padding: '2rem 2.5rem',
    borderTop: '1px solid rgba(255,255,255,0.06)',
    display: 'flex', alignItems: 'center', gap: '1.5rem',
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
    fontSize: 11, color: 'rgba(255,255,255,0.45)',
    letterSpacing: '0.05em',
    maxWidth: '64rem', margin: '0 auto',
    flexWrap: 'wrap',
  }}>
    <Wordmark size="sm" />
    <span style={{ fontFamily: 'Georgia, serif', fontStyle: 'italic', fontSize: 12.5, color: 'rgba(255,255,255,0.45)' }}>
      — context that compounds
    </span>
    <div style={{ marginLeft: 'auto', display: 'flex', gap: 20 }}>
      {['Docs', 'Changelog', 'GitHub', 'Contact'].map(l => (
        <a key={l} style={{ color: 'rgba(255,255,255,0.45)', textDecoration: 'none', cursor: 'pointer' }}>{l}</a>
      ))}
    </div>
  </footer>
);

window.Footer = Footer;
