// Hero. Source: _source/lib/components/landing/Hero.svelte
const Hero = () => (
  <section style={{
    padding: '7rem 0 5rem',
    textAlign: 'left',
  }}>
    <div style={{ maxWidth: '50rem', margin: '0 auto', padding: '0 2rem' }}>
      <div style={{
        fontFamily: '"JetBrains Mono", ui-monospace, monospace',
        fontSize: 10.5, letterSpacing: '0.28em', textTransform: 'uppercase',
        color: '#7eb8da', marginBottom: 28,
      }}>A KNOWLEDGE BASE FOR BUILDERS</div>

      <h1 style={{
        fontFamily: 'Georgia, serif', fontWeight: 300,
        fontSize: 'clamp(2.4rem, 5vw, 3.6rem)',
        lineHeight: 1.15, letterSpacing: '0.01em',
        color: '#e8e4df', margin: '0 0 1.5rem 0',
      }}>
        Clarify your <em style={{ color: '#7eb8da', fontStyle: 'italic' }}>intention</em>.<br/>
        Remember what you <em style={{ color: '#7eb8da', fontStyle: 'italic' }}>decided</em>.<br/>
        <span style={{ color: 'rgba(255,255,255,0.55)' }}>Every session builds on the last.</span>
      </h1>

      <p style={{
        fontFamily: 'Georgia, serif', fontStyle: 'italic',
        fontSize: '1.15rem', lineHeight: 1.65,
        color: 'rgba(255,255,255,0.45)',
        margin: '0 0 2rem 0', maxWidth: 540,
      }}>
        Temper gives your work a throughline — the connective thread
        across sessions, decisions, and evolving understanding that
        turns scattered context into a navigable history.
      </p>

      <CliBlock lines={[
        { cmd: 'temper', arg: 'warmup', flags: '--context myapp' },
        { out: 'loaded 847 resources · 12 active sessions · ready' },
        { cmd: 'temper', arg: 'session start', flags: '--goal "finish invoice flow"' },
      ]} />

      <div style={{ display: 'flex', gap: 16, marginTop: '2rem', alignItems: 'center' }}>
        <a href="#premise" style={{
          fontFamily: '"JetBrains Mono", ui-monospace, monospace',
          fontSize: 11, letterSpacing: '0.1em', textTransform: 'uppercase',
          color: '#7eb8da', padding: '9px 16px',
          border: '1px solid rgba(126,184,218,0.50)',
          textDecoration: 'none',
          transition: 'background 0.15s, color 0.15s',
        }} onMouseEnter={e => { e.currentTarget.style.background='rgba(126,184,218,0.12)'; e.currentTarget.style.color='#e8e4df'; }}
           onMouseLeave={e => { e.currentTarget.style.background='transparent'; e.currentTarget.style.color='#7eb8da'; }}>
          See how it works
        </a>
        <a style={{
          fontFamily: '"JetBrains Mono", ui-monospace, monospace',
          fontSize: 10, letterSpacing: '0.18em', textTransform: 'uppercase',
          color: 'rgba(255,255,255,0.45)', textDecoration: 'none', cursor: 'pointer',
        }}>Read the docs →</a>
      </div>
    </div>
  </section>
);

window.Hero = Hero;
