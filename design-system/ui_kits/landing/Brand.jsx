// Brand mark + wordmark. Source: _source/brand-mark.svg
const BrandGlyph = ({ size = 16 }) => (
  <svg viewBox="0 0 32 32" width={size} height={size} fill="none" style={{ display: 'inline-block', verticalAlign: 'middle' }}>
    <path d="M 12 7 L 12 25" stroke="currentColor" strokeWidth="3.5" strokeLinecap="round" />
    <path d="M 6 13 L 18 13 Q 23 13 25 16.5 Q 27 20 25 24" stroke="currentColor" strokeWidth="2.8" strokeLinecap="round" />
  </svg>
);

const Wordmark = ({ size = 'md' }) => {
  const sizes = { sm: 11, md: 14, lg: 18 };
  const glyphSizes = { sm: 13, md: 16, lg: 22 };
  const fs = sizes[size] ?? 14;
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 8, color: '#7eb8da' }}>
      <BrandGlyph size={glyphSizes[size] ?? 16} />
      <span style={{ fontFamily: '"JetBrains Mono", ui-monospace, monospace', fontWeight: 500, fontSize: fs, letterSpacing: '0.15em' }}>temper</span>
    </span>
  );
};

Object.assign(window, { BrandGlyph, Wordmark });
