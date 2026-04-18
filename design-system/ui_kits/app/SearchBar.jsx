// Search bar across top of main. Opens command palette on click or ⌘K.
// Source: _source/routes/(app)/+layout.svelte
const SearchBar = ({ onOpen }) => (
  <header style={{
    position: 'sticky', top: 0, zIndex: 5,
    display: 'flex', alignItems: 'center', gap: 12,
    padding: '14px 28px',
    background: '#0a0a0f',
    borderBottom: '1px solid rgba(255,255,255,0.06)',
  }}>
    <button
      onClick={onOpen}
      style={{
        flex: 1, display: 'flex', justifyContent: 'space-between', alignItems: 'center',
        padding: '7px 12px',
        background: '#12121a',
        border: '1px solid rgba(255,255,255,0.1)',
        borderRadius: 4,
        fontFamily: '"JetBrains Mono", monospace',
        fontSize: 12,
        color: 'rgba(255,255,255,0.45)',
        cursor: 'pointer',
      }}
      onMouseEnter={e => { e.currentTarget.style.borderColor = 'rgba(255,255,255,0.2)'; }}
      onMouseLeave={e => { e.currentTarget.style.borderColor = 'rgba(255,255,255,0.1)'; }}
    >
      <span>Search the vault…</span>
      <kbd style={{
        fontSize: 10, background: 'rgba(255,255,255,0.06)',
        border: '1px solid rgba(255,255,255,0.1)', borderRadius: 2,
        padding: '1px 6px', color: 'rgba(255,255,255,0.45)',
        fontFamily: '"JetBrains Mono", monospace',
      }}>⌘K</kbd>
    </button>
  </header>
);

window.SearchBar = SearchBar;
