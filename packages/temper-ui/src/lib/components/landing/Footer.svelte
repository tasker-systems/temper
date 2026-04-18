<footer class="footer">
  <a href="/" class="footer-logo">
    <svg class="footer-mark" viewBox="0 0 32 32" xmlns="http://www.w3.org/2000/svg">
      <path d="M 12 7 L 12 25" stroke="currentColor" stroke-width="3.5" stroke-linecap="round" fill="none"/>
      <path d="M 6 13 L 18 13 Q 23 13 25 16.5 Q 27 20 25 24" stroke="currentColor" stroke-width="2.8" stroke-linecap="round" fill="none"/>
    </svg>
    <span class="footer-wordmark">temper</span>
  </a>
  <div class="footer-links">
    <a href="/how-it-works">How it works</a>
    <a href="/builders">Builders</a>
    <a href="/agents">Agents</a>
    <a href="/docs">Docs</a>
  </div>
</footer>

<style>
  .footer { max-width: 800px; margin: 0 auto; padding: 4rem 2.5rem; border-top: 1px solid var(--rule); display: flex; justify-content: space-between; align-items: center; }
  .footer-logo { display: flex; align-items: center; gap: 0.45rem; text-decoration: none; color: var(--temper-blue-dim); transition: color 0.2s; }
  .footer-logo:hover { color: var(--temper-blue); }
  .footer-mark { width: 16px; height: 16px; }
  .footer-wordmark { font-family: var(--font-mono); font-size: 0.7rem; letter-spacing: 0.1em; }
  .footer-links { display: flex; gap: 1.5rem; }
  .footer-links a { font-family: var(--font-mono); font-size: 0.65rem; color: rgba(255, 255, 255, 0.25); text-decoration: none; letter-spacing: 0.05em; transition: color 0.2s; }
  .footer-links a:hover { color: var(--graphite); }
  @media (max-width: 640px) {
    .footer { flex-direction: column; gap: 1.5rem; text-align: center; }
    .footer-links { flex-wrap: wrap; justify-content: center; gap: 1rem; }
  }
</style>
