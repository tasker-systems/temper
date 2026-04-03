<svelte:head>
  <title>Documentation — temper</title>
</svelte:head>

<div class="docs-placeholder">
  <h1>Documentation</h1>
  <p>
    Temper is under active development. Documentation is coming soon.
  </p>
  <p>
    In the meantime, the
    <a href="https://github.com/tasker-systems/temper">GitHub repository README</a>
    is the best reference for getting started.
  </p>
  <a href="/" class="back-link">&larr; Back to home</a>
</div>

<style>
  .docs-placeholder {
    max-width: 600px;
    margin: 0 auto;
    padding: 12rem 2.5rem 6rem;
  }

  h1 {
    font-family: var(--serif);
    font-size: 2rem;
    font-weight: 300;
    color: var(--text);
    margin-bottom: 1.5rem;
  }

  p {
    font-family: var(--serif);
    font-size: 1rem;
    color: var(--text-mid);
    line-height: 1.8;
    margin-bottom: 1rem;
  }

  a {
    color: var(--blue);
    text-decoration: none;
    transition: color 0.2s;
  }

  a:hover {
    color: var(--text);
  }

  .back-link {
    display: inline-block;
    margin-top: 2rem;
    font-family: var(--mono);
    font-size: 0.75rem;
    color: var(--text-dim);
    letter-spacing: 0.05em;
  }

  .back-link:hover {
    color: var(--blue);
  }
</style>
