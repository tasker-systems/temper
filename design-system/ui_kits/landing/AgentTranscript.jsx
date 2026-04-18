// Agent transcript block. Source: _source/lib/components/landing/AgentTranscript.svelte
const AgentTranscript = () => {
  const Line = ({ who, userSide, children }) => (
    <div style={{ display: 'flex', gap: 14, marginBottom: '0.9rem' }}>
      <span style={{
        minWidth: 60,
        fontFamily: '"JetBrains Mono", ui-monospace, monospace',
        fontSize: 10, letterSpacing: '0.14em', textTransform: 'uppercase',
        color: userSide ? 'rgba(255,255,255,0.45)' : '#7eb8da',
        paddingTop: 4,
      }}>{who}</span>
      <span style={{
        flex: 1,
        fontFamily: 'Georgia, serif', fontSize: 14, lineHeight: 1.7,
        color: 'rgba(255,255,255,0.65)',
      }}>{children}</span>
    </div>
  );
  const Code = ({ children }) => (
    <code style={{
      fontFamily: '"JetBrains Mono", ui-monospace, monospace',
      fontSize: 11.5, color: '#7eb8da',
      background: 'rgba(255,255,255,0.03)',
      padding: '1px 5px', borderRadius: 2,
    }}>{children}</code>
  );
  return (
    <div style={{
      background: 'rgba(255,255,255,0.02)',
      border: '1px solid rgba(255,255,255,0.06)',
      padding: '1.4rem 1.6rem',
      margin: '1.5rem 0',
    }}>
      <Line who="you" userSide>What did we decide about the invoice provider?</Line>
      <Line who="agent">
        Checking the throughline… <Code>temper recall invoice</Code> — you chose <em style={{ color: '#e8e4df', fontStyle: 'italic' }}>Stripe</em> after comparing three options; the decision doc cites PCI scope as the deciding factor. The alternatives (Paddle, Lemon Squeezy) are preserved under <Code>research/seq‑003</Code>.
      </Line>
      <Line who="you" userSide>What's deferred from that session?</Line>
      <Line who="agent">
        Two items: the <em style={{ color: '#e8e4df', fontStyle: 'italic' }}>tax‑jurisdiction</em> question, and the <em style={{ color: '#e8e4df', fontStyle: 'italic' }}>webhook retry policy</em>. Both are tagged <Code>deferred</Code> with links back to this decision.
      </Line>
    </div>
  );
};

window.AgentTranscript = AgentTranscript;
