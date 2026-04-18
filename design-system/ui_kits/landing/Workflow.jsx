// Four-step workflow strip. Source: _source/routes/(public)/+page.svelte
const Workflow = () => {
  const steps = [
    { cmd: 'temper init',   desc: 'Create a vault and tell temper how you work — your tools, your conventions, your rhythm.' },
    { cmd: 'temper add',    desc: 'Bring in your docs. Temper extracts markdown, adds frontmatter, and makes everything searchable.' },
    { cmd: 'temper search', desc: 'Semantic search across your vault. Find decisions by meaning, not just keywords.' },
    { cmd: 'temper sync',   desc: 'Push to the cloud. Pull to another machine. Your vault follows you.' },
  ];
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '1.2rem', marginTop: '1.5rem' }}>
      {steps.map(s => (
        <div key={s.cmd} style={{ display: 'flex', alignItems: 'flex-start', gap: '1.2rem' }}>
          <span style={{
            fontFamily: '"JetBrains Mono", ui-monospace, monospace',
            fontSize: 11,
            padding: '6px 12px',
            border: '1px solid rgba(255,255,255,0.1)',
            color: '#7eb8da',
            whiteSpace: 'nowrap',
            minWidth: 140,
            letterSpacing: '0.02em',
          }}>{s.cmd}</span>
          <span style={{
            fontFamily: 'Georgia, serif', fontSize: 14.5,
            color: 'rgba(255,255,255,0.65)', lineHeight: 1.7, paddingTop: 5,
          }}>{s.desc}</span>
        </div>
      ))}
    </div>
  );
};

// Six concept cards
const Concepts = () => {
  const [hover, setHover] = React.useState(null);
  const items = [
    ['Goals',     "The outcome you're working toward. Tasks and sessions roll up to goals."],
    ['Tasks',     'Discrete units of work with mode (plan/build) and effort (small/medium/large).'],
    ['Sessions',  'What happened in a working session — decisions made, context discovered, next steps.'],
    ['Research',  'Investigation and analysis. Design explorations, comparisons, architectural options.'],
    ['Decisions', 'The choice, the alternatives, the constraints. Captured so you never re-litigate.'],
    ['Concepts',  'Domain knowledge. The vocabulary of your project that humans and agents share.'],
  ];
  return (
    <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(200px, 1fr))', gap: '1rem', marginTop: '1.5rem' }}>
      {items.map(([name, body], i) => (
        <div key={name}
          onMouseEnter={() => setHover(i)} onMouseLeave={() => setHover(null)}
          style={{
            border: `1px solid ${hover === i ? 'rgba(126,184,218,0.25)' : 'rgba(255,255,255,0.06)'}`,
            padding: '1.2rem', transition: 'border-color 0.2s',
          }}>
          <div style={{
            fontFamily: '"JetBrains Mono", ui-monospace, monospace',
            fontSize: 10.5, color: '#7eb8da', letterSpacing: '0.18em',
            textTransform: 'uppercase', marginBottom: '0.6rem',
          }}>{name}</div>
          <p style={{
            fontFamily: 'Georgia, serif', fontSize: 13.5,
            color: 'rgba(255,255,255,0.55)', lineHeight: 1.65, margin: 0,
          }}>{body}</p>
        </div>
      ))}
    </div>
  );
};

Object.assign(window, { Workflow, Concepts });
