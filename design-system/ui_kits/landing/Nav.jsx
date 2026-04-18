// Top nav. Source: _source/lib/components/landing/Nav.svelte
const Nav = ({ current = 'home' }) => {
  const [scrolled, setScrolled] = React.useState(false);
  React.useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 40);
    window.addEventListener('scroll', onScroll);
    return () => window.removeEventListener('scroll', onScroll);
  }, []);
  const navStyle = {
    position: 'sticky', top: 0, zIndex: 50,
    display: 'flex', alignItems: 'center',
    padding: '20px 32px',
    borderBottom: scrolled ? '1px solid rgba(255,255,255,0.06)' : '1px solid transparent',
    background: scrolled ? 'rgba(10,10,15,0.95)' : 'transparent',
    backdropFilter: scrolled ? 'blur(12px)' : 'none',
    transition: 'background 0.2s, border-color 0.2s',
  };
  const linkStyle = (active) => ({
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
    fontSize: 11.5, letterSpacing: '0.05em',
    color: active ? 'rgba(255,255,255,0.85)' : 'rgba(255,255,255,0.45)',
    textDecoration: 'none',
    cursor: 'pointer',
  });
  const ctaStyle = {
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
    fontSize: 10.5, letterSpacing: '0.1em', textTransform: 'uppercase',
    color: '#7eb8da',
    padding: '7px 14px',
    border: '1px solid rgba(126,184,218,0.50)',
    background: 'transparent', cursor: 'pointer', textDecoration: 'none',
    transition: 'background 0.15s, color 0.15s',
  };
  return (
    <nav style={navStyle}>
      <Wordmark size="md" />
      <div style={{ marginLeft: 'auto', display: 'flex', gap: 24, alignItems: 'center' }}>
        <a style={linkStyle(current === 'builders')}>For builders</a>
        <a style={linkStyle(current === 'agents')}>For agents</a>
        <a style={linkStyle(current === 'docs')}>Docs</a>
        <a style={{ ...ctaStyle, marginLeft: 12 }}
           onMouseEnter={e => { e.currentTarget.style.background = 'rgba(126,184,218,0.12)'; e.currentTarget.style.color = '#e8e4df'; }}
           onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = '#7eb8da'; }}>
          Sign in
        </a>
      </div>
    </nav>
  );
};

window.Nav = Nav;
