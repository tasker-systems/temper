// Resource view = editorial hero + chips + markdown
// Source: _source/lib/components/ResourceMetaHeader.svelte + MarkdownRenderer.svelte
const ResourceView = ({ resource, onBack }) => {
  if (!resource) return null;

  return (
    <div style={{ padding: '36px 44px 80px', maxWidth: 820 }}>
      <a onClick={onBack} style={{
        display: 'inline-block', marginBottom: 28,
        fontFamily: '"JetBrains Mono", monospace',
        fontSize: 10.5, letterSpacing: '0.18em', textTransform: 'uppercase',
        color: 'rgba(255,255,255,0.45)', cursor: 'pointer', textDecoration: 'none',
      }} onMouseEnter={e => e.currentTarget.style.color = '#e8e4df'}
         onMouseLeave={e => e.currentTarget.style.color = 'rgba(255,255,255,0.45)'}>
        ← All resources
      </a>

      <div style={{
        fontFamily: '"JetBrains Mono", monospace',
        fontSize: 10, letterSpacing: '0.22em', textTransform: 'uppercase',
        color: KIND_COLOR[resource.kind] ?? '#7eb8da', marginBottom: 14,
      }}>{resource.kind} · SEQ {resource.seq}</div>

      <h1 style={{
        fontFamily: 'Georgia, serif',
        fontSize: 'clamp(2rem, 4.5vw, 3rem)',
        fontWeight: 400, letterSpacing: '-0.02em', lineHeight: 1.05,
        color: '#e8e4df', margin: '0 0 16px 0',
      }}>{resource.title}</h1>

      <div style={{
        display: 'flex', gap: 18, alignItems: 'center', flexWrap: 'wrap',
        fontFamily: '"JetBrains Mono", monospace',
        fontSize: 10, letterSpacing: '0.15em', textTransform: 'uppercase',
        color: 'rgba(255,255,255,0.45)',
        paddingBottom: 22,
        borderBottom: '1px solid rgba(255,255,255,0.06)',
      }}>
        <span>STAGE · <span style={{ color: '#e8e4df' }}>{resource.stage}</span></span>
        <span style={{ color: 'rgba(255,255,255,0.15)' }}>·</span>
        <span>EFFORT · <span style={{ color: '#7eb8da' }}>{resource.effort}</span></span>
        <span style={{ color: 'rgba(255,255,255,0.15)' }}>·</span>
        <span>UPDATED · <span style={{ color: '#e8e4df' }}>{resource.ago}</span> ago</span>
      </div>

      <MarkdownDemo resource={resource} />
    </div>
  );
};

// Fake rendered-markdown body.
const MarkdownDemo = ({ resource }) => (
  <article style={{
    marginTop: 36,
    fontFamily: 'Georgia, serif',
    fontSize: 17, lineHeight: 1.8,
    color: 'rgba(255,255,255,0.8)',
  }}>
    <p style={{ margin: '0 0 1.4em 0' }}>
      <em style={{ color: 'rgba(255,255,255,0.55)' }}>The choice:</em>{' '}
      <strong style={{ color: '#e8e4df', fontWeight: 400 }}>Stripe</strong>, evaluated against Paddle and Lemon Squeezy.
      The deciding factor: PCI scope. We never want card numbers near our servers, and
      Stripe's hosted elements plus their attested SAQ-A handling means our compliance
      surface stays tiny.
    </p>

    <h2 style={{
      fontFamily: 'Georgia, serif', fontWeight: 300, fontSize: 22,
      color: '#e8e4df', margin: '2em 0 0.6em 0',
    }}>Alternatives <em style={{ color: '#7eb8da' }}>considered</em></h2>

    <p style={{ margin: '0 0 1.3em 0' }}>
      <strong style={{ color: '#e8e4df', fontWeight: 400 }}>Paddle</strong>{' '}
      acts as merchant of record — tax is handled for you. Attractive for EU VAT work,
      but the takerate and payout schedule didn't fit our cash-flow shape.
    </p>

    <p style={{ margin: '0 0 1.3em 0' }}>
      <strong style={{ color: '#e8e4df', fontWeight: 400 }}>Lemon Squeezy</strong>{' '}
      shares Paddle's MoR model and ships with a cleaner dashboard, but the webhooks
      surface area is smaller than we need once invoices get complicated.
    </p>

    <pre style={{
      background: 'rgba(255,255,255,0.03)',
      border: '1px solid rgba(255,255,255,0.06)',
      borderRadius: 4, padding: '14px 18px',
      fontFamily: '"JetBrains Mono", monospace',
      fontSize: 12.5, lineHeight: 1.7,
      color: 'rgba(255,255,255,0.75)',
      overflowX: 'auto', margin: '1.6em 0',
    }}>
{'---\nkind: decision\nseq: 003\nstage: decided\neffort: medium\ncreated: 2026-03-19\nconcepts: [invoice, pci-scope, payments]\n---'}
    </pre>

    <blockquote style={{
      borderLeft: '2px solid rgba(126,184,218,0.25)',
      paddingLeft: '1.8rem', margin: '2em 0',
      fontStyle: 'italic', color: 'rgba(255,255,255,0.55)',
      fontSize: 16.5,
    }}>
      "The frontmatter carries the throughline. The content carries the thinking.
      Git carries the history."
    </blockquote>

    <h2 style={{
      fontFamily: 'Georgia, serif', fontWeight: 300, fontSize: 22,
      color: '#e8e4df', margin: '2em 0 0.6em 0',
    }}>Deferred</h2>

    <ul style={{ margin: '0 0 1.4em 0', paddingLeft: '1.3em' }}>
      <li style={{ marginBottom: 8 }}>Tax jurisdiction handling — Stripe Tax vs. TaxJar, pending volume data.</li>
      <li style={{ marginBottom: 8 }}>Webhook retry policy — idempotency keys in place; retry budget TBD.</li>
    </ul>
  </article>
);

Object.assign(window, { ResourceView, MarkdownDemo });
